//! Application state and the per-frame shell: player bar, sidebar, right panel and page.

use std::{
    io, mem,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use aria_core::{
    PlaybackClock,
    command::Command,
    event::Event,
    i18n::{Localizer, os_language},
    logging::LogLevelHandle,
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    playback_service::PlayerView,
    player::queue::RepeatMode,
    runtime::BackendHandle,
    settings::{SIDEBAR_MAX, SIDEBAR_MIN, Settings},
};
use eframe::egui::{self, CentralPanel, Frame, Margin, UiBuilder, accesskit::Role};

use crate::{
    artwork::ArtworkTextures,
    folders::{FolderOpener, FolderPicker},
    nav::{History, Route},
    shortcuts::read_shortcuts,
    ui::{
        layout::{
            player_bar::{self, NowPlaying, PlayerAction, PlayerBar},
            right_panel,
            sidebar::{self, SidebarAction},
        },
        pages::{self, LibraryView, PageAction, PageCtx},
        theme::{
            Palette, fonts,
            visuals::{self, SPACE},
        },
        widgets::Toasts,
    },
    whole_ms,
};

/// How long [`eframe::App::on_exit`] waits for the backend to save and stop.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long, in seconds, the app redraws often after a command that moves the position.
const SETTLE_FOR: f64 = 1.0;
/// How often it redraws then.
const SETTLE_REPAINT: Duration = Duration::from_millis(50);
/// Shown when a folder could not be opened in the file manager.
const OPEN_FOLDER_FAILED: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "notice-open-folder-failed",
    args: Vec::new(),
};

/// Shown when the folder dialog could not be opened.
const PICK_FOLDER_FAILED: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "notice-pick-folder-failed",
    args: Vec::new(),
};

/// The handles the shell drives. Each is optional, so tests, and a start-up that lost one,
/// still run.
#[derive(Default)]
pub struct AppDeps {
    /// The backend thread; without it nothing is saved.
    pub backend: Option<BackendHandle>,
    /// Changes the log level live. `main` keeps the logging guard itself, so the log file
    /// stays open until after the window has closed.
    pub logging: Option<LogLevelHandle>,
    /// Aria's directories, when they could be resolved.
    pub paths: Option<AppPaths>,
    /// Where playback is. Taken from `backend` when left out.
    pub clock: Option<Arc<PlaybackClock>>,
}

impl AppDeps {
    /// No backend, logging or paths, for tests.
    pub fn none() -> Self {
        Self::default()
    }
}

/// The application: settings, localization, navigation and the handles it drives.
pub struct AriaApp {
    settings: Settings,
    localizer: Localizer,
    history: History,
    deps: AppDeps,
    /// Startup notices, until the first frame shows them as toasts.
    pending_notices: Vec<Notice>,
    /// Fonts go in on the first frame, once there is a context.
    fonts_installed: bool,
    /// Visuals must be applied before the next frame draws anything.
    style_dirty: bool,
    /// The sidebar must take `settings.sidebar_width` next frame, overriding the width egui
    /// remembers.
    sidebar_width_dirty: bool,
    /// Last frame, the window was narrower than the saved sidebar width.
    sidebar_squeezed: bool,
    /// Notices on screen.
    toasts: Toasts,
    /// Folders being opened in the file manager.
    folders: FolderOpener,
    /// The dialog for choosing a folder to add to the library.
    picker: FolderPicker,
    /// What the backend last reported of the library and the player. Starts empty, since a
    /// disabled library sends nothing at all.
    library: LibraryView,
    /// What the backend last reported of the player.
    player: PlayerView,
    /// Where playback is, read each frame by the player bar.
    clock: Option<Arc<PlaybackClock>>,
    /// Cover textures. Made on the first frame, once there is a context and a cache folder.
    artwork: Option<ArtworkTextures>,
    /// Until when, in egui's time, the app redraws often so that a seek or a skip shows its
    /// new position. The clock changes without any event to say so.
    settle_until: f64,
    /// Whether making `artwork` has been tried.
    artwork_started: bool,
    /// Whether the Settings page's reset dialog is open. Navigation and panel shortcuts
    /// wait while it is.
    modal_open: bool,
    /// Resets sent to the backend that it has not answered yet. Each answer brings the
    /// defaults, which are applied already, and changes made since are newer: it is ignored.
    outstanding_resets: u32,
    /// When `main` began, until the first frame logs how long it took to arrive.
    startup: Option<Instant>,
}

