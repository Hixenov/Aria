//! The page shown in the central panel, one per [`Route`].

mod albums;
mod artists;
mod home;
mod library;
mod liked;
mod local_music;
mod playlists;
mod search;
mod settings;

use aria_core::{
    i18n::{FluentArgs, Localizer},
    settings::Settings,
};
use eframe::egui::Ui;

use crate::{
    nav::Route,
    ui::{
        theme::Palette,
        widgets::{empty_state, section_header},
    },
};

/// What a page reads, and where it leaves requests for the app.
pub struct PageCtx<'a> {
    /// Localized text.
    pub tr: &'a Localizer,
    /// Colors for the active theme.
    pub palette: Palette,
    /// The current settings.
    pub settings: &'a Settings,
    /// Whether Aria's directories are known, so they can be opened.
    pub paths_available: bool,
    /// Whether the page has its modal dialog open. The app ignores navigation and panel
    /// shortcuts while it is set.
    pub modal_open: &'a mut bool,
    /// Requests for the app, handled after the page is drawn.
    pub out: &'a mut Vec<PageAction>,
}

/// A request from a page to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum PageAction {
    /// Apply and save these settings.
    UpdateSettings(Settings),
    /// Restore the default settings.
    ResetSettings,
    /// Open the settings folder in the file manager.
    OpenConfigDir,
    /// Open the log folder in the file manager.
    OpenLogDir,
}

/// Draws the page for `route`.
pub fn show(route: Route, ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    match route {
        Route::Home => home::show(ui, ctx),
        Route::Search => search::show(ui, ctx),
        Route::Library => library::show(ui, ctx),
        Route::Liked => liked::show(ui, ctx),
        Route::Playlists => playlists::show(ui, ctx),
        Route::Albums => albums::show(ui, ctx),
        Route::Artists => artists::show(ui, ctx),
        Route::LocalMusic => local_music::show(ui, ctx),
        Route::Settings => settings::show(ui, ctx),
    }
}

/// A page whose content comes in a later phase: its heading, then an empty state saying
/// which phase.
fn pending(ui: &mut Ui, ctx: &PageCtx<'_>, route: Route) {
    let title = ctx.tr.tr(route.label_key());
    section_header(ui, &title);
    let body = match route.phase() {
        Some(phase) => {
            let mut args = FluentArgs::new();
            args.set("phase", phase);
            ctx.tr.tr_args("empty-phase-pending", &args).into_owned()
        }
        None => String::new(),
    };
    empty_state(ui, route.icon(), &title, &body, &ctx.palette);
}
