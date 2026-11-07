//! Toasts: notices stacked in the bottom-right corner of the window, newest at the bottom.
//!
//! Info and warning toasts go away on their own after [`TOAST_SECONDS`]; errors stay until
//! dismissed. A repaint is scheduled only while a toast is waiting to expire, so an idle
//! window never repaints on their account.

use std::time::Duration;

use aria_core::{
    i18n::Localizer,
    notice::{Notice, NoticeLevel},
};
use eframe::egui::{
    Align2, Area, Color32, Context, FontId, Frame, Id, Label, Margin, Order, RichText, Sense,
    Stroke, TextStyle, Ui, UiBuilder, accesskit::Role, pos2, vec2,
};

use super::IconButton;
use crate::ui::theme::{Palette, fonts::icon_font, icons, visuals::SPACE};

/// Seconds an info or warning toast stays on screen.
pub const TOAST_SECONDS: f64 = 6.0;

/// Id of the area holding the toasts.
const AREA_ID: &str = "aria_toasts";
/// Outer width of a toast.
const WIDTH: f32 = 360.0;
/// Size of the level icon.
const ICON_SIZE: f32 = 18.0;
/// Edge length of the dismiss button, and its icon size.
const DISMISS: [f32; 2] = [24.0, 14.0];
/// Corner radius of a toast.
const RADIUS: u8 = 8;

/// One notice on screen.
#[derive(Debug)]
struct Toast {
    /// Tells this toast's widgets apart from the others', frame after frame.
    id: u64,
    notice: Notice,
    /// When it goes away on its own, in seconds of egui input time. Errors have none.
    expires_at: Option<f64>,
}

/// The notices on screen, oldest first.
#[derive(Debug, Default)]
pub struct Toasts {
    toasts: Vec<Toast>,
    next_id: u64,
    /// Room left free below the stack.
    bottom_inset: f32,
}

impl Toasts {
    /// No toasts yet. They stack `bottom_inset` points above the bottom of the window, clear
    /// of what is drawn there.
    pub fn new(bottom_inset: f32) -> Self {
        Self {
            bottom_inset,
            ..Self::default()
        }
    }

    /// Shows `notice` from `now` on, in seconds of egui input time. The same notice already
    /// on screen is replaced, so a failure that repeats shows once, with a fresh timer.
    pub fn push(&mut self, notice: Notice, now: f64) {
        self.toasts.retain(|toast| toast.notice != notice);
        let expires_at = match notice.level {
            NoticeLevel::Info | NoticeLevel::Warning => Some(now + TOAST_SECONDS),
            NoticeLevel::Error => None,
        };
        self.toasts.push(Toast {
            id: self.next_id,
            notice,
            expires_at,
        });
        self.next_id += 1;
    }

    /// Removes the toasts whose time is up at `now`.
    pub fn expire(&mut self, now: f64) {
        self.toasts
            .retain(|toast| toast.expires_at.is_none_or(|at| at > now));
    }

    /// The number of toasts on screen.
    pub fn len(&self) -> usize {
        self.toasts.len()
    }

    /// Whether no toast is on screen.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// The notices on screen, oldest first.
    #[cfg(test)]
    pub(crate) fn notices(&self) -> impl Iterator<Item = &Notice> {
        self.toasts.iter().map(|toast| &toast.notice)
    }

    /// When the next toast goes away on its own, if one will.
    fn next_expiry(&self) -> Option<f64> {
        self.toasts
            .iter()
            .filter_map(|toast| toast.expires_at)
            .min_by(f64::total_cmp)
    }

    /// Removes the toasts whose time is up, then draws the rest. While one is waiting to
    /// expire, a repaint is scheduled for that moment.
    pub fn show(&mut self, ctx: &Context, tr: &Localizer, p: &Palette) {
        let now = ctx.input(|i| i.time);
        self.expire(now);
        if self.toasts.is_empty() {
            return;
        }
        if let Some(delay) = self
            .next_expiry()
            .and_then(|at| Duration::try_from_secs_f64(at - now).ok())
        {
            ctx.request_repaint_after(delay);
        }

        let style = ToastStyle::new(ctx, p);
        let texts: Vec<_> = self
            .toasts
            .iter()
            .map(|toast| tr.tr_args(toast.notice.key, &toast.notice.fluent_args()))
            .collect();
        // An area anchored at a corner is placed by the size it had last frame, which is stale
        // as soon as a toast comes or goes. So measure the stack and place its top-left corner.
        let height = texts
            .iter()
            .map(|text| style.height(ctx, text))
            .sum::<f32>()
            + SPACE[1] * (texts.len() - 1) as f32;
        let screen = ctx.content_rect();
        let top_left = pos2(
            screen.right() - SPACE[3] - WIDTH,
            screen.bottom() - self.bottom_inset - SPACE[3] - height,
        );

        let mut dismissed = None;
        Area::new(Id::unique(AREA_ID))
            .order(Order::Foreground)
            .fixed_pos(top_left)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = SPACE[1];
                // Oldest first, so the newest is at the bottom.
                for (toast, text) in self.toasts.iter().zip(&texts) {
                    if toast_ui(ui, &style, toast, text, tr, p) {
                        dismissed = Some(toast.id);
                    }
                }
            });
        if let Some(id) = dismissed {
            self.toasts.retain(|toast| toast.id != id);
            ctx.request_repaint();
        }
    }
}