impl AriaApp {
    /// Creates the app. Nothing touches egui until the first frame, which installs the
    /// fonts and visuals.
    ///
    /// `localizer` should already speak `settings.language`. `startup_notices` wait in
    /// [`Self::pending_notices`] to be shown.
    pub fn new(
        settings: Settings,
        localizer: Localizer,
        deps: AppDeps,
        startup_notices: Vec<Notice>,
    ) -> AriaApp {
        let clock = deps
            .clock
            .clone()
            .or_else(|| deps.backend.as_ref().map(BackendHandle::clock));
        AriaApp {
            settings: settings.sanitized(),
            localizer,
            history: History::new(Route::Home),
            deps,
            pending_notices: startup_notices,
            fonts_installed: false,
            style_dirty: true,
            sidebar_width_dirty: true,
            sidebar_squeezed: false,
            toasts: Toasts::new(player_bar::HEIGHT),
            folders: FolderOpener::default(),
            picker: FolderPicker::default(),
            library: LibraryView::default(),
            player: PlayerView::default(),
            clock,
            artwork: None,
            artwork_started: false,
            settle_until: 0.0,
            modal_open: false,
            outstanding_resets: 0,
            startup: None,
        }
    }

    /// Has the first frame log `startup stage=first_frame ms=<n>`, measured from `started`,
    /// the instant `main` began.
    /// Without it, as in tests, nothing is logged.
    pub fn with_startup_instant(mut self, started: Instant) -> AriaApp {
        self.startup = Some(started);
        self
    }

    /// The route on screen.
    pub fn current_route(&self) -> Route {
        self.history.current()
    }

    /// The settings in effect.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Startup notices not shown yet, oldest first. The first frame shows them as toasts.
    pub fn pending_notices(&self) -> &[Notice] {
        &self.pending_notices
    }

