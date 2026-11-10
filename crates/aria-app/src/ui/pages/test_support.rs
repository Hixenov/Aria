//! What page tests share: a world to draw a page in, and a harness that does it.

use aria_core::{
    i18n::{FluentArgs, Localizer},
    settings::{Language, Settings},
};
use aria_library::Collections;
use eframe::egui::{Theme, Ui};
use egui_kittest::Harness;

use super::{LibraryView, PageAction, PageCtx};
use crate::ui::theme::{Palette, fonts};

/// Everything a page reads, and what it asked for.
pub struct World {
    pub collections: Collections,
    pub library: LibraryView,
    /// The requests the page made, in order.
    pub out: Vec<PageAction>,
    pub modal_open: bool,
    tr: Localizer,
    settings: Settings,
}

impl World {
    pub fn new(collections: Collections, library: LibraryView) -> Self {
        Self {
            collections,
            library,
            out: Vec::new(),
            modal_open: false,
            tr: Localizer::new(Language::En).unwrap(),
            settings: Settings::default(),
        }
    }

    /// Runs `draw` with a [`PageCtx`] over this world.
    pub fn page<R>(&mut self, ui: &mut Ui, draw: impl FnOnce(&mut Ui, &mut PageCtx<'_>) -> R) -> R {
        let mut ctx = PageCtx {
            tr: &self.tr,
            palette: Palette::for_theme(Theme::Dark, &self.settings),
            settings: &self.settings,
            paths_available: false,
            modal_open: &mut self.modal_open,
            out: &mut self.out,
            library: &self.library,
            collections: &self.collections,
            devices: &[],
            artwork: None,
        };
        draw(ui, &mut ctx)
    }

    /// The English text of `key`.
    pub fn english(&self, key: &str) -> String {
        self.tr.tr(key).into_owned()
    }

    /// The English text of `key` with `args`.
    pub fn english_with(&self, key: &str, args: &FluentArgs) -> String {
        self.tr.tr_args(key, args).into_owned()
    }
}

/// How long a frame lasts in the harness, in seconds. Each queued event gets a frame of its
/// own, so a click made of four events must fit in egui's double-click delay.
const STEP_DT: f32 = 0.05;

/// A harness that draws with `draw` each frame, with the fonts installed and room for a page.
pub fn harness<S: 'static>(
    state: S,
    draw: impl FnMut(&mut Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(1000.0, 600.0))
        .with_step_dt(STEP_DT)
        .with_context_setup(fonts::install)
        .build_ui_state(draw, state);
    harness.run();
    harness
}