/// How a toast is laid out, shared by measuring and drawing so the two agree.
struct ToastStyle {
    frame: Frame,
    font: FontId,
    text_color: Color32,
    /// Width of the frame's contents.
    inner_width: f32,
    /// Width the text wraps at.
    text_width: f32,
    /// Space above the text that centers its first line on the dismiss button.
    text_top: f32,
}

impl ToastStyle {
    fn new(ctx: &Context, p: &Palette) -> Self {
        let style = ctx.global_style();
        let frame = Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(RADIUS)
            .inner_margin(Margin::same(SPACE[2] as i8))
            .shadow(style.visuals.popup_shadow);
        let font = TextStyle::Body.resolve(&style);
        let line_height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        let inner_width = WIDTH - frame.total_margin().sum().x;
        Self {
            frame,
            font,
            text_color: p.text,
            inner_width,
            // The icon, the text and the dismiss button, with a gap between each.
            text_width: inner_width - ICON_SIZE - DISMISS[0] - 2.0 * SPACE[1],
            text_top: ((DISMISS[0] - line_height) / 2.0).max(0.0),
        }
    }

    /// The outer height of a toast showing `text`.
    fn height(&self, ctx: &Context, text: &str) -> f32 {
        let text_height = ctx.fonts_mut(|fonts| {
            fonts
                .layout(
                    text.to_owned(),
                    self.font.clone(),
                    self.text_color,
                    self.text_width,
                )
                .size()
                .y
        });
        let content = DISMISS[0].max(self.text_top + text_height);
        content + self.frame.total_margin().sum().y
    }
}

/// Draws one toast: the level icon, `text` and a dismiss button. Returns whether the button
/// was clicked.
fn toast_ui(
    ui: &mut Ui,
    style: &ToastStyle,
    toast: &Toast,
    text: &str,
    tr: &Localizer,
    p: &Palette,
) -> bool {
    let (icon, icon_color, role) = match toast.notice.level {
        NoticeLevel::Info => (icons::INFO, p.accent, Role::Status),
        NoticeLevel::Warning => (icons::WARNING, p.text_muted, Role::Status),
        NoticeLevel::Error => (icons::WARNING, p.danger, Role::Alert),
    };
    let scope = UiBuilder::new().id_salt(toast.id).accessibility_role(role);
    ui.scope_builder(scope, |ui| {
        style
            .frame
            .show(ui, |ui| {
                ui.set_width(style.inner_width);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE[1];
                    let [button, button_icon] = DISMISS;
                    // The icon and the button share a band as tall as the button, and the
                    // first line of text is centered in it too.
                    let (icon_rect, _) =
                        ui.allocate_exact_size(vec2(ICON_SIZE, button), Sense::hover());
                    ui.painter().text(
                        icon_rect.center(),
                        Align2::CENTER_CENTER,
                        icon,
                        icon_font(ICON_SIZE),
                        icon_color,
                    );
                    ui.allocate_ui(vec2(style.text_width, 0.0), |ui| {
                        ui.add_space(style.text_top);
                        let text = RichText::new(text)
                            .font(style.font.clone())
                            .color(style.text_color);
                        ui.add(Label::new(text).selectable(false).wrap());
                    });
                    let dismiss = tr.tr("notice-dismiss");
                    let response =
                        ui.add(IconButton::new(icons::X, &dismiss, p).size(button, button_icon));
                    // Several toasts can be up at once: say which one this button dismisses.
                    ui.ctx()
                        .accesskit_node_builder(response.id, |node| node.set_description(text));
                    response.clicked()
                })
                .inner
            })
            .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: Notice = Notice {
        level: NoticeLevel::Info,
        key: "notice-a",
        args: Vec::new(),
    };
    const WARNING: Notice = Notice {
        level: NoticeLevel::Warning,
        key: "notice-b",
        args: Vec::new(),
    };
    const ERROR: Notice = Notice {
        level: NoticeLevel::Error,
        key: "notice-c",
        args: Vec::new(),
    };

    #[test]
    fn info_and_warning_expire_after_six_seconds() {
        let mut toasts = Toasts::default();
        toasts.push(INFO, 1.0);
        toasts.push(WARNING, 2.0);
        toasts.expire(7.0);
        assert_eq!(toasts.notices().collect::<Vec<_>>(), [&WARNING]);
        toasts.expire(8.0);
        assert!(toasts.is_empty());
    }

    #[test]
    fn errors_never_expire() {
        let mut toasts = Toasts::default();
        toasts.push(ERROR, 0.0);
        toasts.expire(1e9);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts.next_expiry(), None);
    }

    #[test]
    fn next_expiry_is_the_soonest() {
        let mut toasts = Toasts::default();
        toasts.push(WARNING, 3.0);
        toasts.push(ERROR, 0.0);
        toasts.push(INFO, 1.0);
        assert_eq!(toasts.next_expiry(), Some(1.0 + TOAST_SECONDS));
    }

    #[test]
    fn a_repeated_notice_shows_once_with_a_fresh_timer() {
        let mut toasts = Toasts::default();
        toasts.push(WARNING, 0.0);
        toasts.push(INFO, 1.0);
        toasts.push(WARNING, 5.0);
        // Newest last, and only one warning.
        assert_eq!(toasts.notices().collect::<Vec<_>>(), [&INFO, &WARNING]);
        toasts.expire(10.0);
        assert_eq!(toasts.notices().collect::<Vec<_>>(), [&WARNING]);
    }
}