    /// Draws one whole frame into the root `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(started) = self.startup.take() {
            let ms = whole_ms(started.elapsed());
            tracing::info!(stage = %"first_frame", ms, "startup");
        }
        let ctx = ui.ctx().clone();
        // Events first, so settings the backend replaced are styled in this very frame.
        self.drain_events(&ctx);
        self.prepare_style(ui);
        for notice in mem::take(&mut self.pending_notices) {
            self.notify(&ctx, notice);
        }
        self.collect_folder_failures(&ctx);
        self.collect_picked_folder();
        self.handle_shortcuts(&ctx);

        self.poll_artwork(&ctx);

        let palette = Palette::for_theme(ctx.theme(), &self.settings);
        // The player bar goes first so it spans the full width of the window.
        self.show_player_bar(ui, &ctx, &palette);
        self.show_sidebar(ui, &ctx, &palette);
        right_panel::show(
            ui,
            &self.localizer,
            &palette,
            self.settings.right_panel_open,
        );
        self.show_page(ui, &ctx, &palette);
        self.toasts.show(&ctx, &self.localizer, &palette);
    }

    /// Starts the cover textures once, if there is a cache folder, and uploads what the worker
    /// has decoded.
    fn poll_artwork(&mut self, ctx: &egui::Context) {
        if !self.artwork_started {
            self.artwork_started = true;
            self.artwork = self
                .deps
                .paths
                .as_ref()
                .map(|paths| ArtworkTextures::new(paths.cache_dir.join("artwork"), ctx));
        }
        if let Some(artwork) = &mut self.artwork {
            artwork.poll();
        }
    }

    fn show_player_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, palette: &Palette) {
        let now = NowPlaying::of(&self.player, &self.library.snapshot, &self.localizer);
        let position_ms = self.clock.as_ref().map_or(0, |clock| {
            player_bar::position_ms(clock.read(), &self.player)
        });
        let mut bar = PlayerBar {
            tr: &self.localizer,
            palette,
            view: &self.player,
            now: now.as_ref(),
            position_ms,
            settings: &self.settings,
            queue_open: self.settings.right_panel_open,
            artwork: self.artwork.as_mut(),
        };
        let actions = player_bar::show(ui, &mut bar);
        let now = ctx.input(|input| input.time);
        let moves_position = |action: &PlayerAction| {
            matches!(
                action,
                PlayerAction::Seek(_)
                    | PlayerAction::Previous
                    | PlayerAction::Next
                    | PlayerAction::TogglePlay
            )
        };
        if actions.iter().any(moves_position) {
            self.settle_until = now + SETTLE_FOR;
        }
        for action in actions {
            self.handle_player_action(ctx, action);
        }
        if now < self.settle_until {
            ctx.request_repaint_after(SETTLE_REPAINT);
        }
    }

    fn handle_player_action(&mut self, ctx: &egui::Context, action: PlayerAction) {
        let mut settings = self.settings.clone();
        match action {
            PlayerAction::TogglePlay => return self.send(Command::TogglePlay),
            PlayerAction::Next => return self.send(Command::Next),
            PlayerAction::Previous => return self.send(Command::Previous),
            PlayerAction::Seek(position_ms) => return self.send(Command::Seek(position_ms)),
            PlayerAction::ToggleQueue => return self.toggle_right_panel(ctx),
            PlayerAction::SetVolume(volume) => {
                settings.volume = volume;
                // Raising the volume is asking to hear it.
                if volume > 0.0 {
                    settings.muted = false;
                }
            }
            PlayerAction::ToggleMute => settings.muted = !settings.muted,
            PlayerAction::ToggleShuffle => settings.shuffle = !settings.shuffle,
            PlayerAction::CycleRepeat => {
                settings.repeat = match settings.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
            }
        }
        self.update_settings(ctx, settings);
    }

    /// Installs the fonts on the first frame and applies the visuals when they changed.
    fn prepare_style(&mut self, ui: &mut egui::Ui) {
        if !self.fonts_installed {
            // No text has been laid out this pass yet, so the fonts apply right away.
            fonts::install(ui.ctx());
            self.fonts_installed = true;
        }
        if self.style_dirty {
            visuals::apply(ui.ctx(), &self.settings);
            // The root `Ui` copied the old style when the pass began.
            ui.reset_style();
            self.style_dirty = false;
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        let events = match &self.deps.backend {
            Some(backend) => backend.drain(),
            None => return,
        };
        for event in events {
            self.handle_event(ctx, event);
        }
    }

    /// Applies one event from the backend. [`Self::show`] does this for each event the backend
    /// has sent; tests call it to play the backend's part.
    pub fn handle_event(&mut self, ctx: &egui::Context, event: Event) {
        match event {
            Event::Notice(notice) => self.notify(ctx, notice),
            Event::SettingsReplaced(settings) => {
                if self.outstanding_resets > 0 {
                    self.outstanding_resets -= 1;
                    tracing::debug!("the backend confirmed a reset");
                } else {
                    self.apply_settings(ctx, settings);
                }
            }
            Event::SettingsSaved => tracing::trace!("settings saved"),
            Event::LibraryUpdated(snapshot) => {
                self.library.snapshot = snapshot;
            }
            Event::ScanProgress(scan) => {
                // Covers that were missing may have been written by the scan that just ended;
                // between snapshots, a missing cover stays missing.
                let finished = self.library.scan.is_some() && scan.is_none();
                self.library.scan = scan;
                if finished && let Some(artwork) = &mut self.artwork {
                    artwork.retry_failed();
                }
            }
            Event::Player(player) => {
                self.library.current = player.current;
                self.player = player;
            }
            // Nothing shows the devices yet.
            Event::OutputDevices(_) => {}
        }
    }

    /// Shows `notice` as a toast from now on.
    fn notify(&mut self, ctx: &egui::Context, notice: Notice) {
        self.toasts.push(notice, ctx.input(|i| i.time));
    }

    /// Turns folders that failed to open since the last frame into a toast.
    fn collect_folder_failures(&mut self, ctx: &egui::Context) {
        if self.folders.failures() > 0 {
            self.notify(ctx, OPEN_FOLDER_FAILED);
        }
    }

    /// Adds the folder the user chose in the dialog, if one was chosen since the last frame.
    fn collect_picked_folder(&mut self) {
        if let Some(folder) = self.picker.picked() {
            self.send(Command::AddFolder(folder));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal_open {
            // The dialog is modal: nothing behind it changes until it is closed.
            return;
        }
        let shortcuts = ctx.input_mut(read_shortcuts);
        if shortcuts.toggle_right_panel {
            self.toggle_right_panel(ctx);
        }
        if let Some(route) = shortcuts.jump {
            self.history.navigate(route);
        }
        if shortcuts.back {
            self.history.back();
        }
        if shortcuts.forward {
            self.history.forward();
        }
    }

    fn show_sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, palette: &Palette) {
        // A window narrower than the saved width squeezes the panel, and egui then remembers
        // the squeezed width. Restore the saved one once there is room again.
        let squeezed = ui.available_width() < self.settings.sidebar_width;
        if self.sidebar_squeezed && !squeezed {
            self.sidebar_width_dirty = true;
        }
        self.sidebar_squeezed = squeezed;

        let output = sidebar::show(
            ui,
            &self.localizer,
            palette,
            &self.history,
            self.settings.sidebar_width,
            mem::take(&mut self.sidebar_width_dirty),
        );
        match output.action {
            Some(SidebarAction::Navigate(route)) => self.history.navigate(route),
            Some(SidebarAction::Back) => {
                self.history.back();
            }
            Some(SidebarAction::Forward) => {
                self.history.forward();
            }
            None => {}
        }
        if !squeezed {
            self.save_sidebar_width(ctx, output.width);
        }
    }

    /// Saves the sidebar's width once a resize has ended: the panel's width differs from
    /// the saved one and no pointer button is held.
    fn save_sidebar_width(&mut self, ctx: &egui::Context, width: f32) {
        let resized = (width - self.settings.sidebar_width).abs() > 0.5;
        let in_range = (SIDEBAR_MIN - 0.5..=SIDEBAR_MAX + 0.5).contains(&width);
        if resized && in_range && !ctx.input(|i| i.pointer.any_down()) {
            let mut settings = self.settings.clone();
            settings.sidebar_width = width;
            self.update_settings(ctx, settings);
        }
    }

    fn show_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, palette: &Palette) {
        let mut actions = Vec::new();
        let mut page = PageCtx {
            tr: &self.localizer,
            palette: *palette,
            settings: &self.settings,
            paths_available: self.deps.paths.is_some(),
            modal_open: &mut self.modal_open,
            out: &mut actions,
            library: &self.library,
            artwork: self.artwork.as_mut(),
        };
        let route = self.history.current();
        let frame = Frame::new()
            .fill(palette.bg)
            .inner_margin(Margin::symmetric(SPACE[4] as i8, SPACE[3] as i8));
        CentralPanel::default().frame(frame).show(ui, |ui| {
            let main = UiBuilder::new().accessibility_role(Role::Main);
            ui.scope_builder(main, |ui| pages::show(route, ui, &mut page));
        });
        for action in actions {
            self.handle_page_action(ctx, action);
        }
    }

    fn handle_page_action(&mut self, ctx: &egui::Context, action: PageAction) {
        match action {
            PageAction::UpdateSettings(settings) => self.update_settings(ctx, settings),
            PageAction::ResetSettings => {
                self.apply_settings(ctx, Settings::default());
                self.outstanding_resets = self.outstanding_resets.saturating_add(1);
                self.send(Command::ResetSettings);
            }
            PageAction::OpenConfigDir => self.open_folder(ctx, |paths| &paths.config_dir),
            PageAction::OpenLogDir => self.open_folder(ctx, |paths| &paths.log_dir),
            PageAction::PlayContext { ids, start } => {
                self.send(Command::PlayContext { ids, start });
            }
            PageAction::PlayNext(id) => self.send(Command::PlayNext(id)),
            PageAction::AddToQueue(id) => self.send(Command::AddToQueue(id)),
            PageAction::PickFolder => self.pick_folder(ctx),
            PageAction::RemoveFolder(id) => self.send(Command::RemoveFolder(id)),
            PageAction::Rescan => self.send(Command::Rescan),
            PageAction::ClearArtwork => self.send(Command::ClearArtwork),
        }
    }

    /// Opens the folder `pick` chooses in the file manager. Only folders Aria computed are
    /// ever opened.
    fn open_folder(&mut self, ctx: &egui::Context, pick: fn(&AppPaths) -> &PathBuf) {
        let started = match &self.deps.paths {
            Some(paths) => self.folders.open(ctx, pick(paths).clone()),
            None => Err(io::Error::other("Aria's folders are unknown")),
        };
        if let Err(err) = started {
            tracing::warn!(%err, "could not open the folder");
            self.notify(ctx, OPEN_FOLDER_FAILED);
        }
    }

    /// Opens the folder dialog, unless one is open already.
    fn pick_folder(&mut self, ctx: &egui::Context) {
        if let Err(err) = self.picker.start(ctx) {
            tracing::warn!(%err, "could not open the folder dialog");
            self.notify(ctx, PICK_FOLDER_FAILED);
        }
    }

    fn toggle_right_panel(&mut self, ctx: &egui::Context) {
        let mut settings = self.settings.clone();
        settings.right_panel_open = !settings.right_panel_open;
        self.update_settings(ctx, settings);
    }

    /// The one place settings change: applies `new` and has the backend save it.
    fn update_settings(&mut self, ctx: &egui::Context, new: Settings) {
        let new = new.sanitized();
        if new == self.settings {
            return;
        }
        self.apply_settings(ctx, new.clone());
        self.send(Command::UpdateSettings(new));
    }

    /// Applies `new` to the running app without asking the backend to save it.
    fn apply_settings(&mut self, ctx: &egui::Context, new: Settings) {
        let old = mem::replace(&mut self.settings, new.sanitized());
        let new = &self.settings;
        if old.theme != new.theme
            || old.accent_dark != new.accent_dark
            || old.accent_light != new.accent_light
        {
            self.style_dirty = true;
        }
        if old.language != new.language {
            let language = new.language.unwrap_or_else(os_language);
            if let Err(err) = self.localizer.set_language(language) {
                tracing::warn!(%err, "could not switch the interface language");
            }
        }
        if old.log_level != new.log_level
            && let Some(logging) = &self.deps.logging
        {
            logging.set_level(new.log_level);
        }
        if old.sidebar_width != new.sidebar_width {
            self.sidebar_width_dirty = true;
        }
        ctx.request_repaint();
    }

    fn send(&self, command: Command) {
        if let Some(backend) = &self.deps.backend {
            backend.send(command);
        }
    }
}

