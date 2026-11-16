//! The soundboard as the backend keeps it.
//!
//! The library thread owns the sounds. It sends them as [`SoundView`]s, with `missing` checked on
//! that thread, after it opens and after every change, and it answers a sound fired with a
//! [`FiredSound`] once it has found its file. The [`Soundboard`] on the backend worker adds which
//! sounds are playing, as the engine reports them, and turns a fired sound into the engine's
//! request.
//!
//! The engine's token for a sound is its [`SoundId`], so firing a sound that is playing starts it
//! again.

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use aria_audio::{DeviceId, EngineCommand, EngineEvent, SoundRoute};
use aria_library::{SOUNDS_MAX, SoundId};
use serde::{Deserialize, Serialize};

use crate::{
    event::Event,
    hotkey::Hotkey,
    notice::{Notice, NoticeLevel},
    settings::Settings,
};

/// A sound name was empty or too long.
pub(crate) const SOUND_NAME_INVALID: Notice =
    Notice::new(NoticeLevel::Warning, "sound-name-invalid");
/// A sound's file is not where it was.
pub(crate) const SOUND_MISSING: Notice = Notice::new(NoticeLevel::Warning, "sound-missing");
/// The engine could not play a sound.
pub(crate) const SOUND_FAILED: Notice = Notice::new(NoticeLevel::Warning, "sound-failed");
/// The second device could not be used for a sound.
pub(crate) const SOUND_DEVICE_FAILED: Notice =
    Notice::new(NoticeLevel::Warning, "sound-device-failed");
/// A hotkey would take a key that types.
pub(crate) const HOTKEY_NOT_ALLOWED: Notice =
    Notice::new(NoticeLevel::Warning, "hotkey-not-allowed");
/// A hotkey is already used inside Aria.
pub(crate) const HOTKEY_TAKEN: Notice = Notice::new(NoticeLevel::Warning, "hotkey-taken");

/// Some files were not added, because the soundboard is full.
pub(crate) fn sound_limit() -> Notice {
    Notice::new(NoticeLevel::Warning, "sound-limit").with_arg("max", SOUNDS_MAX.to_string())
}

/// One sound, as the UI shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundView {
    pub id: SoundId,
    pub name: String,
    /// 0.0 to 1.0.
    pub volume: f32,
    pub hotkey: Option<Hotkey>,
    /// The file was not there when this was built.
    pub missing: bool,
    pub playing: bool,
}

/// The sounds, in the order they were added.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SoundboardState {
    pub sounds: Vec<SoundView>,
}

/// Where sounds are played.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SoundTarget {
    /// The device the music uses.
    #[default]
    Mine,
    /// The second device. Without one, as [`Mine`](Self::Mine).
    Second,
    /// Both devices. Without a second one, as [`Mine`](Self::Mine).
    Both,
}

impl SoundTarget {
    /// The engine's name for it. The engine applies the rules for a missing second device.
    pub fn route(self) -> SoundRoute {
        match self {
            Self::Mine => SoundRoute::Mine,
            Self::Second => SoundRoute::Second,
            Self::Both => SoundRoute::Both,
        }
    }
}

/// A sound the library found on disk, for the engine to play.
#[derive(Debug, Clone, PartialEq)]
pub struct FiredSound {
    pub id: SoundId,
    pub path: PathBuf,
    /// The sound's volume, 0.0 to 1.0.
    pub gain: f32,
}

/// A hotkey as stored by the library: text that is empty or not a hotkey counts as none.
pub(crate) fn stored_hotkey(text: Option<&str>) -> Option<Hotkey> {
    let text = text?;
    match text.parse() {
        Ok(hotkey) => Some(hotkey),
        Err(_) => {
            if !text.trim().is_empty() {
                tracing::debug!("a stored hotkey that cannot be read is ignored");
            }
            None
        }
    }
}

