//! Global hotkeys: the soundboard's combinations, registered with the system.
//!
//! [`Hotkeys`] keeps the registered set equal to the one wanted, and turns presses into what they
//! are for. The system behind it is a [`HotkeyBackend`]: [`SystemHotkeys`] in the app, a fake in
//! tests. Aria receives only the combinations it registered; it reads no other keystrokes.
//!
//! A press wakes the window itself: `global-hotkey` hands each press to Aria's handler, which
//! queues it and asks for a repaint. eframe then runs [`eframe::App::logic`] even while the window
//! is minimized, and the app reads the presses there. Nothing polls.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

use aria_core::{
    hotkey::{Hotkey, HotkeyKey},
    runtime::Waker,
};
use aria_library::SoundId;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};

/// The system's side of the hotkeys.
pub(crate) trait HotkeyBackend {
    /// Asks the system for `hotkey`. An error says why it refused, in words for the log.
    fn register(&mut self, hotkey: Hotkey) -> Result<(), String>;
    /// Gives `hotkey` back to the system.
    fn unregister(&mut self, hotkey: Hotkey);
    /// The registered hotkeys pressed since the last call, oldest first. Key-down only.
    fn pressed(&mut self) -> Vec<Hotkey>;
}

impl<B: HotkeyBackend + ?Sized> HotkeyBackend for Box<B> {
    fn register(&mut self, hotkey: Hotkey) -> Result<(), String> {
        (**self).register(hotkey)
    }

    fn unregister(&mut self, hotkey: Hotkey) {
        (**self).unregister(hotkey);
    }

    fn pressed(&mut self) -> Vec<Hotkey> {
        (**self).pressed()
    }
}

/// What a hotkey is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyAction {
    /// Play this sound.
    Play(SoundId),
    /// Stop every sound.
    StopAll,
    /// Turn the microphone on or off.
    ToggleMicrophone,
}

/// The hotkeys wanted, and those the system gave.
pub(crate) struct Hotkeys<B: HotkeyBackend> {
    backend: B,
    /// What each wanted hotkey is for, also while the switch is off.
    wanted: HashMap<Hotkey, HotkeyAction>,
    /// The hotkeys the system holds for Aria now.
    registered: HashSet<Hotkey>,
}

impl<B: HotkeyBackend> Hotkeys<B> {
    /// Nothing wanted and nothing registered yet.
    pub(crate) fn new(backend: B) -> Self {
        Self {
            backend,
            wanted: HashMap::new(),
            registered: HashSet::new(),
        }
    }

    /// Make the registered set match `wanted`; returns the hotkeys the system refused.
    ///
    /// With `enabled` off, everything is unregistered and `wanted` is still kept, so turning it
    /// back on restores them. A refused hotkey is asked for again on the next call. Should a
    /// hotkey be wanted twice, its first action wins.
    pub(crate) fn sync(&mut self, enabled: bool, wanted: &[(Hotkey, HotkeyAction)]) -> Vec<Hotkey> {
        let mut map = HashMap::with_capacity(wanted.len());
        for (hotkey, action) in wanted {
            map.entry(*hotkey).or_insert(*action);
        }
        self.wanted = map;

        let stale: Vec<Hotkey> = self
            .registered
            .iter()
            .filter(|hotkey| !enabled || !self.wanted.contains_key(hotkey))
            .copied()
            .collect();
        for hotkey in stale {
            self.backend.unregister(hotkey);
            self.registered.remove(&hotkey);
        }
        if !enabled {
            return Vec::new();
        }

        let mut refused = Vec::new();
        let mut seen = HashSet::new();
        for (hotkey, _) in wanted {
            if !seen.insert(*hotkey) || self.registered.contains(hotkey) {
                continue;
            }
            match self.backend.register(*hotkey) {
                Ok(()) => {
                    self.registered.insert(*hotkey);
                }
                Err(err) => {
                    tracing::debug!(%hotkey, %err, "the system refused a hotkey");
                    refused.push(*hotkey);
                }
            }
        }
        refused
    }

    /// What the presses since the last call are for. A press of a hotkey no longer registered
    /// or wanted is dropped.
    pub(crate) fn poll(&mut self) -> Vec<HotkeyAction> {
        self.backend
            .pressed()
            .into_iter()
            .filter(|hotkey| self.registered.contains(hotkey))
            .filter_map(|hotkey| self.wanted.get(&hotkey).copied())
            .collect()
    }