impl eframe::App for AriaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(backend) = self.deps.backend.take() {
            let clean = backend.shutdown(SHUTDOWN_TIMEOUT);
            tracing::debug!(clean, "backend stopped");
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        // The page background, so newly exposed areas never flash another color on resize.
        visuals.extreme_bg_color.to_normalized_gamma_f32()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use aria_core::{
        FakeFactory, OutputFormat,
        notice::NoticeLevel,
        runtime::{Backend, BackendConfig, BackendServices, Waker},
        settings::{Language, Theme},
    };
    use egui_kittest::Harness;

    use super::*;

    fn paths() -> AppPaths {
        AppPaths {
            config_dir: "config".into(),
            data_dir: "data".into(),
            cache_dir: "cache".into(),
            log_dir: "logs".into(),
        }
    }

    #[test]
    fn answers_to_own_resets_are_ignored() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.handle_page_action(&ctx, PageAction::ResetSettings);
        app.handle_page_action(&ctx, PageAction::ResetSettings);
        assert_eq!(app.settings, Settings::default());

        // A change made after the resets, before the backend answered them.
        let dark = Settings {
            theme: Theme::Dark,
            ..Settings::default()
        };
        app.update_settings(&ctx, dark.clone());
        for _ in 0..2 {
            app.handle_event(&ctx, Event::SettingsReplaced(Settings::default()));
            assert_eq!(
                app.settings, dark,
                "a late answer to a reset undid a change"
            );
        }

