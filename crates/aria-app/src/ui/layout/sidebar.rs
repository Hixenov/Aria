//! The left sidebar: wordmark, history buttons and the navigation list.

use aria_core::{
    i18n::Localizer,
    settings::{SIDEBAR_MAX, SIDEBAR_MIN},
};
use eframe::egui::{
    Align, FontFamily, FontId, Label, Layout, Margin, Panel, RichText, ScrollArea, Separator, Ui,
    UiBuilder, accesskit::Role, vec2,
};

use super::surface_frame;
use crate::{
    APP_NAME,
    nav::{History, Route},
    ui::{
        theme::{Palette, fonts::SEMIBOLD, icons, visuals::SPACE},
        widgets::{IconButton, NAV_ROW_HEIGHT, nav_item},
    },
};

/// Panel id, unique among the root panels.
const ID: &str = "aria_sidebar";
/// Height of the wordmark row.
const HEADER_HEIGHT: f32 = 32.0;
/// Text size of the wordmark.
const WORDMARK_SIZE: f32 = 18.0;
/// Edge length of the history buttons, and their icon size.
const HISTORY_BUTTON: [f32; 2] = [28.0, 16.0];

/// Navigation groups, top to bottom, with a separator between them. [`PINNED`] sits at the
/// bottom. Together they follow [`Route::ALL`], so Ctrl+1…9 match the order on screen.
const GROUPS: [&[Route]; 2] = [
    &[Route::Home, Route::Search],
    &[
        Route::Library,
        Route::Liked,
        Route::Playlists,
        Route::Albums,
        Route::Artists,
        Route::LocalMusic,
    ],
];
/// The entry pinned to the bottom of the sidebar.
const PINNED: Route = Route::Settings;

/// What the user asked of the sidebar this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarAction {
    /// Show this route.
    Navigate(Route),
    /// Step back in history.
    Back,
    /// Step forward in history.
    Forward,
}

/// The result of showing the sidebar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarOutput {
    /// What the user clicked, if anything.
    pub action: Option<SidebarAction>,
    /// The panel's outer width this frame.
    pub width: f32,
}

/// Shows the resizable sidebar. It starts at `width`; with `force_width` it takes `width`
/// this frame even if egui remembers another one.
pub fn show(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    history: &History,
    width: f32,
    force_width: bool,
) -> SidebarOutput {
    let margin = Margin {
        left: SPACE[1] as i8,
        right: SPACE[1] as i8,
        top: SPACE[2] as i8,
        bottom: SPACE[1] as i8,
    };
    let mut panel = Panel::left(ID)
        .resizable(true)
        .size_range(SIDEBAR_MIN..=SIDEBAR_MAX)
        .default_size(width)
        .frame(surface_frame(p, margin));
    if force_width {
        panel = panel.exact_size(width);
    }

    // egui draws the hovered and dragged resize edge with widget strokes; make those accent
    // colored for the edge only, and give the contents the normal style back.
    let base = ui.style().clone();
    let widgets = &mut ui.style_mut().visuals.widgets;
    widgets.hovered.fg_stroke.color = p.accent_hover;
    widgets.active.fg_stroke.color = p.accent;
    let response = panel.show(ui, |ui| {
        ui.set_style(base.clone());
        contents(ui, tr, p, history)
    });
    ui.set_style(base);

    SidebarOutput {
        action: response.inner,
        width: response.response.rect.width(),
    }
}

fn contents(ui: &mut Ui, tr: &Localizer, p: &Palette, history: &History) -> Option<SidebarAction> {
    let mut action = header(ui, tr, p, history);
    ui.add_space(SPACE[2]);
    if let Some(route) = nav_list(ui, tr, p, history.current()) {
        action = Some(SidebarAction::Navigate(route));
    }
    action
}

/// The wordmark, with the back and forward buttons on the right.
fn header(ui: &mut Ui, tr: &Localizer, p: &Palette, history: &History) -> Option<SidebarAction> {
    let mut action = None;
    let size = vec2(ui.available_width(), HEADER_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = SPACE[0];
        let [button, icon] = HISTORY_BUTTON;
        let forward = tr.tr("nav-forward");
        let forward_button = IconButton::new(icons::CARET_RIGHT, &forward, p).size(button, icon);
        if ui
            .add_enabled(history.can_forward(), forward_button)
            .clicked()
        {
            action = Some(SidebarAction::Forward);
        }
        let back = tr.tr("nav-back");
        let back_button = IconButton::new(icons::CARET_LEFT, &back, p).size(button, icon);
        if ui.add_enabled(history.can_back(), back_button).clicked() {
            action = Some(SidebarAction::Back);
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(SPACE[2]);
            let font = FontId::new(WORDMARK_SIZE, FontFamily::Name(SEMIBOLD.into()));
            let wordmark = RichText::new(APP_NAME).font(font).color(p.text);
            ui.add(Label::new(wordmark).selectable(false).truncate());
        });
    });
    action
}

/// The navigation entries; returns the one clicked.
fn nav_list(ui: &mut Ui, tr: &Localizer, p: &Palette, current: Route) -> Option<Route> {
    let mut clicked = None;
    let mut entry = |ui: &mut Ui, route: Route| {
        let label = tr.tr(route.label_key());
        if nav_item(ui, route.icon(), &label, route == current, p).clicked() {
            clicked = Some(route);
        }
    };
    ui.scope_builder(
        UiBuilder::new().accessibility_role(Role::Navigation),
        |ui| {
            ui.spacing_mut().item_spacing.y = SPACE[0] / 2.0;
            // Scrolls only when zoomed in far enough that the list no longer fits.
            let list_height = (ui.available_height() - NAV_ROW_HEIGHT - SPACE[1]).max(0.0);
            ScrollArea::vertical()
                .id_salt("aria_nav_list")
                .max_height(list_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, group) in GROUPS.iter().enumerate() {
                        if i > 0 {
                            ui.add(Separator::default().spacing(SPACE[2]));
                        }
                        for &route in *group {
                            entry(ui, route);
                        }
                    }
                });
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| entry(ui, PINNED));
        },
    );
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_order_follows_route_order() {
        let shown: Vec<Route> = GROUPS
            .iter()
            .flat_map(|group| group.iter().copied())
            .chain([PINNED])
            .collect();
        assert_eq!(shown, Route::ALL);
    }
}
