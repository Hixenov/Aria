//! Navigation and panel shortcuts: Ctrl+1…9, Alt+Left/Right, the mouse's back and forward
//! buttons, and Ctrl+Alt+R.

use eframe::egui::{self, InputState, Key, KeyboardShortcut, Modifiers, PointerButton};

use crate::nav::Route;

/// Ctrl+Alt+R (Cmd+Alt+R on macOS) toggles the right panel.
const TOGGLE_RIGHT_PANEL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::R);
/// Alt+Left steps back in history.
const BACK: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft);
/// Alt+Right steps forward in history.
const FORWARD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight);
/// With Ctrl (Cmd on macOS), these jump to the matching entry of [`Route::ALL`].
const ROUTE_KEYS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

/// What the keyboard and mouse asked for this frame.
#[derive(Default)]
pub(crate) struct Shortcuts {
    pub(crate) jump: Option<Route>,
    pub(crate) back: bool,
    pub(crate) forward: bool,
    pub(crate) toggle_right_panel: bool,
}

/// Reads and consumes this frame's shortcuts, so a focused text field doesn't act on them
/// too.
pub(crate) fn read_shortcuts(input: &mut InputState) -> Shortcuts {
    let mut shortcuts = Shortcuts {
        toggle_right_panel: consume_exact(input, &TOGGLE_RIGHT_PANEL),
        ..Shortcuts::default()
    };
    for (route, key) in Route::ALL.into_iter().zip(ROUTE_KEYS) {
        if consume_exact(input, &KeyboardShortcut::new(Modifiers::COMMAND, key)) {
            shortcuts.jump = Some(route);
        }
    }
    shortcuts.back =
        input.consume_shortcut(&BACK) | input.pointer.button_pressed(PointerButton::Extra1);
    shortcuts.forward =
        input.consume_shortcut(&FORWARD) | input.pointer.button_pressed(PointerButton::Extra2);
    shortcuts
}

/// Consumes `shortcut` if its key was pressed this frame with exactly its modifiers, and not
/// with AltGr.
///
/// `consume_shortcut` alone ignores extra Alt and Shift. Windows reports AltGr as Ctrl+Alt,
/// and AltGr+key types a character on many keyboard layouts (`²` on AltGr+2, `®` on AltGr+R
/// in US-International), so such a press is left for typing.
fn consume_exact(input: &mut InputState, shortcut: &KeyboardShortcut) -> bool {
    let altgr = altgr_involved(input);
    let pressed = input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == shortcut.logical_key
                    && modifiers.matches_exact(shortcut.modifiers)
                    && !(altgr && modifiers.ctrl && modifiers.alt)
        )
    });
    pressed && input.consume_shortcut(shortcut)
}

/// Whether the right Alt key, which is AltGr on many layouts, is held or changed this frame.
///
/// Windows reports AltGr with the same modifiers as the left Ctrl and Alt keys together;
/// only the physical key tells the two apart.
fn altgr_involved(input: &InputState) -> bool {
    input.key_down(Key::AltRight)
        || input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::AltRight,
                    ..
                }
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+`digit` (Cmd on macOS).
    fn ctrl(digit: Key) -> KeyboardShortcut {
        KeyboardShortcut::new(Modifiers::COMMAND, digit)
    }

    #[test]
    fn plain_ctrl_digit_is_consumed() {
        let mut input = InputState::default();
        input.events.push(key_event(Key::Num3, Modifiers::COMMAND));
        assert!(consume_exact(&mut input, &ctrl(Key::Num3)));
        assert!(input.events.is_empty(), "the shortcut was not consumed");
    }

    #[test]
    fn altgr_digit_is_left_alone() {
        let mut input = InputState::default();
        let altgr = Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND;
        input.events.push(key_event(Key::Num3, altgr));
        assert!(!consume_exact(&mut input, &ctrl(Key::Num3)));
        assert_eq!(input.events.len(), 1);
    }

    #[test]
    fn ctrl_alt_r_toggles_the_right_panel() {
        let mut input = InputState::default();
        input.keys_down.extend([Key::ControlLeft, Key::AltLeft]);
        input.events.push(key_event(Key::R, CTRL_ALT));
        assert!(read_shortcuts(&mut input).toggle_right_panel);
        assert!(input.events.is_empty(), "the shortcut was not consumed");
    }

    #[test]
    fn altgr_r_is_left_for_typing() {
        // Windows reports AltGr as Ctrl+Alt; only the right Alt key being down gives it away.
        let mut input = InputState::default();
        input.keys_down.extend([Key::ControlLeft, Key::AltRight]);
        input.events.push(key_event(Key::R, CTRL_ALT));
        assert!(!read_shortcuts(&mut input).toggle_right_panel);
        assert_eq!(input.events.len(), 1, "the key press was consumed");
    }

    #[test]
    fn altgr_released_in_the_same_frame_still_counts() {
        let mut input = InputState::default();
        input.events.push(key_event(Key::AltRight, CTRL_ALT));
        input.events.push(key_event(Key::R, CTRL_ALT));
        input.events.push(egui::Event::Key {
            key: Key::AltRight,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::CTRL | Modifiers::COMMAND,
        });
        assert!(!read_shortcuts(&mut input).toggle_right_panel);
    }

    #[test]
    fn ctrl_alt_shift_r_is_not_the_panel_shortcut() {
        let mut input = InputState::default();
        input
            .events
            .push(key_event(Key::R, CTRL_ALT | Modifiers::SHIFT));
        assert!(!read_shortcuts(&mut input).toggle_right_panel);
    }

    /// Ctrl+Alt as Windows and Linux report it: Ctrl is also the command key there.
    const CTRL_ALT: Modifiers = Modifiers {
        alt: true,
        ctrl: true,
        shift: false,
        mac_cmd: false,
        command: true,
    };

    fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
}
