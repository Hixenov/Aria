//! The modal that captures a hotkey: it waits for a combination to be pressed and shows it.
//! Escape cancels; Delete or Backspace clears the hotkey.
//!
//! The dialog sees only what reaches the window while it is open. The app unregisters every
//! global hotkey for that time, so the combination being typed comes here and fires nothing.
//!
//! The window toolkit does not tell a numpad key from its twin on the main keyboard, so the
//! "Numpad key" switch says which was meant: with it on, a digit, `+`, `-` or `/` becomes the
//! numpad's. The numpad's `*` and `.` arrive as no key at all and cannot be caught. Neither can
//! the Windows key, which the toolkit does not report on Windows; on a Mac, Command counts as it.

use aria_core::{
    hotkey::{Hotkey, HotkeyKey},
    i18n::Localizer,
};
use eframe::egui::{
    Align, Button, Checkbox, Context, Event, Frame, Id, Key, Label, Layout, Margin, Modal,
    Modifiers, RichText, Stroke, WidgetInfo, accesskit::Role, vec2,
};

use crate::ui::theme::{
    Palette, fonts,
    visuals::{SPACE, TITLE_SIZE},
};

/// Width of the dialog's content.
const WIDTH: f32 = 360.0;
/// Least width of its buttons.
const BUTTON_WIDTH: f32 = 88.0;
/// Corner radius of its card.
const RADIUS: u8 = 8;
/// Size of the combination as it is shown.
const COMBO_SIZE: f32 = 22.0;
/// Id of the modal.
const ID: &str = "aria_hotkey_dialog";

/// What the user did with the dialog this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAnswer {
    /// Still open.
    Open,
    /// Closed by Cancel, Escape or a click outside.
    Cancelled,
    /// Closed by Clear, Delete or Backspace: the hotkey is to be removed.
    Cleared,
    /// Confirmed with this hotkey, which is allowed and not used elsewhere in Aria.
    Confirmed(Hotkey),
}

/// What the dialog knows of the hotkeys.
pub struct HotkeyDialog<'a> {
    /// The hotkey being replaced, if there is one.
    pub current: Option<Hotkey>,
    /// The hotkeys used elsewhere in Aria: by other sounds, or by "Stop all".
    pub taken: &'a [Hotkey],
}

/// A key press as the window reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pressed {
    /// The key as the keyboard layout names it.
    logical: Key,
    /// The key by its place on a US keyboard, when the window knows it.
    physical: Option<Key>,
    modifiers: Modifiers,
}

/// What has been pressed in the dialog so far, and whether a numpad key is meant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotkeyCapture {
    pressed: Option<Pressed>,
    /// The "Numpad key" switch.
    numpad: bool,
}

impl HotkeyCapture {
    /// Nothing pressed yet. The switch starts on when `current` is a numpad key.
    pub fn new(current: Option<Hotkey>) -> Self {
        Self {
            pressed: None,
            numpad: current.is_some_and(|hotkey| is_numpad(hotkey.key)),
        }
    }

    /// The hotkey pressed, as the switch reads it, if the key can be one.
    pub fn hotkey(&self) -> Option<Hotkey> {
        let pressed = self.pressed?;
        Some(Hotkey {
            ctrl: pressed.modifiers.ctrl,
            alt: pressed.modifiers.alt,
            shift: pressed.modifiers.shift,
            // The Command key on a Mac; Windows does not report its key here.
            win: pressed.modifiers.mac_cmd,
            key: resolve(pressed, self.numpad)?,
        })
    }

    /// Whether the switch can change the hotkey: nothing is pressed yet, or the key pressed has a
    /// numpad twin.
    fn numpad_applies(&self) -> bool {
        self.pressed
            .is_none_or(|pressed| letter(pressed).is_none() && numpad_key(place(pressed)).is_some())
    }
}

/// Whether `key` is on the numpad.
fn is_numpad(key: HotkeyKey) -> bool {
    matches!(
        key,
        HotkeyKey::Numpad(_)
            | HotkeyKey::NumpadAdd
            | HotkeyKey::NumpadSubtract
            | HotkeyKey::NumpadMultiply
            | HotkeyKey::NumpadDivide
            | HotkeyKey::NumpadDecimal
    )
}

