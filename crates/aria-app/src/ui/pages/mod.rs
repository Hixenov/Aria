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
    DeviceInfo,
    i18n::{FluentArgs, Localizer},
    settings::Settings,
};
use aria_library::{FolderId, TrackId};
use eframe::egui::Ui;

pub use self::local_music::{LibraryView, Scope, SortKey, Tab, TrackView};

use crate::{
    artwork::ArtworkTextures,
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
    /// The library as the backend last reported it.
    pub library: &'a LibraryView,
    /// The output devices as the backend last listed them. Empty until it answers.
    pub devices: &'a [DeviceInfo],
    /// Cover textures, when the thumbnails' folder is known. Without it, covers show as
    /// placeholders.
    pub artwork: Option<&'a mut ArtworkTextures>,
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
    /// Make `ids`, in this order, the list being played, and start the one at `start`.
    PlayContext {
        /// The tracks, as the page shows them.
        ids: Vec<TrackId>,
        /// The index in `ids` of the track to start.
        start: usize,
    },
    /// Play a track right after the current one.
    PlayNext(TrackId),
    /// Put a track at the end of the queue.
    AddToQueue(TrackId),
    /// Ask the user for a folder to add to the library.
    PickFolder,
    /// Take a folder out of the library. Its files are left alone.
    RemoveFolder(FolderId),
    /// Scan the library's folders again.
    Rescan,
    /// Delete the saved artwork thumbnails.
    ClearArtwork,
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
/// which phase. The heading already names the page, so the empty state does not repeat it.
fn pending(ui: &mut Ui, ctx: &PageCtx<'_>, route: Route) {
    section_header(ui, &ctx.tr.tr(route.label_key()));
    let Some(phase) = route.phase() else {
        return;
    };
    let mut args = FluentArgs::new();
    args.set("phase", phase);
    let text = ctx.tr.tr_args("empty-phase-pending", &args);
    empty_state(ui, route.icon(), &text, "", &ctx.palette);
}
