//! The Soundboard page: a wrapping grid of tiles, one per sound. A click plays a sound, a
//! right click opens its menu: rename, hotkey, volume and remove. A hotkey the system refused is
//! struck through on its tile.
//!
//! The page keeps no copy of the sounds. Every change is a request, and the list it draws is
//! the one the backend last reported. A volume is sent only when the user lets go of the bar:
//! each change rebuilds the whole list in the library.

use aria_core::{
    i18n::{FluentArgs, Localizer},
    soundboard::SoundView,
};
use aria_library::SoundId;
use eframe::egui::{
    Align2, Button, CursorIcon, Id, Sense, Stroke, StrokeKind, TextStyle, Ui, WidgetInfo,
    accesskit::{Role, Toggled},
    text::{LayoutJob, TextFormat},
    vec2,
};

use super::{PageAction, PageCtx};
use crate::{
    nav::Route,
    ui::{
        theme::{
            Palette, fonts,
            fonts::semibold,
            icons,
            visuals::{SPACE, TITLE_SIZE},
        },
        widgets::{Confirm, Scrub, confirm_dialog, empty_state_with, section_header},
    },
};

/// Size of a tile.
const TILE: [f32; 2] = [140.0, 72.0];
/// Corner radius of a tile.
const RADIUS: u8 = 8;
/// Space between a tile's edge and its text.
const PADDING: f32 = SPACE[1];
/// Size of the icons on a tile.
const ICON_SIZE: f32 = 16.0;
/// Width of the outline of a tile that is playing, or has the keyboard focus.
const OUTLINE: f32 = 1.5;
/// Height of the empty state's button, for centering the block it sits in.
const EMPTY_BUTTON_HEIGHT: f32 = 32.0;
/// Least width of a tile's menu.
const MENU_WIDTH: f32 = 200.0;
/// Height of the volume bar in the menu, knob room included.
const VOLUME_BAR: f32 = 20.0;
/// How far an arrow key moves the volume bar.
const VOLUME_STEP: f32 = 0.05;
/// Id of the remove confirmation.
const DIALOG_ID: &str = "aria_sound_remove_dialog";

/// What the page remembers between frames.
#[derive(Clone, Default)]
struct State {
    /// The sound the open remove confirmation asks about.
    removing: Option<SoundId>,
}

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    // Taken out of egui's memory while it is used, as the Playlists page does: memory stays
    // locked while borrowed from there, and drawing must not hold that.
    let id = Id::unique("aria_soundboard");
    let mut state: State = ui.data_mut(|data| data.remove_temp(id)).unwrap_or_default();
    draw(ui, ctx, &mut state);
    ui.data_mut(|data| data.insert_temp(id, state));
}

fn draw(ui: &mut Ui, ctx: &mut PageCtx<'_>, state: &mut State) {
    let soundboard = ctx.soundboard;
    section_header(ui, &ctx.tr.tr(Route::Soundboard.label_key()));
    ui.add_space(SPACE[1]);

    // A sound removed since the confirmation opened has nothing left to confirm.
    if state
        .removing
        .is_some_and(|id| find(soundboard.sounds.as_slice(), id).is_none())
    {
        state.removing = None;
        *ctx.modal_open = false;
    }
    if soundboard.sounds.is_empty() {
        none_yet(ui, ctx);
        return;
    }

    ui.horizontal(|ui| {
        if ui.button(ctx.tr.tr("soundboard-add")).clicked() {
            ctx.out.push(PageAction::AddSounds);
        }
        if ui.button(ctx.tr.tr("soundboard-stop-all")).clicked() {
            ctx.out.push(PageAction::StopSounds);
        }
    });
    ui.add_space(SPACE[2]);

    eframe::egui::ScrollArea::vertical()
        .id_salt("aria_soundboard_tiles")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE[2], SPACE[2]);
                for sound in &soundboard.sounds {
                    tile(ui, ctx, sound, &mut state.removing);
                }
            });
        });

    if let Some(id) = state.removing
        && let Some(sound) = find(soundboard.sounds.as_slice(), id)
    {
        confirm_remove(ui, ctx, sound, &mut state.removing);
    }
}

fn find(sounds: &[SoundView], id: SoundId) -> Option<&SoundView> {
    sounds.iter().find(|sound| sound.id == id)
}

