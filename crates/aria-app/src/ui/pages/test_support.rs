//! What page tests share: a world to draw a page in, and a harness that does it.

use aria_core::{
    DeviceInfo,
    i18n::{FluentArgs, Localizer},
    settings::{Language, Settings},
    soundboard::SoundboardState,
};
use aria_library::Collections;
use eframe::egui::{Theme, Ui};
use egui_kittest::Harness;

use super::{LibraryView, PageAction, PageCtx};
use crate::spotify::{RemoteCovers, SpotifyUi};
use crate::ui::theme::{Palette, fonts};

/// Everything a page reads, and what it asked for.
pub struct World {
    pub collections: Collections,
    pub library: LibraryView,
    pub soundboard: SoundboardState,
    /// The requests the page made, in order.
    pub out: Vec<PageAction>,
    pub modal_open: bool,
    /// The output devices and the microphones the page is shown.
    pub devices: Vec<DeviceInfo>,
    pub input_devices: Vec<DeviceInfo>,
    pub settings: Settings,
    pub spotify: SpotifyUi,
    pub covers: RemoteCovers,
    tr: Localizer,
}

impl World {
    pub fn new(collections: Collections, library: LibraryView) -> Self {
        Self {
            collections,
            library,
            soundboard: SoundboardState::default(),
            out: Vec::new(),
            modal_open: false,
            devices: Vec::new(),
            input_devices: Vec::new(),
            settings: Settings::default(),
            spotify: SpotifyUi::default(),
            covers: RemoteCovers::default(),
            tr: Localizer::new(Language::En).expect("the English locale loads"),
        }
    }

    /// Runs `draw` with a [`PageCtx`] over this world.
    pub fn page<R>(&mut self, ui: &mut Ui, draw: impl FnOnce(&mut Ui, &mut PageCtx<'_>) -> R) -> R {
        let mut ctx = PageCtx {
            tr: &self.tr,
            palette: Palette::for_theme(Theme::Dark, &self.settings),
            settings: &self.settings,
            paths_available: false,
            exclusive_supported: true,
            modal_open: &mut self.modal_open,
            out: &mut self.out,
            library: &self.library,
            collections: &self.collections,
            soundboard: &self.soundboard,
            inactive_hotkeys: &[],
            devices: &self.devices,
            input_devices: &self.input_devices,
            artwork: None,
            spotify: &self.spotify,
            covers: &mut self.covers,
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
    harness_sized(eframe::egui::vec2(1000.0, 600.0), state, draw)
}

/// [`harness`], with a window of `size`.
pub fn harness_sized<S: 'static>(
    size: eframe::egui::Vec2,
    state: S,
    draw: impl FnMut(&mut Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    let mut harness = Harness::builder()
        .with_size(size)
        .with_step_dt(STEP_DT)
        .with_context_setup(fonts::install)
        .build_ui_state(draw, state);
    harness.run();
    harness
}