/// Why a combination cannot be confirmed, as the key of the text that says so.
fn refusal(hotkey: Hotkey, taken: &[Hotkey]) -> Option<&'static str> {
    if !hotkey.is_allowed() {
        Some("hotkey-not-allowed")
    } else if taken.contains(&hotkey) {
        Some("hotkey-taken")
    } else {
        None
    }
}

/// The hotkey key of `key`: a letter, a digit or F1 to F24. Other keys are not caught.
fn hotkey_key(key: Key) -> Option<HotkeyKey> {
    match key.name().parse() {
        Ok(found @ (HotkeyKey::Letter(_) | HotkeyKey::Digit(_) | HotkeyKey::F(_))) => Some(found),
        _ => None,
    }
}

/// The numpad key that `key` may stand for: a digit, `+`, `-` or `/`.
fn numpad_key(key: Key) -> Option<HotkeyKey> {
    match key {
        Key::Plus => Some(HotkeyKey::NumpadAdd),
        Key::Minus => Some(HotkeyKey::NumpadSubtract),
        Key::Slash => Some(HotkeyKey::NumpadDivide),
        other => match hotkey_key(other) {
            Some(HotkeyKey::Digit(d)) => Some(HotkeyKey::Numpad(d)),
            _ => None,
        },
    }
}

/// The letter pressed, as the layout names it. The system's hotkeys follow the layout's
/// letters, so a letter is taken from the logical key.
fn letter(pressed: Pressed) -> Option<HotkeyKey> {
    hotkey_key(pressed.logical).filter(|key| matches!(key, HotkeyKey::Letter(_)))
}

/// The key by its place, so that Shift+1 is still 1 and not `!`.
fn place(pressed: Pressed) -> Key {
    pressed.physical.unwrap_or(pressed.logical)
}

/// The hotkey key of a press, with the "Numpad key" switch as `numpad`.
fn resolve(pressed: Pressed, numpad: bool) -> Option<HotkeyKey> {
    if let Some(found) = letter(pressed) {
        return Some(found);
    }
    let key = place(pressed);
    if numpad && let Some(found) = numpad_key(key) {
        return Some(found);
    }
    hotkey_key(key)
}

/// What the key presses of this frame ask for: an answer, or none while the dialog stays open.
/// A press of a key that can be part of a hotkey is kept in `capture`.
fn read_keys(events: &[Event], capture: &mut HotkeyCapture) -> Option<HotkeyAnswer> {
    for event in events {
        let Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers,
        } = event
        else {
            continue;
        };
        match key {
            Key::Escape => return Some(HotkeyAnswer::Cancelled),
            Key::Delete | Key::Backspace => return Some(HotkeyAnswer::Cleared),
            _ => {
                let pressed = Pressed {
                    logical: *key,
                    physical: *physical_key,
                    modifiers: *modifiers,
                };
                // Kept when some position of the switch makes a hotkey of it.
                if resolve(pressed, true).is_some() {
                    capture.pressed = Some(pressed);
                }
            }
        }
    }
    None
}