    /// Whether the system accepts `hotkey` right now; leaves nothing registered that was not before.
    pub(crate) fn try_register(&mut self, hotkey: Hotkey) -> bool {
        if self.registered.contains(&hotkey) {
            return true;
        }
        match self.backend.register(hotkey) {
            Ok(()) => {
                self.backend.unregister(hotkey);
                true
            }
            Err(err) => {
                tracing::debug!(%hotkey, %err, "the system refused a hotkey");
                false
            }
        }
    }

    /// Whether the system holds any hotkey for Aria now.
    #[cfg(test)]
    pub(crate) fn any_registered(&self) -> bool {
        !self.registered.is_empty()
    }
}

/// Whether the process's one press handler has been installed. `global-hotkey` keeps a single
/// handler for the whole process, so only one [`SystemHotkeys`] can ever receive presses.
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

/// The system's hotkeys, through `global-hotkey`. The manager must be made on the main thread,
/// before the window opens, and lives as long as the window: dropping it unregisters everything.
pub struct SystemHotkeys {
    manager: GlobalHotKeyManager,
    /// The system's id of each registered hotkey.
    ids: HashMap<u32, Hotkey>,
    /// The ids of the hotkeys pressed, as the handler queued them.
    presses: Receiver<u32>,
}

impl SystemHotkeys {
    /// Makes the manager and installs the press handler, which queues each key-down and calls
    /// `wake`. Call it once, on the main thread, before the event loop starts: the handler must
    /// be in place before any hotkey is registered, or `global-hotkey` fixes it at none.
    pub fn new(wake: Waker) -> Result<Self, String> {
        if HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
            return Err("the hotkey handler is already installed".to_owned());
        }
        let manager = GlobalHotKeyManager::new().map_err(|err| err.to_string())?;
        let (sender, presses) = mpsc::channel();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            // A release comes from a thread of the crate's own; only presses matter.
            if event.state != HotKeyState::Pressed {
                return;
            }
            if sender.send(event.id).is_ok() {
                wake();
            }
        }));
        Ok(Self {
            manager,
            ids: HashMap::new(),
            presses,
        })
    }
}

impl HotkeyBackend for SystemHotkeys {
    fn register(&mut self, hotkey: Hotkey) -> Result<(), String> {
        let system =
            to_system(hotkey).ok_or_else(|| "a key the system does not know".to_owned())?;
        self.manager
            .register(system)
            .map_err(|err| err.to_string())?;
        self.ids.insert(system.id(), hotkey);
        Ok(())
    }

    fn unregister(&mut self, hotkey: Hotkey) {
        let Some(system) = to_system(hotkey) else {
            return;
        };
        self.ids.remove(&system.id());
        if let Err(err) = self.manager.unregister(system) {
            tracing::debug!(%hotkey, %err, "could not unregister a hotkey");
        }
    }

    fn pressed(&mut self) -> Vec<Hotkey> {
        self.presses
            .try_iter()
            .filter_map(|id| self.ids.get(&id).copied())
            .collect()
    }
}

/// A system that gives no hotkeys: where the manager could not be made. Every hotkey is refused,
/// so the soundboard shows them as inactive and none is saved.
pub(crate) struct NoHotkeys;

impl HotkeyBackend for NoHotkeys {
    fn register(&mut self, _hotkey: Hotkey) -> Result<(), String> {
        Err("global hotkeys are unavailable".to_owned())
    }

    fn unregister(&mut self, _hotkey: Hotkey) {}

    fn pressed(&mut self) -> Vec<Hotkey> {
        Vec::new()
    }
}

/// The letters' keys, `A` to `Z`.
const LETTERS: [Code; 26] = [
    Code::KeyA,
    Code::KeyB,
    Code::KeyC,
    Code::KeyD,
    Code::KeyE,
    Code::KeyF,
    Code::KeyG,
    Code::KeyH,
    Code::KeyI,
    Code::KeyJ,
    Code::KeyK,
    Code::KeyL,
    Code::KeyM,
    Code::KeyN,
    Code::KeyO,
    Code::KeyP,
    Code::KeyQ,
    Code::KeyR,
    Code::KeyS,
    Code::KeyT,
    Code::KeyU,
    Code::KeyV,
    Code::KeyW,
    Code::KeyX,
    Code::KeyY,
    Code::KeyZ,
];

/// The main keyboard's digits, 0 to 9.
const DIGITS: [Code; 10] = [
    Code::Digit0,
    Code::Digit1,
    Code::Digit2,
    Code::Digit3,
    Code::Digit4,
    Code::Digit5,
    Code::Digit6,
    Code::Digit7,
    Code::Digit8,
    Code::Digit9,
];