/// The empty state of a soundboard with no sounds, with the button that adds the first.
fn none_yet(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    let title = ctx.tr.tr("soundboard-empty");
    let hint = ctx.tr.tr("soundboard-empty-hint");
    let button = ctx.tr.tr("soundboard-add");
    empty_state_with(
        ui,
        icons::SPEAKER_HIGH,
        &title,
        &hint,
        &ctx.palette,
        EMPTY_BUTTON_HEIGHT,
        |ui| {
            let button = Button::new(&*button).min_size(vec2(0.0, EMPTY_BUTTON_HEIGHT));
            if ui.add(button).clicked() {
                ctx.out.push(PageAction::AddSounds);
            }
        },
    );
}

/// One tile: the name, the hotkey, and whether the sound is playing or its file is missing.
/// A missing sound is greyed but still clickable, since playing it is what explains why not.
fn tile(ui: &mut Ui, ctx: &mut PageCtx<'_>, sound: &SoundView, removing: &mut Option<SoundId>) {
    let (tr, p) = (ctx.tr, ctx.palette);
    let (rect, response) = ui.allocate_exact_size(vec2(TILE[0], TILE[1]), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &sound.name));
    let hotkey = sound.hotkey.as_ref().map(ToString::to_string);
    let inactive = sound
        .hotkey
        .is_some_and(|hotkey| ctx.inactive_hotkeys.contains(&hotkey));
    let description = describe(tr, sound, hotkey.as_deref(), inactive);
    let toggled = if sound.playing {
        Toggled::True
    } else {
        Toggled::False
    };
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_toggled(toggled);
        if !description.is_empty() {
            node.set_description(description.as_str());
        }
    });

    if ui.is_rect_visible(rect) {
        paint(ui, &p, rect, sound, hotkey.as_deref(), inactive, &response);
    }
    if response.clicked() {
        ctx.out.push(PageAction::PlaySound(sound.id));
    }
    let response = if inactive {
        response.on_hover_text(tr.tr("hotkey-inactive-hint"))
    } else {
        response
    };
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .context_menu(|ui| menu(ui, ctx, sound, hotkey.as_deref(), removing));
}

/// What assistive technology reads after a tile's name: its hotkey, whether that works, and
/// that its file is missing. Empty when there is nothing to add.
fn describe(tr: &Localizer, sound: &SoundView, hotkey: Option<&str>, inactive: bool) -> String {
    let mut parts = Vec::new();
    if let Some(hotkey) = hotkey {
        parts.push(hotkey.to_owned());
        if inactive {
            parts.push(tr.tr("hotkey-inactive-hint").into_owned());
        }
    }
    if sound.missing {
        parts.push(tr.tr("sound-missing").into_owned());
    }
    parts.join(". ")
}

