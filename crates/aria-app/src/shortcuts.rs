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
        toggle_right_panel: input.consume_shortcut(&TOGGLE_RIGHT_PANEL),
        ..Shortcuts::default()
    };
    for (route, key) in Route::ALL.into_iter().zip(ROUTE_KEYS) {
        if consume_ctrl_digit(input, key) {
            shortcuts.jump = Some(route);
        }
    }
    shortcuts.back =
        input.consume_shortcut(&BACK) | input.pointer.button_pressed(PointerButton::Extra1);
    shortcuts.forward =
        input.consume_shortcut(&FORWARD) | input.pointer.button_pressed(PointerButton::Extra2);
    shortcuts
}

/// Consumes Ctrl+`digit` (Cmd on macOS) pressed with no other modifier.
///
/// `consume_shortcut` ignores extra Alt and Shift, but Windows reports AltGr as Ctrl+Alt,
/// and AltGr+digit types a character on many keyboard layouts.
fn consume_ctrl_digit(input: &mut InputState, digit: Key) -> bool {
    let exact = input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == digit && modifiers.matches_exact(Modifiers::COMMAND)
        )
    });
    exact && input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, digit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ctrl_digit_is_consumed() {
        let mut input = InputState::default();
        input.events.push(key_event(Key::Num3, Modifiers::COMMAND));
        assert!(consume_ctrl_digit(&mut input, Key::Num3));
        assert!(input.events.is_empty(), "the shortcut was not consumed");
    }

    #[test]
    fn altgr_digit_is_left_alone() {
        let mut input = InputState::default();
        let altgr = Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND;
        input.events.push(key_event(Key::Num3, altgr));
        assert!(!consume_ctrl_digit(&mut input, Key::Num3));
        assert_eq!(input.events.len(), 1);
    }

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