/// Why `hotkey` may not be used, as the notice that says so, or `None` if it may. `taken` are the
/// hotkeys already in use inside Aria, apart from the one it would replace.
pub(crate) fn hotkey_refusal(
    hotkey: &Hotkey,
    taken: impl IntoIterator<Item = Hotkey>,
) -> Option<Notice> {
    if !hotkey.is_allowed() {
        Some(HOTKEY_NOT_ALLOWED)
    } else if taken.into_iter().any(|other| other == *hotkey) {
        Some(HOTKEY_TAKEN)
    } else {
        None
    }
}

/// The engine's token for a sound.
fn token(id: SoundId) -> Option<u64> {
    u64::try_from(id.0).ok()
}

/// The sound a token names.
fn sound_of(token: u64) -> Option<SoundId> {
    i64::try_from(token).ok().map(SoundId)
}

/// The soundboard on the backend worker: the library's sounds, which of them play, and the
/// settings that concern them.
#[derive(Debug, Default)]
pub(crate) struct Soundboard {
    /// As the library last sent them; `playing` is filled in when the state is built.
    sounds: Vec<SoundView>,
    /// The sounds the engine says are playing.
    playing: HashSet<SoundId>,
    target: SoundTarget,
    /// The second device the engine was last told of.
    device: Option<DeviceId>,
    stop_hotkey: Option<Hotkey>,
}

impl Soundboard {
    /// Takes in the settings. Returns the request that tells the engine of a new second device.
    pub(crate) fn on_settings(&mut self, settings: &Settings) -> Option<EngineCommand> {
        self.target = settings.soundboard_target;
        self.stop_hotkey = settings.stop_sounds_hotkey;
        if settings.soundboard_device == self.device {
            return None;
        }
        self.device.clone_from(&settings.soundboard_device);
        Some(EngineCommand::SetSoundDevice(self.device.clone()))
    }

    /// The hotkey of "Stop all", as the settings last said.
    pub(crate) fn stop_hotkey(&self) -> Option<Hotkey> {
        self.stop_hotkey
    }

    /// Why the "Stop all" hotkey of `settings` may not be used, or `None` if it may or has not
    /// changed.
    pub(crate) fn stop_hotkey_refusal(&self, settings: &Settings) -> Option<Notice> {
        let hotkey = settings.stop_sounds_hotkey?;
        if Some(hotkey) == self.stop_hotkey {
            return None;
        }
        hotkey_refusal(&hotkey, self.sounds.iter().filter_map(|sound| sound.hotkey))
    }

    /// Takes in the sounds the library sent, and returns the state to show.
    pub(crate) fn on_sounds(&mut self, sounds: Vec<SoundView>) -> Event {
        self.sounds = sounds;
        self.state()
    }

    /// The library is gone: no sounds are left to show.
    pub(crate) fn clear(&mut self) -> Event {
        self.sounds.clear();
        self.state()
    }

    /// The engine is gone, and every sound with it. Returns the state to show, if it changed.
    pub(crate) fn silence(&mut self) -> Option<Event> {
        if self.playing.is_empty() {
            return None;
        }
        self.playing.clear();
        Some(self.state())
    }

    /// The engine's request for a fired sound, on the outputs the settings name.
    pub(crate) fn play(&self, fired: FiredSound) -> Option<EngineCommand> {
        let Some(token) = token(fired.id) else {
            tracing::debug!(id = fired.id.0, "a sound with an id no token can name");
            return None;
        };
        Some(EngineCommand::PlaySound {
            token,
            path: fired.path,
            gain: fired.gain,
            route: self.target.route(),
        })
    }

    /// Acts on what the engine says of sounds: a new state when one starts or ends, a notice when
    /// one fails. Other events are not for the soundboard.
    pub(crate) fn on_engine(&mut self, event: EngineEvent) -> Option<Event> {
        match event {
            EngineEvent::SoundStarted { token } => {
                let id = sound_of(token)?;
                self.playing.insert(id).then(|| self.state())
            }
            EngineEvent::SoundEnded { token } => {
                let id = sound_of(token)?;
                self.playing.remove(&id).then(|| self.state())
            }
            EngineEvent::SoundFailed { .. } => Some(Event::Notice(SOUND_FAILED)),
            EngineEvent::SoundDeviceFailed => Some(Event::Notice(SOUND_DEVICE_FAILED)),
            _ => None,
        }
    }