/// The numpad's digits, 0 to 9.
const NUMPAD: [Code; 10] = [
    Code::Numpad0,
    Code::Numpad1,
    Code::Numpad2,
    Code::Numpad3,
    Code::Numpad4,
    Code::Numpad5,
    Code::Numpad6,
    Code::Numpad7,
    Code::Numpad8,
    Code::Numpad9,
];

/// The function keys, F1 to F24.
const F_KEYS: [Code; 24] = [
    Code::F1,
    Code::F2,
    Code::F3,
    Code::F4,
    Code::F5,
    Code::F6,
    Code::F7,
    Code::F8,
    Code::F9,
    Code::F10,
    Code::F11,
    Code::F12,
    Code::F13,
    Code::F14,
    Code::F15,
    Code::F16,
    Code::F17,
    Code::F18,
    Code::F19,
    Code::F20,
    Code::F21,
    Code::F22,
    Code::F23,
    Code::F24,
];

/// The system's form of `hotkey`, or `None` for a key no hotkey can use, such as a letter
/// outside `A` to `Z`.
pub(crate) fn to_system(hotkey: Hotkey) -> Option<HotKey> {
    let code = match hotkey.key {
        HotkeyKey::Letter(c) => {
            let index = u32::from(c).checked_sub(u32::from('A'))?;
            *LETTERS.get(usize::try_from(index).ok()?)?
        }
        HotkeyKey::Digit(d) => *DIGITS.get(usize::from(d))?,
        HotkeyKey::Numpad(d) => *NUMPAD.get(usize::from(d))?,
        HotkeyKey::F(n) => *F_KEYS.get(usize::from(n).checked_sub(1)?)?,
        HotkeyKey::NumpadAdd => Code::NumpadAdd,
        HotkeyKey::NumpadSubtract => Code::NumpadSubtract,
        HotkeyKey::NumpadMultiply => Code::NumpadMultiply,
        HotkeyKey::NumpadDivide => Code::NumpadDivide,
        HotkeyKey::NumpadDecimal => Code::NumpadDecimal,
    };
    let mut mods = Modifiers::empty();
    for (held, modifier) in [
        (hotkey.ctrl, Modifiers::CONTROL),
        (hotkey.alt, Modifiers::ALT),
        (hotkey.shift, Modifiers::SHIFT),
        (hotkey.win, Modifiers::META),
    ] {
        if held {
            mods |= modifier;
        }
    }
    Some(HotKey::new(Some(mods), code))
}

#[cfg(test)]
pub(crate) mod fake {
    //! A system that refuses what it is told to, and presses what it is told to.

    use std::{
        cell::RefCell,
        collections::{HashSet, VecDeque},
        rc::Rc,
    };

    use aria_core::hotkey::Hotkey;

    use super::HotkeyBackend;

    /// What the fake system holds, shared with the test.
    #[derive(Default)]
    pub(crate) struct FakeSystem {
        /// The hotkeys another program has.
        pub(crate) taken: HashSet<Hotkey>,
        /// The hotkeys registered now.
        pub(crate) registered: HashSet<Hotkey>,
        /// Presses waiting to be read.
        pub(crate) presses: VecDeque<Hotkey>,
        /// Every hotkey asked for, in order.
        pub(crate) asked: Vec<Hotkey>,
    }

    /// The backend the code under test owns; the test keeps the other handle.
    #[derive(Clone, Default)]
    pub(crate) struct Fake(pub(crate) Rc<RefCell<FakeSystem>>);

    impl Fake {
        /// Another program takes `hotkey`.
        pub(crate) fn take(&self, hotkey: Hotkey) {
            self.0.borrow_mut().taken.insert(hotkey);
        }

        /// The user presses `hotkey`. The fake delivers it even when it is not registered, as a
        /// press already queued when it was unregistered would be.
        pub(crate) fn press(&self, hotkey: Hotkey) {
            self.0.borrow_mut().presses.push_back(hotkey);
        }

        /// The hotkeys registered now.
        pub(crate) fn registered(&self) -> HashSet<Hotkey> {
            self.0.borrow().registered.clone()
        }
    }

    impl HotkeyBackend for Fake {
        fn register(&mut self, hotkey: Hotkey) -> Result<(), String> {
            let mut system = self.0.borrow_mut();
            system.asked.push(hotkey);
            if system.taken.contains(&hotkey) {
                return Err("taken by another program".to_owned());
            }
            if !system.registered.insert(hotkey) {
                return Err("already registered".to_owned());
            }
            Ok(())
        }