/// Draws a tile into `rect`.
fn paint(
    ui: &Ui,
    p: &Palette,
    rect: eframe::egui::Rect,
    sound: &SoundView,
    hotkey: Option<&str>,
    inactive: bool,
    response: &eframe::egui::Response,
) {
    let painter = ui.painter();
    let fill = if response.hovered() {
        p.surface_raised
    } else {
        p.surface
    };
    let stroke = if response.has_focus() {
        Stroke::new(OUTLINE, p.focus)
    } else if sound.playing {
        Stroke::new(OUTLINE, p.accent)
    } else {
        Stroke::new(1.0, p.border)
    };
    painter.rect(rect, RADIUS, fill, stroke, StrokeKind::Inside);

    let text_color = if sound.missing { p.text_muted } else { p.text };
    let inner = rect.shrink(PADDING);
    // The icon is at the top right, so the name stops short of it.
    let name_width = inner.width() - ICON_SIZE - PADDING;
    let mut job = LayoutJob::simple(
        sound.name.clone(),
        semibold(TITLE_SIZE - 2.0),
        text_color,
        name_width,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(inner.left_top(), galley, text_color);

    let icon = if sound.missing {
        Some((icons::WARNING, p.danger))
    } else if sound.playing {
        Some((icons::SPEAKER_HIGH, p.accent))
    } else {
        None
    };
    if let Some((icon, color)) = icon {
        painter.text(
            inner.right_top(),
            Align2::RIGHT_TOP,
            icon,
            fonts::icon_font(ICON_SIZE),
            color,
        );
    }
    if let Some(hotkey) = hotkey {
        // A hotkey that does not work is struck through, in the color of a warning.
        let (color, strike) = if inactive {
            (p.danger, Stroke::new(1.0, p.danger))
        } else {
            (p.text_muted, Stroke::NONE)
        };
        let format = TextFormat {
            font_id: TextStyle::Small.resolve(ui.style()),
            color,
            strikethrough: strike,
            ..TextFormat::default()
        };
        let galley = painter.layout_job(LayoutJob::single_section(hotkey.to_owned(), format));
        let at = inner.left_bottom() - vec2(0.0, galley.size().y);
        painter.galley(at, galley, color);
    }
}

/// A tile's menu: rename, hotkey, volume and remove.
fn menu(
    ui: &mut Ui,
    ctx: &mut PageCtx<'_>,
    sound: &SoundView,
    hotkey: Option<&str>,
    removing: &mut Option<SoundId>,
) {
    let (tr, p) = (ctx.tr, ctx.palette);
    ui.set_min_width(MENU_WIDTH);
    if ui.button(tr.tr("sound-rename")).clicked() {
        ctx.out.push(PageAction::AskSoundName(sound.id));
    }
    if ui.button(tr.tr("sound-hotkey")).clicked() {
        ctx.out.push(PageAction::AskSoundHotkey(sound.id));
    }
    // The hotkey as it is now, under the button that changes it.
    let current = hotkey.map_or_else(|| tr.tr("sound-hotkey-none").into_owned(), str::to_owned);
    ui.label(eframe::egui::RichText::new(current).color(p.text_muted));
    ui.add_space(SPACE[0]);

    // The title and the percentage are painted: the bar below names itself, and reads its own
    // value, to assistive technology.
    let small = TextStyle::Small.resolve(ui.style());
    let width = ui.available_width();
    let (_, title) = ui.allocate_space(vec2(width, small.size));
    let (_, bar) = ui.allocate_space(vec2(width, VOLUME_BAR));
    let label = tr.tr("sound-volume");
    let moved = Scrub {
        id: "sound_volume",
        rect: bar,
        value: sound.volume,
        enabled: true,
        label: &label,
        step: VOLUME_STEP,
    }
    .show(ui, &p);
    if let Some(volume) = moved.committed {
        ctx.out.push(PageAction::SetSoundVolume {
            id: sound.id,
            volume,
        });
    }
    let shown = moved.dragging.or(moved.committed).unwrap_or(sound.volume);
    let painter = ui.painter();
    painter.text(
        title.left_top(),
        Align2::LEFT_TOP,
        &*label,
        small.clone(),
        p.text_muted,
    );
    painter.text(
        title.right_top(),
        Align2::RIGHT_TOP,
        percent(shown),
        small,
        p.text_muted,
    );
    ui.add_space(SPACE[0]);

    ui.separator();
    let remove = eframe::egui::RichText::new(tr.tr("sound-remove")).color(p.danger);
    if ui.button(remove).clicked() {
        *removing = Some(sound.id);
        *ctx.modal_open = true;
    }
}

/// `volume`, a fraction, as a whole percentage.
fn percent(volume: f32) -> String {
    format!("{}%", (volume.clamp(0.0, 1.0) * 100.0).round())
}

/// Asks before a sound is taken off the soundboard. Confirming makes the request.
fn confirm_remove(
    ui: &Ui,
    ctx: &mut PageCtx<'_>,
    sound: &SoundView,
    removing: &mut Option<SoundId>,
) {
    let tr = ctx.tr;
    let mut args = FluentArgs::new();
    args.set("name", sound.name.as_str());
    let text = Confirm {
        title: &tr.tr("sound-remove"),
        body: &tr.tr_args("sound-remove-confirm", &args),
        confirm: &tr.tr("settings-confirm"),
        cancel: &tr.tr("settings-cancel"),
    };
    let answer = confirm_dialog(ui, &ctx.palette, Id::unique(DIALOG_ID), &text, |_| {});
    if answer.is_some() {
        *removing = None;
        *ctx.modal_open = false;
    }
    if answer == Some(true) {
        ctx.out.push(PageAction::RemoveSound(sound.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_reads_as_a_whole_percentage() {
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.456), "46%");
        assert_eq!(percent(7.0), "100%", "out of range is held to the bar");
    }
}