    fn state(&self) -> Event {
        let sounds = self
            .sounds
            .iter()
            .map(|sound| SoundView {
                playing: self.playing.contains(&sound.id),
                ..sound.clone()
            })
            .collect();
        Event::Soundboard(Arc::new(SoundboardState { sounds }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Hotkey {
        text.parse().unwrap()
    }

    fn view(id: i64, hotkey: Option<&str>) -> SoundView {
        SoundView {
            id: SoundId(id),
            name: format!("Sound {id}"),
            volume: 1.0,
            hotkey: hotkey.map(key),
            missing: false,
            playing: false,
        }
    }

    fn state(event: Option<Event>) -> Arc<SoundboardState> {
        match event {
            Some(Event::Soundboard(state)) => state,
            other => panic!("not a soundboard state: {other:?}"),
        }
    }

    fn playing(state: &SoundboardState) -> Vec<i64> {
        state
            .sounds
            .iter()
            .filter(|sound| sound.playing)
            .map(|sound| sound.id.0)
            .collect()
    }

    #[test]
    fn a_fired_sound_becomes_a_request_with_its_path_gain_and_route() {
        let mut board = Soundboard::default();
        let fired = FiredSound {
            id: SoundId(7),
            path: PathBuf::from("horn.ogg"),
            gain: 0.25,
        };
        for (target, route) in [
            (SoundTarget::Mine, SoundRoute::Mine),
            (SoundTarget::Second, SoundRoute::Second),
            (SoundTarget::Both, SoundRoute::Both),
        ] {
            board.on_settings(&Settings {
                soundboard_target: target,
                ..Settings::default()
            });
            assert_eq!(
                board.play(fired.clone()),
                Some(EngineCommand::PlaySound {
                    token: 7,
                    path: PathBuf::from("horn.ogg"),
                    gain: 0.25,
                    route,
                })
            );
        }
    }

    #[test]
    fn the_device_is_sent_only_when_it_changes() {
        let mut board = Soundboard::default();
        let cable = DeviceId("cable".into());
        let with = |device: Option<DeviceId>| Settings {
            soundboard_device: device,
            ..Settings::default()
        };
        assert_eq!(
            board.on_settings(&with(None)),
            None,
            "the engine starts with none"
        );
        assert_eq!(
            board.on_settings(&with(Some(cable.clone()))),
            Some(EngineCommand::SetSoundDevice(Some(cable.clone())))
        );
        assert_eq!(board.on_settings(&with(Some(cable))), None);
        assert_eq!(
            board.on_settings(&with(None)),
            Some(EngineCommand::SetSoundDevice(None))
        );
    }

    #[test]
    fn starting_and_ending_mark_the_sound() {
        let mut board = Soundboard::default();
        board.on_sounds(vec![view(1, None), view(2, None)]);
        let started = state(board.on_engine(EngineEvent::SoundStarted { token: 2 }));
        assert_eq!(playing(&started), [2]);
        // A restart changes nothing that shows.
        assert_eq!(
            board.on_engine(EngineEvent::SoundStarted { token: 2 }),
            None
        );
        let ended = state(board.on_engine(EngineEvent::SoundEnded { token: 2 }));
        assert!(playing(&ended).is_empty());
        // The end of a sound that played nowhere changes nothing either.
        assert_eq!(board.on_engine(EngineEvent::SoundEnded { token: 1 }), None);
        // A failure leaves a sound playing under the same token alone.
        board.on_engine(EngineEvent::SoundStarted { token: 1 });
        assert_eq!(
            board.on_engine(EngineEvent::SoundFailed { token: 1 }),
            Some(Event::Notice(SOUND_FAILED))
        );
        assert_eq!(playing(&state(Some(board.state()))), [1]);
    }

    #[test]
    fn a_new_list_keeps_what_plays() {
        let mut board = Soundboard::default();
        board.on_sounds(vec![view(1, None)]);
        board.on_engine(EngineEvent::SoundStarted { token: 1 });
        let renamed = SoundView {
            name: "Horn".into(),
            ..view(1, None)
        };
        let shown = state(Some(board.on_sounds(vec![renamed])));
        assert_eq!(shown.sounds[0].name, "Horn");
        assert!(shown.sounds[0].playing);
    }

    #[test]
    fn the_device_failing_gives_a_notice() {
        let mut board = Soundboard::default();
        assert_eq!(
            board.on_engine(EngineEvent::SoundDeviceFailed),
            Some(Event::Notice(SOUND_DEVICE_FAILED))
        );
        assert_eq!(board.on_engine(EngineEvent::Stopped), None);
    }

    #[test]
    fn a_stop_hotkey_used_by_a_sound_is_refused() {
        let mut board = Soundboard::default();
        board.on_sounds(vec![view(1, Some("Ctrl+F1")), view(2, None)]);
        let with = |hotkey: Option<&str>| Settings {
            stop_sounds_hotkey: hotkey.map(key),
            ..Settings::default()
        };
        assert_eq!(
            board.stop_hotkey_refusal(&with(Some("Ctrl+F1"))),
            Some(HOTKEY_TAKEN)
        );
        assert_eq!(
            board.stop_hotkey_refusal(&with(Some("Shift+S"))),
            Some(HOTKEY_NOT_ALLOWED)
        );
        assert_eq!(board.stop_hotkey_refusal(&with(Some("Ctrl+F2"))), None);
        assert_eq!(board.stop_hotkey_refusal(&with(None)), None);
        // One already in effect is not checked again.
        board.on_settings(&with(Some("Ctrl+F1")));
        assert_eq!(board.stop_hotkey_refusal(&with(Some("Ctrl+F1"))), None);
    }

    #[test]
    fn a_hotkey_is_refused_when_taken_or_not_allowed() {
        assert_eq!(
            hotkey_refusal(&key("A"), []),
            Some(HOTKEY_NOT_ALLOWED),
            "even when free"
        );
        assert_eq!(
            hotkey_refusal(&key("Ctrl+A"), [key("Alt+B"), key("ctrl+a")]),
            Some(HOTKEY_TAKEN)
        );
        assert_eq!(hotkey_refusal(&key("Ctrl+A"), [key("Alt+A")]), None);
    }

    #[test]
    fn a_stored_hotkey_that_cannot_be_read_is_none() {
        assert_eq!(stored_hotkey(None), None);
        assert_eq!(stored_hotkey(Some("")), None);
        assert_eq!(stored_hotkey(Some("Hyper+Q")), None);
        assert_eq!(stored_hotkey(Some("Ctrl+F1")), Some(key("Ctrl+F1")));
    }

    #[test]
    fn the_target_is_written_in_lower_case() {
        #[derive(Serialize, Deserialize)]
        struct Holder {
            target: SoundTarget,
        }
        let text = toml::to_string(&Holder {
            target: SoundTarget::Second,
        })
        .unwrap();
        assert_eq!(text.trim(), "target = \"second\"");
        for (text, target) in [
            ("mine", SoundTarget::Mine),
            ("second", SoundTarget::Second),
            ("both", SoundTarget::Both),
        ] {
            let holder: Holder = toml::from_str(&format!("target = \"{text}\"")).unwrap();
            assert_eq!(holder.target, target);
        }
        assert_eq!(SoundTarget::default(), SoundTarget::Mine);
    }

    #[test]
    fn the_limit_notice_names_the_limit() {
        assert_eq!(sound_limit().args, [("max", "100".to_owned())]);
    }
}