        fn unregister(&mut self, hotkey: Hotkey) {
            self.0.borrow_mut().registered.remove(&hotkey);
        }

        fn pressed(&mut self) -> Vec<Hotkey> {
            self.0.borrow_mut().presses.drain(..).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use aria_core::hotkey::F_KEY_MAX;

    use super::{fake::Fake, *};

    fn key(text: &str) -> Hotkey {
        text.parse()
            .unwrap_or_else(|err| panic!("{text} is a hotkey: {err}"))
    }

    fn hotkeys() -> (Hotkeys<Fake>, Fake) {
        let fake = Fake::default();
        (Hotkeys::new(fake.clone()), fake)
    }

    fn set(texts: &[&str]) -> HashSet<Hotkey> {
        texts.iter().map(|text| key(text)).collect()
    }

    #[test]
    fn sync_registers_new_and_unregisters_removed() {
        let (mut hotkeys, fake) = hotkeys();
        let first = [
            (key("Ctrl+1"), HotkeyAction::Play(SoundId(1))),
            (key("F9"), HotkeyAction::StopAll),
        ];
        assert!(hotkeys.sync(true, &first).is_empty());
        assert_eq!(fake.registered(), set(&["Ctrl+1", "F9"]));
        assert!(hotkeys.any_registered());

        let second = [
            (key("F9"), HotkeyAction::StopAll),
            (key("Alt+F2"), HotkeyAction::Play(SoundId(2))),
        ];
        assert!(hotkeys.sync(true, &second).is_empty());
        assert_eq!(fake.registered(), set(&["F9", "Alt+F2"]));
        // F9 stayed registered: it was not asked for a second time.
        let asked = fake.0.borrow().asked.clone();
        assert_eq!(asked.iter().filter(|h| **h == key("F9")).count(), 1);

        assert!(hotkeys.sync(true, &[]).is_empty());
        assert!(fake.registered().is_empty());
        assert!(!hotkeys.any_registered());
    }

    #[test]
    fn turning_the_switch_off_unregisters_everything_and_keeps_the_list() {
        let (mut hotkeys, fake) = hotkeys();
        let wanted = [(key("Ctrl+1"), HotkeyAction::Play(SoundId(1)))];
        hotkeys.sync(true, &wanted);
        assert!(hotkeys.sync(false, &wanted).is_empty());
        assert!(fake.registered().is_empty());
        assert!(!hotkeys.any_registered());
        assert_eq!(
            hotkeys.wanted.get(&key("Ctrl+1")),
            Some(&HotkeyAction::Play(SoundId(1))),
            "the list is kept"
        );
        // A press while off fires nothing.
        fake.press(key("Ctrl+1"));
        assert!(hotkeys.poll().is_empty());

        hotkeys.sync(true, &wanted);
        assert_eq!(fake.registered(), set(&["Ctrl+1"]));
    }

    #[test]
    fn a_refused_hotkey_is_reported_and_retried_on_the_next_sync() {
        let (mut hotkeys, fake) = hotkeys();
        fake.take(key("Ctrl+1"));
        let wanted = [
            (key("Ctrl+1"), HotkeyAction::Play(SoundId(1))),
            (key("Ctrl+2"), HotkeyAction::Play(SoundId(2))),
        ];
        assert_eq!(hotkeys.sync(true, &wanted), [key("Ctrl+1")]);
        assert_eq!(fake.registered(), set(&["Ctrl+2"]));
        assert_eq!(hotkeys.sync(true, &wanted), [key("Ctrl+1")], "asked again");

        // The other program let it go.
        fake.0.borrow_mut().taken.clear();
        assert!(hotkeys.sync(true, &wanted).is_empty());
        assert_eq!(fake.registered(), set(&["Ctrl+1", "Ctrl+2"]));
    }

    #[test]
    fn a_press_becomes_its_action() {
        let (mut hotkeys, fake) = hotkeys();
        hotkeys.sync(
            true,
            &[
                (key("Ctrl+1"), HotkeyAction::Play(SoundId(7))),
                (key("F9"), HotkeyAction::StopAll),
            ],
        );
        fake.press(key("Ctrl+1"));
        fake.press(key("F9"));
        fake.press(key("Ctrl+1"));
        assert_eq!(
            hotkeys.poll(),
            [
                HotkeyAction::Play(SoundId(7)),
                HotkeyAction::StopAll,
                HotkeyAction::Play(SoundId(7)),
            ]
        );
        assert!(hotkeys.poll().is_empty(), "each press is read once");
    }

    #[test]
    fn a_press_of_a_hotkey_no_longer_wanted_is_dropped() {
        let (mut hotkeys, fake) = hotkeys();
        hotkeys.sync(true, &[(key("Ctrl+1"), HotkeyAction::Play(SoundId(1)))]);
        fake.press(key("Ctrl+1"));
        hotkeys.sync(true, &[(key("Ctrl+2"), HotkeyAction::Play(SoundId(2)))]);
        assert!(hotkeys.poll().is_empty());
    }

    #[test]
    fn try_register_leaves_nothing_behind() {
        let (mut hotkeys, fake) = hotkeys();
        hotkeys.sync(true, &[(key("Ctrl+1"), HotkeyAction::Play(SoundId(1)))]);
        assert!(hotkeys.try_register(key("Ctrl+Alt+K")));
        assert!(hotkeys.try_register(key("Ctrl+1")), "Aria holds it already");
        assert_eq!(fake.registered(), set(&["Ctrl+1"]));
        fake.take(key("Ctrl+Alt+J"));
        assert!(!hotkeys.try_register(key("Ctrl+Alt+J")));
        assert_eq!(fake.registered(), set(&["Ctrl+1"]));
    }

    #[test]
    fn no_hotkeys_refuses_everything() {
        let mut hotkeys = Hotkeys::new(NoHotkeys);
        let wanted = [(key("F9"), HotkeyAction::StopAll)];
        assert_eq!(hotkeys.sync(true, &wanted), [key("F9")]);
        assert!(!hotkeys.try_register(key("F9")));
        assert!(hotkeys.poll().is_empty());
    }

    /// Every key a hotkey can use.
    fn every_key() -> Vec<HotkeyKey> {
        let mut keys: Vec<HotkeyKey> = ('A'..='Z').map(HotkeyKey::Letter).collect();
        keys.extend((0..=9).map(HotkeyKey::Digit));
        keys.extend((0..=9).map(HotkeyKey::Numpad));
        keys.extend((1..=F_KEY_MAX).map(HotkeyKey::F));
        keys.extend([
            HotkeyKey::NumpadAdd,
            HotkeyKey::NumpadSubtract,
            HotkeyKey::NumpadMultiply,
            HotkeyKey::NumpadDivide,
            HotkeyKey::NumpadDecimal,
        ]);
        keys
    }

    #[test]
    fn every_hotkey_key_maps_to_a_system_key() {
        let keys = every_key();
        assert_eq!(keys.len(), 26 + 10 + 10 + 24 + 5);
        let mut codes = HashSet::new();
        for hotkey_key in keys {
            let hotkey = Hotkey {
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
                key: hotkey_key,
            };
            let system = to_system(hotkey).unwrap_or_else(|| panic!("{hotkey_key:?} unmapped"));
            // No two keys of ours share a system key.
            assert!(
                codes.insert(system.key),
                "{hotkey_key:?} shares {}",
                system.key
            );
            assert_eq!(system.mods, Modifiers::CONTROL, "{hotkey_key:?}");
        }
        assert_eq!(
            to_system(key("Ctrl+Alt+Shift+Win+A")).map(|h| (h.mods, h.key)),
            Some((
                Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META,
                Code::KeyA
            ))
        );
        assert_eq!(to_system(key("F24")).map(|h| h.key), Some(Code::F24));
        assert_eq!(
            to_system(key("Numpad3")).map(|h| h.key),
            Some(Code::Numpad3)
        );
        assert_eq!(to_system(key("Ctrl+7")).map(|h| h.key), Some(Code::Digit7));
        assert_eq!(to_system(key("Ctrl+Q")).map(|h| h.key), Some(Code::KeyQ));
    }

    #[test]
    fn a_key_no_hotkey_can_use_has_no_system_key() {
        for bad in [
            HotkeyKey::Letter('a'),
            HotkeyKey::Letter('!'),
            HotkeyKey::Digit(10),
            HotkeyKey::Numpad(10),
            HotkeyKey::F(0),
            HotkeyKey::F(25),
        ] {
            let hotkey = Hotkey {
                ctrl: true,
                alt: false,
                shift: false,
                win: false,
                key: bad,
            };
            assert!(to_system(hotkey).is_none(), "{bad:?}");
        }
    }
}
