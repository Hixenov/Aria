//! The modal that captures a hotkey: it waits for a combination to be pressed and shows it.
//! Escape cancels; Delete or Backspace clears the hotkey.
//!
//! The dialog sees only what reaches the window while it is open. The app unregisters every
//! global hotkey for that time, so the combination being typed comes here and fires nothing.
//! The window toolkit reports neither the Windows key nor whether a digit came from the numpad,
//! so the combinations caught here hold Ctrl, Alt and Shift with a letter, a digit or F1 to F24.

use aria_core::{
    hotkey::{Hotkey, HotkeyKey},
    i18n::Localizer,
};
use eframe::egui::{
    Align, Button, Context, Event, Frame, Id, Key, Label, Layout, Margin, Modal, RichText, Stroke,
    WidgetInfo, accesskit::Role, vec2,
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

/// What the key presses of this frame ask for: an answer, a newly caught combination, or both
/// when Escape or Delete follows one.
fn read_keys(events: &[Event], captured: &mut Option<Hotkey>) -> Option<HotkeyAnswer> {
    for event in events {
        let Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            continue;
        };
        match key {
            Key::Escape => return Some(HotkeyAnswer::Cancelled),
            Key::Delete | Key::Backspace => return Some(HotkeyAnswer::Cleared),
            other => {
                if let Some(found) = hotkey_key(*other) {
                    *captured = Some(Hotkey {
                        ctrl: modifiers.ctrl,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                        // The Command key on a Mac; Windows does not report its key here.
                        win: modifiers.mac_cmd,
                        key: found,
                    });
                }
            }
        }
    }
    None
}

impl HotkeyDialog<'_> {
    /// Draws the dialog. `captured` is the combination pressed so far; the confirm button works
    /// only while it is allowed and not used elsewhere in Aria.
    pub fn show(
        &self,
        ctx: &Context,
        tr: &Localizer,
        p: &Palette,
        captured: &mut Option<Hotkey>,
    ) -> HotkeyAnswer {
        let frame = Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS)
            .inner_margin(Margin::same(SPACE[4] as i8))
            .shadow(ctx.global_style().visuals.window_shadow);
        let result = Modal::new(Id::unique(ID)).frame(frame).show(ctx, |ui| {
            ui.set_width(WIDTH);
            let pressed = ui.input(|input| read_keys(&input.events, captured));

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
            ui.add_space(SPACE[3]);

            // The combination pressed, or else the hotkey as it is now.
            let (shown, color) = match (*captured, self.current) {
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
        captured: Option<Hotkey>,
        answer: HotkeyAnswer,
    }

    fn harness(current: Option<Hotkey>, taken: Vec<Hotkey>) -> Harness<'static, World> {
        let world = World {
            tr: Localizer::new(Language::En).unwrap_or_else(|err| panic!("English: {err}")),
            current,
            taken,
            captured: None,
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
                    world.answer = dialog.show(ui.ctx(), &world.tr, &palette, &mut world.captured);
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
        let mut harness = harness(None, vec![]);
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
        let mut harness = harness(Some(key("F9")), vec![]);
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
            let mut harness = harness(Some(key("F9")), vec![]);
            harness.key_press(clear);
            harness.run();
            assert_eq!(harness.state().answer, HotkeyAnswer::Cleared, "{clear:?}");
        }
        // And the button does the same.
        let mut harness = harness(Some(key("F9")), vec![]);
        let label = en(&harness, "hotkey-dialog-clear");
        harness.get_by_role_and_label(Role::Button, &label).click();
        harness.run();
        assert_eq!(harness.state().answer, HotkeyAnswer::Cleared);
    }

    #[test]
    fn a_combination_that_is_not_allowed_cannot_be_confirmed() {
        let mut harness = harness(None, vec![]);
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
        let mut harness = harness(None, vec![key("Ctrl+1")]);
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
}