impl HotkeyDialog<'_> {
    /// Draws the dialog. `capture` is what was pressed so far; the confirm button works only
    /// while it makes a hotkey that is allowed and not used elsewhere in Aria.
    pub fn show(
        &self,
        ctx: &Context,
        tr: &Localizer,
        p: &Palette,
        capture: &mut HotkeyCapture,
    ) -> HotkeyAnswer {
        let frame = Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS)
            .inner_margin(Margin::same(SPACE[4] as i8))
            .shadow(ctx.global_style().visuals.window_shadow);
        let result = Modal::new(Id::unique(ID)).frame(frame).show(ctx, |ui| {
            ui.set_width(WIDTH);
            let pressed = ui.input(|input| read_keys(&input.events, capture));

            let title = tr.tr("hotkey-dialog-title");
            let heading = RichText::new(&*title)
                .font(fonts::semibold(TITLE_SIZE))
                .color(p.text);
            let response = ui.add(Label::new(heading).selectable(false).wrap());
            response.widget_info(|| WidgetInfo::labeled(Role::Heading, true, &*title));
            ui.add_space(SPACE[1]);
            let hint = tr.tr("hotkey-dialog-hint");
            ui.add(
                Label::new(RichText::new(&*hint).color(p.text_muted))
                    .selectable(false)
                    .wrap(),
            );
            ui.add_space(SPACE[2]);
            let applies = capture.numpad_applies();
            let numpad = Checkbox::new(&mut capture.numpad, tr.tr("hotkey-dialog-numpad"));
            ui.add_enabled(applies, numpad);
            ui.add_space(SPACE[2]);

            // The combination pressed, or else the hotkey as it is now.
            let captured = capture.hotkey();
            let (shown, color) = match (captured, self.current) {
                (Some(hotkey), _) => (hotkey.to_string(), p.text),
                (None, Some(hotkey)) => (hotkey.to_string(), p.text_muted),
                (None, None) => (tr.tr("sound-hotkey-none").into_owned(), p.text_muted),
            };
            let combo = RichText::new(shown)
                .font(fonts::semibold(COMBO_SIZE))
                .color(color);
            ui.add(Label::new(combo).selectable(false));
            let refused = captured.and_then(|hotkey| refusal(hotkey, self.taken));
            if let Some(key) = refused {
                ui.add_space(SPACE[1]);
                let why = RichText::new(tr.tr(key)).color(p.danger);
                ui.add(Label::new(why).selectable(false).wrap());
            }
            let confirmable = captured.filter(|_| refused.is_none());

            ui.add_space(SPACE[4]);
            let clicked = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let min_size = vec2(BUTTON_WIDTH, 0.0);
                    let confirm = RichText::new(tr.tr("settings-confirm")).color(p.on_accent());
                    let confirm = Button::new(confirm).fill(p.accent).min_size(min_size);
                    if ui.add_enabled(confirmable.is_some(), confirm).clicked()
                        && let Some(hotkey) = confirmable
                    {
                        return Some(HotkeyAnswer::Confirmed(hotkey));
                    }
                    let cancel = Button::new(tr.tr("settings-cancel")).min_size(min_size);
                    if ui.add(cancel).clicked() {
                        return Some(HotkeyAnswer::Cancelled);
                    }
                    let clear = Button::new(tr.tr("hotkey-dialog-clear")).min_size(min_size);
                    ui.add_enabled(self.current.is_some(), clear)
                        .clicked()
                        .then_some(HotkeyAnswer::Cleared)
                })
                .inner;
            pressed.or(clicked)
        });
        match result.inner {
            Some(answer) => answer,
            None if result.should_close() => HotkeyAnswer::Cancelled,
            None => HotkeyAnswer::Open,
        }
    }
}

#[cfg(test)]
mod tests {
    use aria_core::settings::{Language, Settings};
    use eframe::egui::{self, Modifiers, Theme};
    use egui_kittest::{
        Harness,
        kittest::{NodeT as _, Queryable as _},
    };

    use super::*;

    fn key(text: &str) -> Hotkey {
        text.parse()
            .unwrap_or_else(|err| panic!("{text} is a hotkey: {err}"))
    }

    /// The dialog's world: what it is given, what was pressed, and its last answer.
    struct World {
        tr: Localizer,
        current: Option<Hotkey>,
        taken: Vec<Hotkey>,
        capture: HotkeyCapture,
        answer: HotkeyAnswer,
    }

    /// The dialog over `current` and `taken`, drawn once.
    fn dialog(current: Option<Hotkey>, taken: Vec<Hotkey>) -> Harness<'static, World> {
        let world = World {
            tr: Localizer::new(Language::En).unwrap_or_else(|err| panic!("English: {err}")),
            current,
            taken,
            capture: HotkeyCapture::new(current),
            answer: HotkeyAnswer::Open,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .with_context_setup(fonts::install)
            .build_ui_state(
                |ui, world: &mut World| {
                    // Once answered, the dialog is gone, as the app closes it.
                    if world.answer != HotkeyAnswer::Open {
                        return;
                    }
                    let palette = Palette::for_theme(Theme::Dark, &Settings::default());
                    let dialog = HotkeyDialog {
                        current: world.current,
                        taken: &world.taken,
                    };
                    world.answer = dialog.show(ui.ctx(), &world.tr, &palette, &mut world.capture);
                },
                world,
            );
        harness.run();
        harness
    }

