//! A thin horizontal bar to drag: the seek bar and the volume bar.

use eframe::egui::{EventFilter, Key, Pos2, Rect, Sense, Ui, WidgetInfo, pos2};

use crate::ui::theme::Palette;

/// Thickness of the track.
const TRACK: f32 = 4.0;
/// Radius of the knob; the track is inset by it so both ends can be reached.
const KNOB: f32 = 5.0;

/// A thin horizontal bar to drag. Add it with [`Scrub::show`].
pub struct Scrub<'a> {
    /// Names the bar among others in the same `Ui`.
    pub id: &'static str,
    /// Where it sits, knob room included.
    pub rect: Rect,
    /// How far along it is, `0.0..=1.0`.
    pub value: f32,
    pub enabled: bool,
    /// Names it for assistive technology.
    pub label: &'a str,
    /// How far an arrow key moves it.
    pub step: f32,
}

/// What the user did to a [`Scrub`] this frame, as fractions of its length.
#[derive(Default)]
pub struct Moved {
    /// Where the pointer is, while it drags the knob.
    pub dragging: Option<f32>,
    /// Where the user let go, clicked, or moved it with the keyboard.
    pub committed: Option<f32>,
}

impl Scrub<'_> {
    pub fn show(self, ui: &mut Ui, p: &Palette) -> Moved {
        let Self {
            id,
            rect,
            value,
            enabled,
            label,
            step,
        } = self;
        let id = ui.scope_id().with(id);
        let sense = if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, id, sense);
        // Assistive technology reads the value as a percentage.
        let percent = f64::from(value) * 100.0;
        response.widget_info(|| WidgetInfo::slider(enabled, percent, label));

        let line = Rect::from_min_max(
            pos2(rect.left() + KNOB, rect.center().y - TRACK / 2.0),
            pos2(rect.right() - KNOB, rect.center().y + TRACK / 2.0),
        );
        let at = |pos: Pos2| ((pos.x - line.left()) / line.width().max(1.0)).clamp(0.0, 1.0);
        let mut moved = Moved::default();
        if enabled {
            let pointer = response.interact_pointer_pos().map(at);
            if response.dragged() {
                moved.dragging = pointer;
            }
            if response.drag_stopped() || response.clicked() {
                moved.committed = pointer;
            }
            if response.has_focus() {
                // Horizontal arrows are this bar's, not the focus's: held, they must not walk
                // on to the next widget.
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        id,
                        EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..EventFilter::default()
                        },
                    );
                });
                let key = |key| ui.input(|input| input.key_pressed(key));
                if key(Key::ArrowRight) || key(Key::ArrowUp) {
                    moved.committed = Some((value + step).min(1.0));
                } else if key(Key::ArrowLeft) || key(Key::ArrowDown) {
                    moved.committed = Some((value - step).max(0.0));
                }
            }
        }

        if ui.is_rect_visible(rect) {
            let shown = moved.dragging.unwrap_or(value);
            let filled = if enabled { p.accent } else { p.text_muted };
            let painter = ui.painter();
            painter.rect_filled(line, TRACK / 2.0, p.surface_raised);
            let done = Rect::from_min_max(
                line.min,
                pos2(line.left() + line.width() * shown, line.bottom()),
            );
            painter.rect_filled(done, TRACK / 2.0, filled);
            if enabled && (response.hovered() || response.dragged() || response.has_focus()) {
                let knob = pos2(done.right(), line.center().y);
                painter.circle_filled(knob, KNOB, p.text);
                if response.has_focus() {
                    painter.circle_stroke(knob, KNOB + 1.5, (1.5, p.focus));
                }
            }
        }
        if enabled {
            response.on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
        }
        moved
    }
}