        // A replacement the app did not ask for is adopted.
        let light = Settings {
            theme: Theme::Light,
            ..Settings::default()
        };
        app.handle_event(&ctx, Event::SettingsReplaced(light.clone()));
        assert_eq!(app.settings, light);
    }

    #[test]
    fn replaced_settings_are_styled_in_the_frame_that_brings_them() {
        let woke = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&woke);
        let waker: Waker = Arc::new(move || flag.store(true, Ordering::SeqCst));
        // Read-only: nothing is written, and a reset is still answered.
        let services = BackendServices {
            library: None,
            output: Box::new(FakeFactory::new(OutputFormat {
                sample_rate: 48_000,
                channels: 2,
            })),
            settings: Settings::default(),
        };
        let backend = Backend::start(None, services, waker, BackendConfig::default()).unwrap();
        let settings = Settings {
            language: Some(Language::En),
            theme: Theme::Light,
            ..Settings::default()
        };
        let deps = AppDeps {
            backend: Some(backend),
            ..AppDeps::none()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        let app = AriaApp::new(settings, localizer, deps, vec![]);
        let mut harness =
            Harness::builder().build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
        assert_eq!(harness.ctx.theme(), egui::Theme::Light);

        // Sent past the reset count, so the answer is a replacement the app adopts.
        harness.state().send(Command::ResetSettings);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !woke.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(woke.load(Ordering::SeqCst), "the backend did not answer");

        // The frame that receives the defaults already draws with their theme, which follows
        // the OS: kittest reports none, so egui falls back to dark.
        harness.step();
        assert_eq!(harness.state().settings().theme, Theme::System);
        assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn backend_notices_become_toasts() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let failed = Notice {
            level: NoticeLevel::Error,
            key: "notice-settings-save-failed",
            args: Vec::new(),
        };
        app.handle_event(&ctx, Event::Notice(failed.clone()));
        assert_eq!(app.toasts.notices().collect::<Vec<_>>(), [&failed]);
    }

    #[test]
    fn failed_folder_open_shows_a_warning_toast() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.deps.paths = Some(paths());
        app.folders = FolderOpener::new(|_| Err(io::Error::other("no file manager")));
        app.handle_page_action(&ctx, PageAction::OpenLogDir);

        let deadline = Instant::now() + Duration::from_secs(5);
        while app.toasts.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
            app.collect_folder_failures(&ctx);
        }
        assert_eq!(
            app.toasts.notices().collect::<Vec<_>>(),
            [&OPEN_FOLDER_FAILED]
        );
    }

    #[test]
    fn folder_request_without_paths_warns_at_once() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.handle_page_action(&ctx, PageAction::OpenConfigDir);
        assert_eq!(
            app.toasts.notices().collect::<Vec<_>>(),
            [&OPEN_FOLDER_FAILED]
        );
    }

    fn app(notices: Vec<Notice>) -> AriaApp {
        let settings = Settings {
            language: Some(Language::En),
            ..Settings::default()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        AriaApp::new(settings, localizer, AppDeps::none(), notices)
    }

    #[test]
    fn failed_covers_are_retried_when_a_scan_ends_not_with_every_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = app(vec![]);
        let id = aria_library::ArtworkId("gone".into());
        let mut textures = ArtworkTextures::new(dir.path().to_path_buf(), &ctx);
        assert!(textures.get(&id).is_none());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !textures.has_failed(&id) {
            assert!(Instant::now() < deadline, "the cover was not tried");
            thread::sleep(Duration::from_millis(2));
            textures.poll();
        }
        app.artwork = Some(textures);

        // A snapshot, even during a scan, leaves the failure alone.
        let scan = aria_library::ScanProgress::default();
        app.handle_event(&ctx, Event::ScanProgress(Some(scan)));
        app.handle_event(&ctx, Event::LibraryUpdated(Arc::default()));
        assert!(app.artwork.as_ref().is_some_and(|a| a.has_failed(&id)));

        // The end of the scan forgets it, so the next draw asks again.
        app.handle_event(&ctx, Event::ScanProgress(None));
        let Some(artwork) = app.artwork.as_mut() else {
            panic!("the textures are there");
        };
        assert!(!artwork.has_failed(&id));
        assert!(artwork.get(&id).is_none());
        assert!(artwork.is_pending(&id));
    }

    #[test]
    fn no_startup_instant_by_default() {
        assert!(app(vec![]).startup.is_none());
    }

    #[test]
    fn first_frame_takes_the_startup_instant_once() {
        let app = app(vec![]).with_startup_instant(Instant::now());
        assert!(app.startup.is_some());
        let mut harness =
            Harness::builder().build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
        harness.step();
        assert!(
            harness.state().startup.is_none(),
            "the first frame logs its timing and forgets the instant"
        );
    }

    #[test]
    fn startup_notices_wait_to_be_shown() {
        let notice = Notice {
            level: NoticeLevel::Warning,
            key: "notice-settings-recovered",
            args: Vec::new(),
        };
        assert_eq!(app(vec![notice.clone()]).pending_notices(), [notice]);
    }

    #[test]
    fn starts_home_with_sanitized_settings() {
        let settings = Settings {
            sidebar_width: 9000.0,
            ..Settings::default()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        let app = AriaApp::new(settings, localizer, AppDeps::none(), vec![]);
        assert_eq!(app.current_route(), Route::Home);
        assert_eq!(app.settings().sidebar_width, SIDEBAR_MAX);
    }

    #[test]
    fn update_settings_skips_unchanged_settings() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.sidebar_width_dirty = false;
        app.update_settings(&ctx, app.settings.clone());
        assert!(!app.sidebar_width_dirty);
        let mut wider = app.settings.clone();
        wider.sidebar_width = 300.0;
        app.update_settings(&ctx, wider);
        assert_eq!(app.settings.sidebar_width, 300.0);
        assert!(app.sidebar_width_dirty);
    }

    #[test]
    fn theme_and_accent_changes_restyle() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.style_dirty = false;
        let mut settings = app.settings.clone();
        settings.log_level = aria_core::settings::LogLevel::Debug;
        app.update_settings(&ctx, settings.clone());
        assert!(!app.style_dirty, "a log level change needs no restyle");
        settings.accent_dark = Some(aria_core::settings::Rgb { r: 1, g: 2, b: 3 });
        app.update_settings(&ctx, settings);
        assert!(app.style_dirty);
    }
}