    fn en(harness: &Harness<'_, World>, key: &str) -> String {
        harness.state().tr.tr(key).into_owned()
    }

    fn confirm_disabled(harness: &Harness<'_, World>) -> bool {
        let label = en(harness, "settings-confirm");
        harness
            .get_by_role_and_label(Role::Button, &label)
            .accesskit_node()
            .is_disabled()
    }

    #[test]
    fn pressing_a_combination_shows_it_and_confirm_sends_it() {
        let mut harness = dialog(None, vec![]);
        harness.get_by_label(&en(&harness, "hotkey-dialog-title"));
        harness.get_by_label(&en(&harness, "sound-hotkey-none"));
        assert!(confirm_disabled(&harness), "nothing pressed yet");

        harness.key_press_modifiers(Modifiers::CTRL | Modifiers::ALT, Key::K);
        harness.run();
        harness.get_by_label("Ctrl+Alt+K");
        assert!(!confirm_disabled(&harness));

        // A second combination replaces the first.
        harness.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::F5);
        harness.run();
        assert!(harness.query_by_label("Ctrl+Alt+K").is_none());
        harness.get_by_label("Ctrl+Shift+F5");

        let label = en(&harness, "settings-confirm");
        harness.get_by_role_and_label(Role::Button, &label).click();
        harness.run();
        assert_eq!(
            harness.state().answer,
            HotkeyAnswer::Confirmed(key("Ctrl+Shift+F5"))
        );
    }

    #[test]
    fn escape_cancels() {
        let mut harness = dialog(Some(key("F9")), vec![]);
        harness.get_by_label("F9");
        harness.key_press_modifiers(Modifiers::CTRL, Key::A);
        harness.run();
        harness.key_press(Key::Escape);
        harness.run();
        assert_eq!(harness.state().answer, HotkeyAnswer::Cancelled);
    }

    #[test]
    fn delete_clears_the_hotkey() {
        for clear in [Key::Delete, Key::Backspace] {
            let mut harness = dialog(Some(key("F9")), vec![]);
            harness.key_press(clear);
            harness.run();
            assert_eq!(harness.state().answer, HotkeyAnswer::Cleared, "{clear:?}");
        }
        // And the button does the same.
        let mut harness = dialog(Some(key("F9")), vec![]);
        let label = en(&harness, "hotkey-dialog-clear");
        harness.get_by_role_and_label(Role::Button, &label).click();
        harness.run();
        assert_eq!(harness.state().answer, HotkeyAnswer::Cleared);
    }

    #[test]
    fn a_combination_that_is_not_allowed_cannot_be_confirmed() {
        let mut harness = dialog(None, vec![]);
        for (modifiers, pressed, text) in [
            (Modifiers::NONE, Key::A, "A"),
            (Modifiers::SHIFT, Key::Num3, "Shift+3"),
        ] {
            harness.key_press_modifiers(modifiers, pressed);
            harness.run();
            harness.get_by_label(text);
            harness.get_by_label(&en(&harness, "hotkey-not-allowed"));
            assert!(confirm_disabled(&harness), "{text} can be confirmed");
        }
        // An F key may stand alone.
        harness.key_press(Key::F7);
        harness.run();
        assert!(
            harness
                .query_by_label(&en(&harness, "hotkey-not-allowed"))
                .is_none()
        );
        assert!(!confirm_disabled(&harness));
    }

    #[test]
    fn a_combination_used_elsewhere_in_aria_cannot_be_confirmed() {
        let mut harness = dialog(None, vec![key("Ctrl+1")]);
        harness.key_press_modifiers(Modifiers::CTRL, Key::Num1);
        harness.run();
        harness.get_by_label(&en(&harness, "hotkey-taken"));
        assert!(confirm_disabled(&harness));
        assert_eq!(harness.state().answer, HotkeyAnswer::Open);
    }

    #[test]
    fn only_letters_digits_and_f_keys_are_caught() {
        assert_eq!(hotkey_key(Key::Q), Some(HotkeyKey::Letter('Q')));
        assert_eq!(hotkey_key(Key::Num0), Some(HotkeyKey::Digit(0)));
        assert_eq!(hotkey_key(Key::F24), Some(HotkeyKey::F(24)));
        for other in [
            Key::F25,
            Key::Space,
            Key::Enter,
            Key::Tab,
            Key::Plus,
            Key::ArrowUp,
        ] {
            assert_eq!(hotkey_key(other), None, "{other:?}");
        }
    }

    fn click(harness: &mut Harness<'_, World>, role: Role, key: &str) {
        let label = en(harness, key);
        harness.get_by_role_and_label(role, &label).click();
        harness.run();
    }

    #[test]
    fn a_numpad_digit_can_be_set_with_the_numpad_toggle() {
        // A bare numpad digit arrives as a digit, which may not stand alone; the switch says
        // it was the numpad's.
        let mut harness = dialog(None, vec![]);
        harness.key_press(Key::Num3);
        harness.run();
        harness.get_by_label("3");
        assert!(confirm_disabled(&harness));
        click(&mut harness, Role::CheckBox, "hotkey-dialog-numpad");
        harness.get_by_label("Numpad3");
        assert!(!confirm_disabled(&harness));
        click(&mut harness, Role::Button, "settings-confirm");
        assert_eq!(
            harness.state().answer,
            HotkeyAnswer::Confirmed(key("Numpad3"))
        );

        // With the switch on first, Ctrl and a numpad digit, or the numpad's +, are numpad keys:
        // never Ctrl and a main-keyboard digit.
        let mut harness = dialog(None, vec![]);
        click(&mut harness, Role::CheckBox, "hotkey-dialog-numpad");
        harness.key_press_modifiers(Modifiers::CTRL, Key::Num1);
        harness.run();
        harness.get_by_label("Ctrl+Numpad1");
        assert!(harness.query_by_label("Ctrl+1").is_none());
        harness.key_press(Key::Plus);
        harness.run();
        harness.get_by_label("NumpadAdd");
        click(&mut harness, Role::Button, "settings-confirm");
        assert_eq!(
            harness.state().answer,
            HotkeyAnswer::Confirmed(key("NumpadAdd"))
        );
    }

    #[test]
    fn the_numpad_toggle_starts_on_for_a_numpad_hotkey_and_leaves_letters_alone() {
        let mut harness = dialog(Some(key("Ctrl+Numpad5")), vec![]);
        harness.key_press_modifiers(Modifiers::CTRL, Key::Num7);
        harness.run();
        harness.get_by_label("Ctrl+Numpad7");

        harness.key_press_modifiers(Modifiers::CTRL, Key::K);
        harness.run();
        harness.get_by_label("Ctrl+K");
        let label = en(&harness, "hotkey-dialog-numpad");
        let toggle = harness.get_by_role_and_label(Role::CheckBox, &label);
        assert!(
            toggle.accesskit_node().is_disabled(),
            "a letter has no numpad twin"
        );
    }

    #[test]
    fn shift_with_a_digit_is_caught_by_its_physical_key() {
        // On a US layout Shift+1 types `!`; the key is still 1.
        let mut harness = dialog(None, vec![]);
        let modifiers = Modifiers::CTRL | Modifiers::SHIFT;
        harness.event_modifiers(
            egui::Event::Key {
                key: Key::Exclamationmark,
                physical_key: Some(Key::Num1),
                pressed: true,
                repeat: false,
                modifiers,
            },
            modifiers,
        );
        harness.run();
        harness.get_by_label("Ctrl+Shift+1");
        click(&mut harness, Role::Button, "settings-confirm");
        assert_eq!(
            harness.state().answer,
            HotkeyAnswer::Confirmed(key("Ctrl+Shift+1"))
        );
    }

    #[test]
    fn a_letter_follows_the_layout_not_the_place() {
        // On a French layout the key in the place of Q types A: the system's hotkeys follow
        // the layout, so it is A.
        let pressed = Pressed {
            logical: Key::A,
            physical: Some(Key::Q),
            modifiers: Modifiers::CTRL,
        };
        assert_eq!(resolve(pressed, false), Some(HotkeyKey::Letter('A')));
        assert_eq!(resolve(pressed, true), Some(HotkeyKey::Letter('A')));
    }
}
