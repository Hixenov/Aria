//! Application state and the per-frame shell: player bar, sidebar, right panel and page.

use aria_core::event::ActivePlayer;
use aria_core::spotify_service::{SpotifyAccount, SpotifyEvent, SpotifyRequest};

use crate::spotify::{RemoteCovers, SpotifyUi};
use std::{
    io, mem,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use aria_core::{
    Collections, DeviceInfo, PlaybackClock,
    command::Command,
    event::Event,
    hotkey::Hotkey,
    i18n::{Localizer, os_language},
    instance::Instance,
    logging::LogLevelHandle,
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    playback_service::PlayerView,
    player::queue::RepeatMode,
    runtime::BackendHandle,
    settings::{SIDEBAR_MAX, SIDEBAR_MIN, Settings},
    soundboard::SoundboardState,
};
use aria_library::{LibrarySnapshot, PLAYLIST_NAME_MAX, PlaylistId, SOUND_NAME_MAX, SoundId};
use eframe::egui::{self, CentralPanel, Frame, Margin, UiBuilder, accesskit::Role};

use crate::{
    artwork::ArtworkTextures,
    folders::{FolderOpener, FolderPicker, SoundPicker},
    hotkeys::{HotkeyAction, HotkeyBackend, Hotkeys, NoHotkeys, SystemHotkeys},
    nav::{History, Route},
    shortcuts::read_shortcuts,
    ui::{
        layout::{
            player_bar::{self, NowPlaying, PlayerAction, PlayerBar},
            right_panel::{self, QueueAction, QueuePanel},
            sidebar::{self, SidebarAction},
        },
        pages::{self, LibraryView, NameFor, PageAction, PageCtx},
        theme::{
            Palette, fonts,
            visuals::{self, SPACE},
        },
        widgets::{HotkeyAnswer, HotkeyCapture, HotkeyDialog, NameAnswer, NameDialog, Toasts},
    },
    whole_ms,
};

/// How long [`eframe::App::on_exit`] waits for the backend to save and stop.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long, in seconds, the app redraws often after a command that moves the position.
const SETTLE_FOR: f64 = 1.0;
/// How often it redraws then.
const SETTLE_REPAINT: Duration = Duration::from_millis(50);
/// Shown when the system refused a hotkey the user chose, which is then not saved.
const HOTKEY_UNAVAILABLE: Notice = Notice::new(NoticeLevel::Warning, "hotkey-unavailable");

/// Shown when global hotkeys cannot be had at all this session, and some are set.
const HOTKEYS_UNAVAILABLE: Notice = Notice::new(NoticeLevel::Warning, "hotkeys-unavailable");

/// Shown when the system refused `count` of the hotkeys Aria has, which do not work now.
fn hotkeys_inactive(count: usize) -> Notice {
    Notice::new(NoticeLevel::Warning, "hotkeys-inactive").with_arg("count", count.to_string())
}

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

/// Shown when the sound file dialog could not be opened.
const PICK_SOUNDS_FAILED: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "notice-pick-sounds-failed",
    args: Vec::new(),
};

/// What the hotkey dialog sets the hotkey of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HotkeyFor {
    /// This sound's.
    Sound(SoundId),
    /// The one of "Stop all".
    StopAll,
    /// The one that turns the microphone on and off.
    Microphone,
}

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
    /// The system's global hotkeys, made on the main thread before the window. Without them
    /// every hotkey is refused.
    pub hotkeys: Option<SystemHotkeys>,
    /// This copy's hold on the data folder, kept until the app ends, and the requests of later
    /// copies to show the window.
    pub instance: Option<Instance>,
}

impl AppDeps {
    /// No backend, logging, paths or hotkeys, for tests.
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
    /// The dialog for choosing sound files to add to the soundboard.
    sound_picker: SoundPicker,
    /// The dialog for choosing where a backup goes.
    backup_picker: FolderPicker,
    /// The dialog for choosing a backup to restore.
    restore_picker: FolderPicker,
    /// The dialog for choosing an M3U file to import.
    import_picker: FolderPicker,
    /// The dialog for choosing where a playlist is exported, and the playlist it is for.
    export_picker: FolderPicker,
    exporting: Option<PlaylistId>,
    /// What the backend last reported of the library and the player. Starts empty, since a
    /// disabled library sends nothing at all.
    library: LibraryView,
    /// What the backend last reported of the player.
    player: PlayerView,
    /// The tracks opened from outside the library this session, as the backend last reported
    /// them.
    opened: Arc<[aria_library::Track]>,
    /// The library's snapshot with the opened tracks in it too, once something has been opened:
    /// the player bar and the queue name whatever plays, while the pages list the library's own.
    /// See [`Self::playing_snapshot`].
    playing: Arc<LibrarySnapshot>,
    /// Favorites, playlists and recent plays as the backend last reported them. Empty until
    /// it does.
    collections: Arc<Collections>,
    /// The soundboard as the backend last reported it. Empty until it does.
    soundboard: Arc<SoundboardState>,
    /// The playlist name dialog, while it is open: what it is for and the text typed so far.
    name_dialog: Option<(NameFor, String)>,
    /// The hotkey dialog, while it is open: what it is for and the combination pressed so far.
    hotkey_dialog: Option<(HotkeyFor, HotkeyCapture)>,
    /// The global hotkeys registered with the system.
    hotkeys: Hotkeys<Box<dyn HotkeyBackend>>,
    /// Whether the system's hotkey manager could be made. Without it every hotkey is refused,
    /// and the notice says so rather than blaming other programs.
    hotkeys_available: bool,
    /// What the hotkeys are for changed, or the dialog opened or closed: they are synced at the
    /// end of the frame.
    hotkeys_dirty: bool,
    /// The hotkeys the system refused at the last sync, marked on their tiles.
    inactive_hotkeys: Vec<Hotkey>,
    /// The output devices the backend last listed. Empty until it answers.
    devices: Vec<DeviceInfo>,
    /// The microphones, as the backend last listed them.
    input_devices: Vec<DeviceInfo>,
    /// Whether the Settings page was on screen last frame, so that entering it asks for the
    /// devices once, not on every frame.
    on_settings: bool,
    /// Where playback is, read each frame by the player bar.
    clock: Option<Arc<PlaybackClock>>,
    /// Cover textures. Made on the first frame, once there is a context and a cache folder.
    artwork: Option<ArtworkTextures>,
    /// Until when, in egui's time, the app redraws often so that a seek or a skip shows its
    /// new position. The clock changes without any event to say so.
    settle_until: f64,
    /// Whether making `artwork` has been tried.
    artwork_started: bool,
    /// Whether a page's confirmation dialog is open: the Settings page's, or the Playlists
    /// page's delete question. The page sets and clears it;
    /// [`App::modal_open`] combines it with the name dialog.
    page_dialog_open: bool,
    /// Resets sent to the backend that it has not answered yet. Each answer brings the
    /// defaults, which are applied already, and changes made since are newer: it is ignored.
    outstanding_resets: u32,
    /// Every command sent, for tests to read.
    #[cfg(test)]
    sent: std::cell::RefCell<Vec<Command>>,
    /// When `main` began, until the first frame logs how long it took to arrive.
    startup: Option<Instant>,
    /// Spotify, as the backend last reported it.
    spotify: SpotifyUi,
    /// Spotify's covers.
    covers: RemoteCovers,
    /// Whether the window was shown at the last frame, as last told to the backend.
    window_shown: Option<bool>,
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
        mut deps: AppDeps,
        startup_notices: Vec<Notice>,
    ) -> AriaApp {
        let hotkeys_available = deps.hotkeys.is_some();
        let hotkeys: Box<dyn HotkeyBackend> = match deps.hotkeys.take() {
            Some(system) => Box::new(system),
            None => Box::new(NoHotkeys),
        };
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
            sound_picker: SoundPicker::default(),
            backup_picker: FolderPicker::default(),
            restore_picker: FolderPicker::default(),
            import_picker: FolderPicker::new(crate::folders::pick_playlist_file_with_system),
            export_picker: FolderPicker::default(),
            exporting: None,
            library: LibraryView::default(),
            player: PlayerView::default(),
            opened: Arc::from(Vec::new()),
            playing: Arc::default(),
            collections: Arc::default(),
            soundboard: Arc::default(),
            name_dialog: None,
            hotkey_dialog: None,
            hotkeys: Hotkeys::new(hotkeys),
            hotkeys_available,
            hotkeys_dirty: true,
            inactive_hotkeys: Vec::new(),
            devices: Vec::new(),
            input_devices: Vec::new(),
            on_settings: false,
            clock,
            artwork: None,
            artwork_started: false,
            settle_until: 0.0,
            #[cfg(test)]
            sent: std::cell::RefCell::default(),
            page_dialog_open: false,
            outstanding_resets: 0,
            startup: None,
            spotify: SpotifyUi::default(),
            covers: RemoteCovers::default(),
            window_shown: None,
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

    /// Spotify as the backend last reported it.
    pub fn spotify(&self) -> &SpotifyUi {
        &self.spotify
    }

    /// The soundboard as the backend last reported it.
    pub fn soundboard(&self) -> &SoundboardState {
        &self.soundboard
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
        self.tell_window_shown(&ctx);
        self.prepare_style(ui);
        for notice in mem::take(&mut self.pending_notices) {
            self.notify(&ctx, notice);
        }
        self.collect_folder_failures(&ctx);
        self.collect_picked_folder();
        self.collect_picked_sounds();
        self.open_dropped(&ctx);
        self.handle_shortcuts(&ctx);

        self.poll_artwork(&ctx);

        let palette = Palette::for_theme(ctx.theme(), &self.settings);
        // The player bar goes first so it spans the full width of the window.
        self.show_player_bar(ui, &ctx, &palette);
        self.show_sidebar(ui, &ctx, &palette);
        self.show_right_panel(ui, &palette);
        self.show_page(ui, &ctx, &palette);
        self.show_name_dialog(&ctx, &palette);
        self.show_hotkey_dialog(&ctx, &palette);
        // After the dialogs, so one that opened this frame already has the keys to itself.
        self.run_hotkeys(&ctx);
        self.toasts.show(&ctx, &self.localizer, &palette);
        self.show_drop_hint(&ctx, &palette);
    }

    /// Tells the backend when the window is minimized or shown again, so Spotify is not polled
    /// while nobody can see it. The first frame tells it the window is shown.
    fn tell_window_shown(&mut self, ctx: &egui::Context) {
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let shown = !minimized;
        if self.window_shown != Some(shown) {
            self.window_shown = Some(shown);
            self.send(Command::WindowShown(shown));
        }
    }

    /// Opens what was dropped on the window this frame, in the order dropped: files play, and
    /// folders are added to the library.
    fn open_dropped(&mut self, ctx: &egui::Context) {
        let paths: Vec<std::path::PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !paths.is_empty() {
            self.send(Command::OpenPaths(paths));
        }
    }

    /// While files are held over the window, says what dropping them does.
    fn show_drop_hint(&self, ctx: &egui::Context, p: &Palette) {
        if ctx.input(|input| input.raw.hovered_files.is_empty()) {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("aria_drop_hint"),
        ));
        painter.rect_filled(screen, 0.0, p.bg.gamma_multiply(0.85));
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            self.localizer.tr("drop-hint"),
            egui::TextStyle::Heading.resolve(&ctx.global_style()),
            p.text,
        );
    }

    fn show_right_panel(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let mut queue = QueuePanel {
            view: &self.player,
            snapshot: if self.opened.is_empty() {
                &self.library.snapshot
            } else {
                &self.playing
            },
            artwork: self.artwork.as_mut(),
        };
        let actions = right_panel::show(
            ui,
            &self.localizer,
            palette,
            self.settings.right_panel_open,
            &mut queue,
        );
        for action in actions {
            self.send(match action {
                QueueAction::Remove(index) => Command::RemoveFromQueue(index),
                QueueAction::Move { from, to } => Command::MoveInQueue { from, to },
                QueueAction::Clear => Command::ClearQueue,
            });
        }
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
        let now = NowPlaying::of(&self.player, self.playing_snapshot(), &self.localizer);
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
            collections: &self.collections,
            spotify: if self.spotify.active == ActivePlayer::Spotify {
                self.spotify.playback.as_deref()
            } else {
                None
            },
            spotify_devices: self
                .spotify
                .signed_in()
                .then_some(self.spotify.devices.as_slice()),
            covers: Some(&mut self.covers),
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
                    | PlayerAction::Spotify(
                        SpotifyRequest::Seek(_)
                            | SpotifyRequest::Previous
                            | SpotifyRequest::Next
                            | SpotifyRequest::TogglePlay
                    )
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
            PlayerAction::ToggleMicrophone => return self.send(Command::ToggleMicrophone),
            PlayerAction::SetSleepTimer(timer) => {
                return self.send(Command::SetSleepTimer(timer));
            }
            PlayerAction::SetFavorite { track, favorite } => {
                return self.send(Command::SetFavorite { track, favorite });
            }
            PlayerAction::Spotify(request) => return self.send(Command::Spotify(request)),
            PlayerAction::SpotifyDevice(id) => {
                return self.send(Command::Spotify(SpotifyRequest::Device(id)));
            }
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

    /// The snapshot the player bar and the queue name tracks from: the library's, with the tracks
    /// opened from outside it.
    fn playing_snapshot(&self) -> &LibrarySnapshot {
        if self.opened.is_empty() {
            &self.library.snapshot
        } else {
            &self.playing
        }
    }

    /// Rebuilds [`Self::playing`], once something has been opened: a copy of the library's
    /// snapshot with the opened tracks added, which sort first by their negative ids.
    fn rebuild_playing(&mut self) {
        if self.opened.is_empty() {
            return;
        }
        let mut playing = (*self.library.snapshot).clone();
        playing.tracks.extend(self.opened.iter().cloned());
        playing.tracks.sort_by_key(|track| track.id);
        self.playing = Arc::new(playing);
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
                self.rebuild_playing();
            }
            Event::Opened(opened) => {
                self.opened = opened;
                self.rebuild_playing();
            }
            Event::Collections(collections) => self.collections = collections,
            Event::Soundboard(soundboard) => {
                self.soundboard = soundboard;
                self.hotkeys_dirty = true;
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
            Event::OutputDevices(devices) => self.devices = devices,
            Event::InputDevices(devices) => self.input_devices = devices,
            Event::ActivePlayer(active) => self.spotify.active = active,
            Event::Spotify(SpotifyEvent::Notice(notice)) => self.notify(ctx, notice),
            Event::Spotify(SpotifyEvent::Cover(cover)) => self.covers.insert(ctx, &cover),
            Event::Spotify(event) => {
                if let SpotifyEvent::Account(SpotifyAccount::SignedOut { .. }) = &event {
                    self.covers.clear();
                }
                self.spotify.apply(event);
            }
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
        if let Some(folder) = self.backup_picker.picked() {
            self.send(Command::BackUp(folder));
        }
        if let Some(folder) = self.restore_picker.picked() {
            self.send(Command::Restore(folder));
        }
        if let Some(file) = self.import_picker.picked() {
            self.send(Command::ImportPlaylist(file));
        }
        if let Some(to) = self.export_picker.picked()
            && let Some(id) = self.exporting.take()
        {
            self.send(Command::ExportPlaylist { id, to });
        }
    }

    /// Adds the sound files the user chose in the dialog, if some were chosen since the last
    /// frame.
    fn collect_picked_sounds(&mut self) {
        if let Some(files) = self.sound_picker.picked() {
            self.send(Command::AddSounds(files));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal_open() {
            // The dialog is modal: nothing behind it changes until it is closed.
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let shortcuts = ctx.input_mut(|input| read_shortcuts(input, typing));
        if shortcuts.toggle_play {
            self.settle_until = ctx.input(|input| input.time) + SETTLE_FOR;
            self.send(Command::TogglePlay);
        }
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
            exclusive_supported: aria_core::playback_service::EXCLUSIVE_SUPPORTED,
            modal_open: &mut self.page_dialog_open,
            out: &mut actions,
            library: &self.library,
            collections: &self.collections,
            soundboard: &self.soundboard,
            inactive_hotkeys: &self.inactive_hotkeys,
            devices: &self.devices,
            input_devices: &self.input_devices,
            artwork: self.artwork.as_mut(),
            spotify: &self.spotify,
            covers: &mut self.covers,
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
        self.list_devices_on_entering_settings(route);
    }

    /// Asks the backend for the output devices and the microphones once per entry into the
    /// Settings route, so the lists are as fresh as the user's last visit and no frame waits for
    /// them. A cable installed since is found this way too.
    fn list_devices_on_entering_settings(&mut self, route: Route) {
        let on_settings = route == Route::Settings;
        if on_settings && !self.on_settings {
            self.send(Command::ListDevices);
            self.send(Command::ListInputDevices);
        }
        self.on_settings = on_settings;
    }

    fn handle_page_action(&mut self, ctx: &egui::Context, action: PageAction) {
        match action {
            PageAction::UpdateSettings(settings) => self.update_settings(ctx, settings),
            PageAction::Spotify(SpotifyRequest::SignIn { .. }) => {
                let text = self.localizer.tr("spotify-signed-in-page");
                let done_page = crate::spotify::signed_in_page(&text);
                self.send(Command::Spotify(SpotifyRequest::SignIn { done_page }));
            }
            PageAction::Spotify(SpotifyRequest::Search(text)) => {
                self.spotify.search_text.clone_from(&text);
                self.send(Command::Spotify(SpotifyRequest::Search(text)));
            }
            PageAction::Spotify(request) => self.send(Command::Spotify(request)),
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
            PageAction::ImportPlaylist => {
                if let Err(err) = self.import_picker.start(ctx) {
                    tracing::warn!(%err, "could not open the file dialog");
                    self.notify(ctx, PICK_FOLDER_FAILED);
                }
            }
            PageAction::ExportPlaylist { id, name } => {
                let started = self.export_picker.start_with(ctx, move || {
                    crate::folders::save_playlist_file_with_system(&name)
                });
                match started {
                    Ok(()) => self.exporting = Some(id),
                    Err(err) => {
                        tracing::warn!(%err, "could not open the save dialog");
                        self.notify(ctx, PICK_FOLDER_FAILED);
                    }
                }
            }
            PageAction::PickBackupFolder => {
                if let Err(err) = self.backup_picker.start(ctx) {
                    tracing::warn!(%err, "could not open the folder dialog");
                    self.notify(ctx, PICK_FOLDER_FAILED);
                }
            }
            PageAction::PickRestoreFolder => {
                if let Err(err) = self.restore_picker.start(ctx) {
                    tracing::warn!(%err, "could not open the folder dialog");
                    self.notify(ctx, PICK_FOLDER_FAILED);
                }
            }
            PageAction::RemoveFolder(id) => self.send(Command::RemoveFolder(id)),
            PageAction::Rescan => self.send(Command::Rescan),
            PageAction::ClearArtwork => self.send(Command::ClearArtwork),
            PageAction::SetFavorite { track, favorite } => {
                self.send(Command::SetFavorite { track, favorite });
            }
            PageAction::AddToPlaylist { playlist, track } => {
                self.send(Command::AddToPlaylist { playlist, track });
            }
            PageAction::RemoveFromPlaylist { playlist, position } => {
                self.send(Command::RemoveFromPlaylist { playlist, position });
            }
            PageAction::MoveInPlaylist { playlist, from, to } => {
                self.send(Command::MoveInPlaylist { playlist, from, to });
            }
            PageAction::DeletePlaylist(id) => self.send(Command::DeletePlaylist(id)),
            PageAction::ClearHistory => self.send(Command::ClearHistory),
            PageAction::AskPlaylistName(purpose) => self.open_name_dialog(purpose),
            PageAction::AddSounds => self.pick_sounds(ctx),
            PageAction::AskSoundName(id) => self.open_name_dialog(NameFor::Sound(id)),
            PageAction::SetSoundVolume { id, volume } => {
                self.send(Command::SetSoundVolume { id, volume });
            }
            PageAction::RemoveSound(id) => self.send(Command::RemoveSound(id)),
            PageAction::PlaySound(id) => self.send(Command::PlaySound(id)),
            PageAction::StopSounds => self.send(Command::StopSounds),
            PageAction::AskSoundHotkey(id) => self.open_hotkey_dialog(HotkeyFor::Sound(id)),
            PageAction::AskStopHotkey => self.open_hotkey_dialog(HotkeyFor::StopAll),
            PageAction::AskMicHotkey => self.open_hotkey_dialog(HotkeyFor::Microphone),
        }
    }

    /// Opens the name dialog for `purpose`, with the playlist's current name when renaming.
    fn open_name_dialog(&mut self, purpose: NameFor) {
        let text = match purpose {
            NameFor::New { .. } => String::new(),
            NameFor::Rename(id) => match self.playlist_name(id) {
                Some(name) => name.to_owned(),
                None => {
                    tracing::debug!(id = id.0, "no such playlist to rename");
                    return;
                }
            },
            NameFor::Sound(id) => match self.sound_name(id) {
                Some(name) => name.to_owned(),
                None => {
                    tracing::debug!(id = id.0, "no such sound to rename");
                    return;
                }
            },
        };
        self.name_dialog = Some((purpose, text));
    }

    /// Whether a modal dialog is open: a page's confirmation (Settings or Playlists), the name
    /// dialog or the hotkey dialog. Navigation and panel shortcuts wait while one is. Each dialog
    /// owns its own state, so closing one cannot close the other's hold on the app.
    fn modal_open(&self) -> bool {
        self.page_dialog_open || self.name_dialog.is_some() || self.hotkey_dialog.is_some()
    }

    /// The name of playlist `id`, if the collections know it.
    fn playlist_name(&self, id: aria_library::PlaylistId) -> Option<&str> {
        let playlists = &self.collections.playlists;
        let playlist = playlists.iter().find(|playlist| playlist.id == id)?;
        Some(&playlist.name)
    }

    /// The name of sound `id`, if the soundboard has it.
    fn sound_name(&self, id: aria_library::SoundId) -> Option<&str> {
        let sound = self.soundboard.sounds.iter().find(|sound| sound.id == id)?;
        Some(&sound.name)
    }

    /// The hotkey `target` has now, or `None` for a sound the soundboard no longer has.
    fn hotkey_of(&self, target: HotkeyFor) -> Option<Option<Hotkey>> {
        match target {
            HotkeyFor::Sound(id) => {
                let sounds = &self.soundboard.sounds;
                sounds.iter().find(|sound| sound.id == id).map(|s| s.hotkey)
            }
            HotkeyFor::StopAll => Some(self.settings.stop_sounds_hotkey),
            HotkeyFor::Microphone => Some(self.settings.mic_toggle_hotkey),
        }
    }

    /// What each hotkey is for: the sounds' first, then the one of Stop all, then the
    /// microphone's.
    fn wanted_hotkeys(&self) -> Vec<(Hotkey, HotkeyAction)> {
        let sounds = self.soundboard.sounds.iter();
        let sounds = sounds.filter_map(|sound| Some((sound.hotkey?, HotkeyAction::Play(sound.id))));
        let stop = self
            .settings
            .stop_sounds_hotkey
            .map(|hotkey| (hotkey, HotkeyAction::StopAll));
        let microphone = self
            .settings
            .mic_toggle_hotkey
            .map(|hotkey| (hotkey, HotkeyAction::ToggleMicrophone));
        sounds.chain(stop).chain(microphone).collect()
    }

    /// Opens the hotkey dialog for `target`, unless it is a sound the soundboard does not have.
    /// The hotkeys are unregistered while it is open, so the combination reaches the dialog.
    fn open_hotkey_dialog(&mut self, target: HotkeyFor) {
        let Some(current) = self.hotkey_of(target) else {
            tracing::debug!(?target, "no such sound to set a hotkey for");
            return;
        };
        self.hotkey_dialog = Some((target, HotkeyCapture::new(current)));
        self.hotkeys_dirty = true;
    }

    /// Draws the hotkey dialog while it is open, and saves what it was confirmed with. A hotkey
    /// the system refuses is not saved.
    fn show_hotkey_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        let Some((target, mut capture)) = self.hotkey_dialog.take() else {
            return;
        };
        // A sound removed while its dialog was open has nothing left to set.
        let Some(current) = self.hotkey_of(target) else {
            self.hotkeys_dirty = true;
            return;
        };
        // The hotkeys used elsewhere in Aria: refused here, before anything is sent.
        let taken: Vec<Hotkey> = self
            .wanted_hotkeys()
            .into_iter()
            .filter(|(_, action)| match (target, action) {
                (HotkeyFor::Sound(id), HotkeyAction::Play(other)) => *other != id,
                (HotkeyFor::StopAll, HotkeyAction::StopAll)
                | (HotkeyFor::Microphone, HotkeyAction::ToggleMicrophone) => false,
                _ => true,
            })
            .map(|(hotkey, _)| hotkey)
            .collect();
        let dialog = HotkeyDialog {
            current,
            taken: &taken,
        };
        match dialog.show(ctx, &self.localizer, palette, &mut capture) {
            HotkeyAnswer::Open => {
                self.hotkey_dialog = Some((target, capture));
                return;
            }
            HotkeyAnswer::Cancelled => {}
            HotkeyAnswer::Cleared => {
                if current.is_some() {
                    self.set_hotkey(ctx, target, None);
                }
            }
            HotkeyAnswer::Confirmed(hotkey) if Some(hotkey) == current => {}
            HotkeyAnswer::Confirmed(hotkey) => {
                if self.hotkeys.try_register(hotkey) {
                    self.set_hotkey(ctx, target, Some(hotkey));
                } else {
                    self.notify(ctx, HOTKEY_UNAVAILABLE);
                }
            }
        }
        self.hotkeys_dirty = true;
    }

    /// Saves `hotkey` as the one of `target`.
    fn set_hotkey(&mut self, ctx: &egui::Context, target: HotkeyFor, hotkey: Option<Hotkey>) {
        match target {
            HotkeyFor::Sound(id) => self.send(Command::SetSoundHotkey { id, hotkey }),
            HotkeyFor::StopAll => {
                let mut settings = self.settings.clone();
                settings.stop_sounds_hotkey = hotkey;
                self.update_settings(ctx, settings);
            }
            HotkeyFor::Microphone => {
                let mut settings = self.settings.clone();
                settings.mic_toggle_hotkey = hotkey;
                self.update_settings(ctx, settings);
            }
        }
    }

    /// Syncs the hotkeys with the system when that is due, and fires what was pressed.
    fn run_hotkeys(&mut self, ctx: &egui::Context) {
        if mem::take(&mut self.hotkeys_dirty) {
            self.sync_hotkeys(ctx);
        }
        self.fire_hotkeys();
    }

    /// Sends what the hotkeys pressed since the last look are for. A press wakes the window
    /// itself, and eframe then runs [`eframe::App::logic`], which calls this, even while the
    /// window is minimized and draws nothing.
    fn fire_hotkeys(&mut self) {
        for action in self.hotkeys.poll() {
            self.send(match action {
                HotkeyAction::Play(id) => Command::PlaySound(id),
                HotkeyAction::StopAll => Command::StopSounds,
                HotkeyAction::ToggleMicrophone => Command::ToggleMicrophone,
            });
        }
    }

    /// Registers the hotkeys wanted now: none while the switch is off or the hotkey dialog is
    /// open. The hotkeys the system refused are marked, and one notice counts them when some
    /// are newly refused.
    fn sync_hotkeys(&mut self, ctx: &egui::Context) {
        let suspended = self.hotkey_dialog.is_some();
        let enabled = self.settings.global_hotkeys && !suspended;
        let wanted = self.wanted_hotkeys();
        let refused = self.hotkeys.sync(enabled, &wanted);
        // Nothing was asked of the system while the dialog is open: the marks stay as they were.
        if suspended {
            return;
        }
        let newly = refused
            .iter()
            .any(|hotkey| !self.inactive_hotkeys.contains(hotkey));
        if newly {
            let now = ctx.input(|input| input.time);
            if self.hotkeys_available {
                // One notice counts them all, in place of one that counted fewer.
                self.toasts
                    .push_replacing(hotkeys_inactive(refused.len()), now);
            } else {
                self.toasts.push(HOTKEYS_UNAVAILABLE, now);
            }
        }
        self.inactive_hotkeys = refused;
    }

    /// Draws the name dialog while it is open, and sends what it was confirmed for.
    fn show_name_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        let Some((purpose, mut text)) = self.name_dialog.take() else {
            return;
        };
        // A playlist deleted while its rename was open has nothing left to rename.
        if let NameFor::Rename(id) = purpose
            && self.playlist_name(id).is_none()
        {
            return;
        }
        // A sound removed while its rename was open has nothing left to rename.
        if let NameFor::Sound(id) = purpose
            && self.sound_name(id).is_none()
        {
            return;
        }
        let (title, confirm, max_chars) = match purpose {
            NameFor::New { .. } => ("playlist-new", "playlist-create", PLAYLIST_NAME_MAX),
            NameFor::Rename(_) => ("playlist-rename", "playlist-rename", PLAYLIST_NAME_MAX),
            NameFor::Sound(_) => ("sound-rename", "sound-rename", SOUND_NAME_MAX),
        };
        let (title, confirm) = (self.localizer.tr(title), self.localizer.tr(confirm));
        let dialog = NameDialog {
            title: &title,
            confirm: &confirm,
            max_chars,
        };
        match dialog.show(ctx, &self.localizer, palette, &mut text) {
            NameAnswer::Open => {
                self.name_dialog = Some((purpose, text));
            }
            NameAnswer::Cancelled => {}
            NameAnswer::Confirmed(name) => {
                self.send(match purpose {
                    NameFor::New { first } => Command::CreatePlaylist { name, first },
                    NameFor::Rename(id) => Command::RenamePlaylist { id, name },
                    NameFor::Sound(id) => Command::RenameSound { id, name },
                });
            }
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

    /// Opens the sound file dialog, unless one is open already.
    fn pick_sounds(&mut self, ctx: &egui::Context) {
        if let Err(err) = self.sound_picker.start(ctx) {
            tracing::warn!(%err, "could not open the sound file dialog");
            self.notify(ctx, PICK_SOUNDS_FAILED);
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
        if old.global_hotkeys != new.global_hotkeys
            || old.stop_sounds_hotkey != new.stop_sounds_hotkey
            || old.mic_toggle_hotkey != new.mic_toggle_hotkey
        {
            self.hotkeys_dirty = true;
        }
        ctx.request_repaint();
    }

    fn send(&self, command: Command) {
        #[cfg(test)]
        self.sent.borrow_mut().push(command.clone());
        if let Some(backend) = &self.deps.backend {
            backend.send(command);
        }
    }
}

impl AriaApp {
    /// What brings the window forward, when another copy of Aria has asked for it since the
    /// last call: shown, restored if minimized, and focused.
    /// Opens the files and folders another launch of Aria was started with, as it asked.
    fn open_requested(&mut self) {
        let Some(instance) = &self.deps.instance else {
            return;
        };
        let paths = instance.take_opened();
        if !paths.is_empty() {
            self.send(Command::OpenPaths(paths));
        }
    }

    fn window_requests(&self) -> Vec<egui::ViewportCommand> {
        let asked = self.deps.instance.as_ref().is_some_and(Instance::take_wake);
        if !asked {
            return Vec::new();
        }
        vec![
            egui::ViewportCommand::Visible(true),
            egui::ViewportCommand::Minimized(false),
            egui::ViewportCommand::Focus,
        ]
    }
}

impl eframe::App for AriaApp {
    /// Runs before every [`Self::ui`], and alone while the window is minimized. So the
    /// backend's events (a new soundboard list, replaced settings) are taken in and the hotkeys
    /// synced here too: an Aria started minimized registers its hotkeys without a drawn frame.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for command in self.window_requests() {
            ctx.send_viewport_cmd(command);
        }
        self.open_requested();
        self.drain_events(ctx);
        self.run_hotkeys(ctx);
    }

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
        CaptureFormat, FakeCapture, FakeFactory, OutputFormat,
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
            spotify: None,
            library: None,
            output: Box::new(FakeFactory::new(OutputFormat {
                sample_rate: 48_000,
                channels: 2,
            })),
            capture: Box::new(
                FakeCapture::new(CaptureFormat {
                    sample_rate: 48_000,
                    channels: 1,
                })
                .0,
            ),
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
    fn a_second_launch_brings_the_window_forward_once() {
        use aria_core::instance::{self, Claim};
        let dir = tempfile::tempdir().unwrap();
        let Claim::First(held) = instance::claim(dir.path(), instance::REACH_WAIT).unwrap() else {
            panic!("expected to be the first");
        };
        let deps = AppDeps {
            instance: Some(held),
            ..AppDeps::none()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        let app = AriaApp::new(Settings::default(), localizer, deps, vec![]);
        assert!(app.window_requests().is_empty());

        let second = instance::claim(dir.path(), instance::REACH_WAIT).unwrap();
        assert!(matches!(second, Claim::Running { reached: true }));
        let started = std::time::Instant::now();
        let mut requests = Vec::new();
        while requests.is_empty() && started.elapsed() < std::time::Duration::from_secs(10) {
            requests = app.window_requests();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            requests,
            vec![
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Minimized(false),
                egui::ViewportCommand::Focus,
            ]
        );
        assert!(app.window_requests().is_empty(), "asked once, shown once");
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

    fn plain_app() -> AriaApp {
        let localizer = Localizer::new(Language::En).unwrap();
        AriaApp::new(Settings::default(), localizer, AppDeps::none(), vec![])
    }

    #[test]
    fn spotify_events_reach_the_ui_state() {
        use aria_core::event::ActivePlayer;
        let ctx = egui::Context::default();
        let mut app = plain_app();
        app.handle_event(
            &ctx,
            Event::Spotify(SpotifyEvent::Account(SpotifyAccount::SignedIn {
                name: "Ann".to_owned(),
            })),
        );
        app.handle_event(&ctx, Event::ActivePlayer(ActivePlayer::Spotify));
        assert!(app.spotify().signed_in());
        assert_eq!(app.spotify().active, ActivePlayer::Spotify);
        app.handle_event(
            &ctx,
            Event::Spotify(SpotifyEvent::Cover(std::sync::Arc::new(
                aria_core::spotify_service::Cover {
                    url: "https://i.scdn.co/a".to_owned(),
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                },
            ))),
        );
        assert_eq!(app.covers.len(), 1);
        app.handle_event(
            &ctx,
            Event::Spotify(SpotifyEvent::Account(SpotifyAccount::SignedOut {
                keyring: true,
            })),
        );
        assert!(app.covers.is_empty(), "covers go with the sign-out");
    }

    #[test]
    fn minimizing_tells_the_backend() {
        let mut harness =
            Harness::builder().build_ui_state(|ui, app: &mut AriaApp| app.show(ui), plain_app());
        harness.run();
        let shown = |harness: &Harness<'_, AriaApp>| -> Vec<bool> {
            harness
                .state()
                .sent
                .borrow()
                .iter()
                .filter_map(|c| match c {
                    Command::WindowShown(shown) => Some(*shown),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(shown(&harness), [true], "the first frame says shown, once");
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .minimized = Some(true);
        harness.step();
        assert_eq!(shown(&harness), [true, false]);
    }

    #[test]
    fn sign_in_carries_the_page_text() {
        let ctx = egui::Context::default();
        let mut app = plain_app();
        app.handle_page_action(
            &ctx,
            PageAction::Spotify(SpotifyRequest::SignIn {
                done_page: String::new(),
            }),
        );
        let sent = app.sent.borrow();
        let [Command::Spotify(SpotifyRequest::SignIn { done_page })] = sent.as_slice() else {
            panic!("{sent:?}");
        };
        assert!(done_page.contains("You can close this tab"), "{done_page}");
    }

    fn toggles_sent(harness: &Harness<'_, AriaApp>) -> usize {
        harness
            .state()
            .sent
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::TogglePlay))
            .count()
    }

    #[test]
    fn files_a_second_launch_was_started_with_are_opened_here() {
        use aria_core::instance::{self, Claim};
        let dir = tempfile::tempdir().unwrap();
        let Claim::First(held) = instance::claim(dir.path(), instance::REACH_WAIT).unwrap() else {
            panic!("expected to be the first");
        };
        let deps = AppDeps {
            instance: Some(held),
            ..AppDeps::none()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        let mut app = AriaApp::new(Settings::default(), localizer, deps, vec![]);
        let paths: Vec<std::path::PathBuf> = vec!["C:/Music/a.mp3".into()];
        let second = instance::claim_opening(dir.path(), instance::REACH_WAIT, &paths).unwrap();
        assert!(matches!(second, Claim::Running { reached: true }));
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(10) {
            app.open_requested();
            if !app.sent.borrow().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let sent = app.sent.borrow();
        assert!(
            matches!(sent.as_slice(), [Command::OpenPaths(opened)] if *opened == paths),
            "{sent:?}"
        );
    }

    #[test]
    fn files_dropped_on_the_window_are_opened_in_the_order_dropped() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app(vec![]));
        harness.run();
        let paths: Vec<std::path::PathBuf> = vec!["C:/Music/a.mp3".into(), "C:/Music/Album".into()];
        /// A file dropped by the test: only its path is read.
        #[derive(Debug)]
        struct Dropped(std::path::PathBuf);
        impl egui::DroppedFile for Dropped {
            fn path(&self) -> &std::path::Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                Err("not read".into())
            }
        }
        harness.input_mut().dropped_files = paths
            .iter()
            .map(|path| Arc::new(Dropped(path.clone())) as egui::DroppedFileHandle)
            .collect();
        harness.run();
        let opened: Vec<Vec<std::path::PathBuf>> = harness
            .state()
            .sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::OpenPaths(paths) => Some(paths.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(opened, [paths], "dropped once, opened once");
    }

    fn list_devices_sent(harness: &Harness<'_, AriaApp>) -> usize {
        harness
            .state()
            .sent
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::ListDevices))
            .count()
    }

    #[test]
    fn entering_settings_lists_the_devices_once_per_entry() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app(vec![]));
        harness.run();
        assert_eq!(list_devices_sent(&harness), 0, "listed away from Settings");
        harness.state_mut().history.navigate(Route::Settings);
        // Several frames on the page: still one request.
        harness.run();
        harness.run();
        assert_eq!(list_devices_sent(&harness), 1);
        harness.state_mut().history.navigate(Route::Home);
        harness.run();
        harness.state_mut().history.navigate(Route::Settings);
        harness.run();
        assert_eq!(list_devices_sent(&harness), 2, "a second visit asks again");
    }

    #[test]
    fn the_device_list_is_kept() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let device = DeviceInfo {
            id: aria_core::DeviceId("a".into()),
            name: "Speakers".into(),
            is_default: true,
        };
        app.handle_event(&ctx, Event::OutputDevices(vec![device.clone()]));
        assert_eq!(app.devices, [device]);
    }

    #[test]
    fn the_microphone_list_requests_input_devices() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app(vec![]));
        harness.run();
        let listed =
            |harness: &Harness<'_, AriaApp>| sent_like(harness.state(), &Command::ListInputDevices);
        assert_eq!(listed(&harness), 0, "listed away from Settings");
        harness.state_mut().history.navigate(Route::Settings);
        harness.run();
        harness.run();
        assert_eq!(listed(&harness), 1);

        let ctx = harness.ctx.clone();
        let mic = DeviceInfo {
            id: aria_core::DeviceId("mic".into()),
            name: "Microphone".into(),
            is_default: true,
        };
        harness
            .state_mut()
            .handle_event(&ctx, Event::InputDevices(vec![mic.clone()]));
        assert_eq!(harness.state().input_devices, [mic]);
    }

    #[test]
    fn the_microphone_hotkey_sends_toggle() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![], &fake);
        let ctx = harness.ctx.clone();
        let mut settings = harness.state().settings.clone();
        settings.mic_toggle_hotkey = Some(key("Ctrl+Alt+M"));
        harness.state_mut().update_settings(&ctx, settings);
        harness.run();
        assert_eq!(fake.registered(), [key("Ctrl+Alt+M")].into());

        fake.press(key("Ctrl+Alt+M"));
        harness.run();

        assert_eq!(sent_like(harness.state(), &Command::ToggleMicrophone), 1);
    }

    #[test]
    fn the_microphone_hotkey_is_saved_in_the_settings() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![], &fake);
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskMicHotkey);
        harness.run();
        harness.key_press(egui::Key::F8);
        harness.run();
        click(&mut harness, "settings-confirm");

        assert_eq!(harness.state().settings.mic_toggle_hotkey, Some(key("F8")));
        assert_eq!(fake.registered(), [key("F8")].into());
    }

    #[test]
    fn the_dialog_refuses_a_hotkey_taken_by_a_sound() {
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![keyed(1, "Airhorn", "Ctrl+1")], &fake);
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskMicHotkey);
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Num1);
        harness.run();

        harness.get_by_label(&english("hotkey-taken"));
        let label = english("settings-confirm");
        let confirm = harness.get_by_role_and_label(egui::accesskit::Role::Button, &label);
        assert!(confirm.accesskit_node().is_disabled());
        assert_eq!(harness.state().settings.mic_toggle_hotkey, None);
    }

    #[test]
    fn the_microphone_icon_click_toggles() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.handle_player_action(&ctx, PlayerAction::ToggleMicrophone);
        assert_eq!(sent_like(&app, &Command::ToggleMicrophone), 1);
    }

    #[test]
    fn the_soundboard_is_kept() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        assert!(app.soundboard().sounds.is_empty());
        let state = SoundboardState {
            sounds: vec![aria_core::soundboard::SoundView {
                id: aria_core::SoundId(1),
                name: "Horn".into(),
                volume: 0.5,
                hotkey: None,
                missing: false,
                playing: true,
            }],
        };
        app.handle_event(&ctx, Event::Soundboard(Arc::new(state.clone())));
        assert_eq!(*app.soundboard(), state);
    }

    fn opened_track(id: i64, title: &str) -> aria_library::Track {
        aria_library::Track {
            id: aria_library::TrackId(id),
            folder: aria_library::FolderId(0),
            path: format!("C:/Downloads/{title}.mp3").into(),
            title: title.to_owned(),
            artist: Some(Arc::from("Somebody")),
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 1000,
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: 0,
            codec: Arc::from("mp3"),
            artwork: None,
            added_at: 0,
        }
    }

    #[test]
    fn a_track_opened_from_outside_the_library_is_named_on_the_bar_and_in_the_queue() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        app.handle_event(
            &ctx,
            Event::Opened(Arc::from(vec![opened_track(-1, "Outside")])),
        );
        app.handle_event(
            &ctx,
            Event::Player(PlayerView {
                current: Some(aria_library::TrackId(-1)),
                ..PlayerView::default()
            }),
        );
        let now = NowPlaying::of(&app.player, app.playing_snapshot(), &app.localizer);
        let now = now.expect("a current track");
        assert_eq!(
            (now.title.as_str(), now.artist.as_str()),
            ("Outside", "Somebody")
        );
        // Not in the library's own lists.
        assert!(app.library.snapshot.tracks.is_empty());
    }

    fn space_harness() -> Harness<'static, AriaApp> {
        let mut app = app(vec![]);
        let track = aria_library::Track {
            id: aria_library::TrackId(1),
            folder: aria_library::FolderId(1),
            path: "music/1.flac".into(),
            title: "Song".to_owned(),
            artist: None,
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 1000,
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: 0,
            codec: Arc::from("flac"),
            artwork: None,
            added_at: 1,
        };
        app.library.snapshot = Arc::new(aria_library::LibrarySnapshot {
            tracks: vec![track],
            folders: vec![aria_library::Folder {
                id: aria_library::FolderId(1),
                path: "music".into(),
                available: true,
            }],
            ..aria_library::LibrarySnapshot::default()
        });
        app.history.navigate(Route::LocalMusic);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
        harness.run();
        harness
    }

    #[test]
    fn space_toggles_play() {
        let mut harness = space_harness();
        harness.key_press(egui::Key::Space);
        harness.run();
        assert_eq!(toggles_sent(&harness), 1);
    }

    #[test]
    fn space_in_search_field_types_a_space() {
        use egui_kittest::kittest::Queryable as _;
        let mut harness = space_harness();
        let english = Localizer::new(Language::En).unwrap();
        let label = english.tr("local-search");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
        field.focus();
        harness.run();
        harness.key_press(egui::Key::Space);
        harness.run();
        assert_eq!(toggles_sent(&harness), 0, "Space played while typing");
    }

    fn playlist(id: i64, name: &str) -> aria_library::Playlist {
        aria_library::Playlist {
            id: aria_library::PlaylistId(id),
            name: name.to_owned(),
            items: Vec::new(),
            unavailable: 0,
        }
    }

    /// How many of the commands sent read as `expected` does.
    fn sent_like(app: &AriaApp, expected: &Command) -> usize {
        let expected = format!("{expected:?}");
        app.sent
            .borrow()
            .iter()
            .filter(|command| format!("{command:?}") == expected)
            .count()
    }

    #[test]
    fn a_collections_event_is_kept() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let collections = Arc::new(aria_library::Collections::new(
            vec![aria_library::TrackId(3)],
            vec![playlist(1, "Road")],
            vec![aria_library::TrackId(2)],
        ));
        app.handle_event(&ctx, Event::Collections(Arc::clone(&collections)));
        assert_eq!(app.collections, collections);
    }

    #[test]
    fn page_actions_become_collection_commands() {
        use aria_library::{PlaylistId, TrackId};
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let (playlist, track) = (PlaylistId(4), TrackId(7));
        let pairs = [
            (
                PageAction::SetFavorite {
                    track,
                    favorite: true,
                },
                Command::SetFavorite {
                    track,
                    favorite: true,
                },
            ),
            (
                PageAction::AddToPlaylist { playlist, track },
                Command::AddToPlaylist { playlist, track },
            ),
            (
                PageAction::RemoveFromPlaylist {
                    playlist,
                    position: 2,
                },
                Command::RemoveFromPlaylist {
                    playlist,
                    position: 2,
                },
            ),
            (
                PageAction::MoveInPlaylist {
                    playlist,
                    from: 1,
                    to: 3,
                },
                Command::MoveInPlaylist {
                    playlist,
                    from: 1,
                    to: 3,
                },
            ),
            (
                PageAction::DeletePlaylist(playlist),
                Command::DeletePlaylist(playlist),
            ),
            (PageAction::ClearHistory, Command::ClearHistory),
        ];
        for (action, command) in pairs {
            app.handle_page_action(&ctx, action);
            assert_eq!(
                sent_like(&app, &command),
                1,
                "{command:?} was not sent once"
            );
        }
        assert_eq!(app.sent.borrow().len(), 6);
    }

    #[test]
    fn the_player_bar_heart_sets_the_favorite() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let track = aria_library::TrackId(9);
        app.handle_player_action(
            &ctx,
            PlayerAction::SetFavorite {
                track,
                favorite: false,
            },
        );
        let command = Command::SetFavorite {
            track,
            favorite: false,
        };
        assert_eq!(sent_like(&app, &command), 1);
    }

    fn dialog_harness(collections: aria_library::Collections) -> Harness<'static, AriaApp> {
        let mut app = app(vec![]);
        app.collections = Arc::new(collections);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
        harness.run();
        harness
    }

    fn english(key: &str) -> String {
        Localizer::new(Language::En).unwrap().tr(key).into_owned()
    }

    #[test]
    fn the_name_dialog_refuses_an_empty_name_and_creates_on_enter() {
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let mut harness = dialog_harness(aria_library::Collections::default());
        let track = aria_library::TrackId(5);
        let ctx = harness.ctx.clone();
        harness.state_mut().handle_page_action(
            &ctx,
            PageAction::AskPlaylistName(NameFor::New { first: Some(track) }),
        );
        harness.run();
        assert!(
            harness.state().modal_open(),
            "the dialog does not hold the app"
        );

        let create = english("playlist-create");
        let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, &create);
        assert!(
            button.accesskit_node().is_disabled(),
            "an empty name can be confirmed"
        );

        let label = english("playlist-name-label");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
        field.focus();
        harness.run();
        harness
            .get_by_role_and_label(egui::accesskit::Role::TextInput, &label)
            .type_text("Road");
        harness.run();
        let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, &create);
        assert!(
            !button.accesskit_node().is_disabled(),
            "a good name cannot be confirmed"
        );

        harness.key_press(egui::Key::Enter);
        harness.run();
        let command = Command::CreatePlaylist {
            name: "Road".to_owned(),
            first: Some(track),
        };
        assert_eq!(sent_like(harness.state(), &command), 1);
        assert!(
            harness.state().name_dialog.is_none(),
            "the dialog stayed open"
        );
        assert!(!harness.state().modal_open());
    }

    fn sound_view(id: i64, name: &str) -> aria_core::soundboard::SoundView {
        aria_core::soundboard::SoundView {
            id: aria_core::SoundId(id),
            name: name.into(),
            volume: 1.0,
            hotkey: None,
            missing: false,
            playing: false,
        }
    }

    #[test]
    fn rename_opens_the_name_dialog_with_the_current_name() {
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let mut harness = dialog_harness(aria_library::Collections::default());
        let ctx = harness.ctx.clone();
        let state = SoundboardState {
            sounds: vec![sound_view(3, "Airhorn")],
        };
        harness
            .state_mut()
            .handle_event(&ctx, Event::Soundboard(Arc::new(state)));
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskSoundName(aria_core::SoundId(3)));
        harness.run();
        assert!(harness.state().modal_open());

        let label = english("playlist-name-label");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
        assert_eq!(field.accesskit_node().value().as_deref(), Some("Airhorn"));

        // 50 characters are the most a sound's name can have.
        let confirm = english("sound-rename");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
        field.focus();
        harness.run();
        harness.state_mut().name_dialog =
            Some((NameFor::Sound(aria_core::SoundId(3)), "x".repeat(51)));
        harness.run();
        let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, &confirm);
        assert!(button.accesskit_node().is_disabled(), "51 characters");
        harness.state_mut().name_dialog =
            Some((NameFor::Sound(aria_core::SoundId(3)), "x".repeat(50)));
        harness.run();
        let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, &confirm);
        assert!(!button.accesskit_node().is_disabled(), "50 characters");
        button.click();
        harness.run();
        let command = Command::RenameSound {
            id: aria_core::SoundId(3),
            name: "x".repeat(50),
        };
        assert_eq!(sent_like(harness.state(), &command), 1);
        assert!(!harness.state().modal_open());
    }

    #[test]
    fn the_sound_actions_become_commands() {
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let id = aria_core::SoundId(4);
        for (action, command) in [
            (PageAction::PlaySound(id), Command::PlaySound(id)),
            (PageAction::StopSounds, Command::StopSounds),
            (PageAction::RemoveSound(id), Command::RemoveSound(id)),
            (
                PageAction::SetSoundVolume { id, volume: 0.25 },
                Command::SetSoundVolume { id, volume: 0.25 },
            ),
        ] {
            app.handle_page_action(&ctx, action);
            assert_eq!(sent_like(&app, &command), 1, "{command:?}");
        }
        // The hotkey dialogs send nothing until they are confirmed, and none opens for a sound
        // the soundboard does not have.
        let before = app.sent.borrow().len();
        app.handle_page_action(&ctx, PageAction::AskSoundHotkey(id));
        app.handle_page_action(&ctx, PageAction::AskStopHotkey);
        assert_eq!(app.sent.borrow().len(), before);
    }

    #[test]
    fn the_name_dialog_closes_on_escape() {
        let mut harness = dialog_harness(aria_library::Collections::default());
        let ctx = harness.ctx.clone();
        harness.state_mut().handle_page_action(
            &ctx,
            PageAction::AskPlaylistName(NameFor::New { first: None }),
        );
        harness.run();
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(harness.state().name_dialog.is_none());
        assert!(!harness.state().modal_open());
        // The first frame tells the backend the window is shown; nothing else goes.
        assert!(
            harness
                .state()
                .sent
                .borrow()
                .iter()
                .all(|c| matches!(c, Command::WindowShown(true))),
            "Escape sent something"
        );
    }

    #[test]
    fn the_name_dialog_renames() {
        use egui_kittest::kittest::Queryable as _;
        let id = aria_library::PlaylistId(5);
        let collections =
            aria_library::Collections::new(vec![], vec![playlist(5, "Old name")], vec![]);
        let mut harness = dialog_harness(collections);
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskPlaylistName(NameFor::Rename(id)));
        harness.run();

        let label = english("playlist-name-label");
        let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
        assert_eq!(field.value().as_deref(), Some("Old name"));

        if let Some((_, text)) = &mut harness.state_mut().name_dialog {
            *text = "  New name ".to_owned();
        }
        harness.run();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, &english("playlist-rename"))
            .click();
        harness.run();
        let command = Command::RenamePlaylist {
            id,
            name: "New name".to_owned(),
        };
        assert_eq!(sent_like(harness.state(), &command), 1);
        assert!(harness.state().name_dialog.is_none());
        assert!(!harness.state().modal_open());
    }

    #[test]
    fn space_is_ignored_with_a_modal_open() {
        let mut harness = space_harness();
        harness.state_mut().page_dialog_open = true;
        harness.key_press(egui::Key::Space);
        harness.run();
        assert_eq!(toggles_sent(&harness), 0, "Space played behind a dialog");
    }

    #[test]
    fn the_two_dialogs_hold_the_app_independently() {
        let mut harness = dialog_harness(aria_library::Collections::default());
        assert!(!harness.state().modal_open());
        let ctx = harness.ctx.clone();
        harness.state_mut().page_dialog_open = true;
        harness.state_mut().handle_page_action(
            &ctx,
            PageAction::AskPlaylistName(NameFor::New { first: None }),
        );
        harness.run();
        harness.state_mut().page_dialog_open = false;
        assert!(
            harness.state().modal_open(),
            "closing the Settings dialog released the name dialog's hold"
        );
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(!harness.state().modal_open());
    }

    // Global hotkeys, with a fake system behind them.

    fn key(text: &str) -> Hotkey {
        text.parse()
            .unwrap_or_else(|err| panic!("{text} is a hotkey: {err}"))
    }

    fn keyed(id: i64, name: &str, hotkey: &str) -> aria_core::soundboard::SoundView {
        aria_core::soundboard::SoundView {
            hotkey: Some(key(hotkey)),
            ..sound_view(id, name)
        }
    }

    /// The app on the Soundboard page with `sounds`, over a fake system the test keeps a handle
    /// to.
    fn hotkey_harness(
        sounds: Vec<aria_core::soundboard::SoundView>,
        fake: &crate::hotkeys::fake::Fake,
    ) -> Harness<'static, AriaApp> {
        let mut app = app(vec![]);
        app.hotkeys = Hotkeys::new(Box::new(fake.clone()));
        app.hotkeys_available = true;
        let ctx = egui::Context::default();
        app.handle_event(
            &ctx,
            Event::Soundboard(Arc::new(SoundboardState { sounds })),
        );
        app.history.navigate(Route::Soundboard);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 780.0))
            .build_ui_state(|ui, app: &mut AriaApp| app.show(ui), app);
        harness.run();
        harness
    }

    fn set_hotkey_sent(app: &AriaApp) -> Vec<Command> {
        app.sent
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::SetSoundHotkey { .. }))
            .cloned()
            .collect()
    }

    fn plays_sent(app: &AriaApp, id: i64) -> usize {
        sent_like(app, &Command::PlaySound(aria_core::SoundId(id)))
    }

    fn click(harness: &mut Harness<'_, AriaApp>, key: &str) {
        use egui_kittest::kittest::Queryable as _;
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, &english(key))
            .click();
        harness.run();
    }

    #[test]
    fn a_press_plays_its_sound_and_stop_all_stops_them() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![keyed(1, "Airhorn", "Ctrl+1")], &fake);
        let ctx = harness.ctx.clone();
        let mut settings = harness.state().settings.clone();
        settings.stop_sounds_hotkey = Some(key("F9"));
        harness.state_mut().update_settings(&ctx, settings);
        harness.run();
        let registered: std::collections::HashSet<Hotkey> = [key("Ctrl+1"), key("F9")].into();
        assert_eq!(fake.registered(), registered);

        fake.press(key("Ctrl+1"));
        fake.press(key("F9"));
        harness.run();
        assert_eq!(plays_sent(harness.state(), 1), 1);
        assert_eq!(sent_like(harness.state(), &Command::StopSounds), 1);
    }

    #[test]
    fn the_switch_off_unregisters_every_hotkey_and_keeps_them() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![keyed(1, "Airhorn", "Ctrl+1")], &fake);
        let ctx = harness.ctx.clone();
        let mut settings = harness.state().settings.clone();
        settings.global_hotkeys = false;
        harness.state_mut().update_settings(&ctx, settings.clone());
        harness.run();
        assert!(fake.registered().is_empty());
        fake.press(key("Ctrl+1"));
        harness.run();
        assert_eq!(plays_sent(harness.state(), 1), 0, "fired while off");
        assert!(
            set_hotkey_sent(harness.state()).is_empty(),
            "the hotkey was dropped"
        );

        settings.global_hotkeys = true;
        harness.state_mut().update_settings(&ctx, settings);
        harness.run();
        assert_eq!(fake.registered(), [key("Ctrl+1")].into());
    }

    #[test]
    fn a_hotkey_the_system_refuses_is_not_saved() {
        let fake = crate::hotkeys::fake::Fake::default();
        fake.take(key("Ctrl+Alt+K"));
        let mut harness = hotkey_harness(vec![sound_view(1, "Airhorn")], &fake);
        let ctx = harness.ctx.clone();
        let id = aria_core::SoundId(1);

        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskSoundHotkey(id));
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::K);
        harness.run();
        click(&mut harness, "settings-confirm");
        assert!(
            set_hotkey_sent(harness.state()).is_empty(),
            "a refused hotkey was saved"
        );
        let unavailable = harness
            .state()
            .toasts
            .notices()
            .filter(|notice| notice.key == "hotkey-unavailable")
            .count();
        assert_eq!(unavailable, 1);
        assert!(harness.state().hotkey_dialog.is_none());
        assert!(
            fake.registered().is_empty(),
            "the check left a hotkey behind"
        );

        // One the system accepts is saved, and is not left registered by the check.
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskSoundHotkey(id));
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::J);
        harness.run();
        click(&mut harness, "settings-confirm");
        let expected = Command::SetSoundHotkey {
            id,
            hotkey: Some(key("Ctrl+Alt+J")),
        };
        assert_eq!(sent_like(harness.state(), &expected), 1);
        assert_eq!(set_hotkey_sent(harness.state()).len(), 1);
        assert!(
            fake.registered().is_empty(),
            "registered before the soundboard has it"
        );
    }

    #[test]
    fn a_hotkey_another_sound_has_is_refused_before_anything_is_sent() {
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let fake = crate::hotkeys::fake::Fake::default();
        let sounds = vec![keyed(1, "Airhorn", "Ctrl+1"), sound_view(2, "Rimshot")];
        let mut harness = hotkey_harness(sounds, &fake);
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskSoundHotkey(aria_core::SoundId(2)));
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Num1);
        harness.run();
        harness.get_by_label(&english("hotkey-taken"));
        let label = english("settings-confirm");
        let confirm = harness.get_by_role_and_label(egui::accesskit::Role::Button, &label);
        assert!(confirm.accesskit_node().is_disabled());
        assert!(set_hotkey_sent(harness.state()).is_empty());
    }

    #[test]
    fn the_stop_all_hotkey_is_saved_in_the_settings_and_cleared_with_delete() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![], &fake);
        let ctx = harness.ctx.clone();
        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskStopHotkey);
        harness.run();
        harness.key_press(egui::Key::F9);
        harness.run();
        click(&mut harness, "settings-confirm");
        assert_eq!(harness.state().settings.stop_sounds_hotkey, Some(key("F9")));
        assert_eq!(fake.registered(), [key("F9")].into());

        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskStopHotkey);
        harness.run();
        harness.key_press(egui::Key::Delete);
        harness.run();
        assert_eq!(harness.state().settings.stop_sounds_hotkey, None);
        assert!(fake.registered().is_empty());
    }

    #[test]
    fn inactive_hotkeys_are_marked_on_their_tiles_and_counted_in_one_notice() {
        use egui_kittest::kittest::{NodeT as _, Queryable as _};
        let fake = crate::hotkeys::fake::Fake::default();
        fake.take(key("Ctrl+1"));
        fake.take(key("Ctrl+2"));
        let sounds = vec![
            keyed(1, "Airhorn", "Ctrl+1"),
            keyed(2, "Rimshot", "Ctrl+2"),
            keyed(3, "Gong", "Ctrl+3"),
        ];
        let mut harness = hotkey_harness(sounds.clone(), &fake);

        let hint = english("hotkey-inactive-hint");
        let description = |harness: &Harness<'_, AriaApp>, name: &str| {
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, name)
                .accesskit_node()
                .description()
        };
        assert_eq!(
            description(&harness, "Airhorn").as_deref(),
            Some(format!("Ctrl+1. {hint}").as_str())
        );
        assert_eq!(
            description(&harness, "Rimshot").as_deref(),
            Some(format!("Ctrl+2. {hint}").as_str())
        );
        assert_eq!(description(&harness, "Gong").as_deref(), Some("Ctrl+3"));

        let inactive = |harness: &Harness<'_, AriaApp>| -> Vec<Notice> {
            harness
                .state()
                .toasts
                .notices()
                .filter(|notice| notice.key == "hotkeys-inactive")
                .cloned()
                .collect()
        };
        let notices = inactive(&harness);
        assert_eq!(notices, [hotkeys_inactive(2)]);
        let text = Localizer::new(Language::En)
            .expect("the English locale loads")
            .tr_args(notices[0].key, &notices[0].fluent_args())
            .into_owned();
        assert!(text.starts_with("2 hotkeys don't work"), "{text}");

        // The same refusals again say nothing new.
        harness.state_mut().toasts = Toasts::new(player_bar::HEIGHT);
        let ctx = harness.ctx.clone();
        harness.state_mut().handle_event(
            &ctx,
            Event::Soundboard(Arc::new(SoundboardState { sounds })),
        );
        harness.run();
        assert!(inactive(&harness).is_empty(), "the notice was repeated");
        assert_eq!(fake.registered(), [key("Ctrl+3")].into());

        // One more refused replaces the notice on screen with one that counts three.
        harness.state_mut().notify(&ctx, hotkeys_inactive(2));
        fake.take(key("F9"));
        let mut settings = harness.state().settings.clone();
        settings.stop_sounds_hotkey = Some(key("F9"));
        harness.state_mut().update_settings(&ctx, settings);
        harness.run();
        assert_eq!(inactive(&harness), [hotkeys_inactive(3)]);
    }

    #[test]
    fn hotkeys_do_not_fire_while_the_hotkey_dialog_is_open() {
        let fake = crate::hotkeys::fake::Fake::default();
        let mut harness = hotkey_harness(vec![keyed(1, "Airhorn", "Ctrl+1")], &fake);
        let ctx = harness.ctx.clone();
        fake.press(key("Ctrl+1"));
        harness.run();
        assert_eq!(plays_sent(harness.state(), 1), 1);

        harness
            .state_mut()
            .handle_page_action(&ctx, PageAction::AskSoundHotkey(aria_core::SoundId(1)));
        harness.run();
        assert!(harness.state().modal_open());
        assert!(fake.registered().is_empty(), "registered behind the dialog");
        fake.press(key("Ctrl+1"));
        harness.run();
        assert_eq!(plays_sent(harness.state(), 1), 1, "fired behind the dialog");

        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(!harness.state().modal_open());
        assert_eq!(fake.registered(), [key("Ctrl+1")].into());
        fake.press(key("Ctrl+1"));
        harness.run();
        assert_eq!(plays_sent(harness.state(), 1), 2);
    }

    #[test]
    fn without_the_system_manager_every_hotkey_is_inactive() {
        // `AppDeps::none()` brings no manager.
        let mut app = app(vec![]);
        let ctx = egui::Context::default();
        let sounds = vec![keyed(1, "Airhorn", "Ctrl+1")];
        app.handle_event(
            &ctx,
            Event::Soundboard(Arc::new(SoundboardState { sounds })),
        );
        app.run_hotkeys(&ctx);
        assert_eq!(app.inactive_hotkeys, [key("Ctrl+1")]);
        assert!(!app.hotkeys.any_registered());
        // The notice does not blame other programs.
        let keys: Vec<&str> = app.toasts.notices().map(|notice| notice.key).collect();
        assert_eq!(keys, ["hotkeys-unavailable"]);
    }

    #[test]
    fn a_window_never_drawn_still_takes_in_events_and_syncs_its_hotkeys() {
        // Started minimized, eframe runs only `logic`: no frame is ever drawn here.
        let woke = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&woke);
        let waker: Waker = Arc::new(move || flag.store(true, Ordering::SeqCst));
        let services = BackendServices {
            spotify: None,
            library: None,
            output: Box::new(FakeFactory::new(OutputFormat {
                sample_rate: 48_000,
                channels: 2,
            })),
            capture: Box::new(
                FakeCapture::new(CaptureFormat {
                    sample_rate: 48_000,
                    channels: 1,
                })
                .0,
            ),
            settings: Settings::default(),
        };
        let backend = Backend::start(None, services, waker, BackendConfig::default())
            .expect("the backend starts");
        let deps = AppDeps {
            backend: Some(backend),
            ..AppDeps::none()
        };
        let settings = Settings {
            language: Some(Language::En),
            stop_sounds_hotkey: Some(key("F9")),
            ..Settings::default()
        };
        let localizer = Localizer::new(Language::En).expect("the English locale loads");
        let mut app = AriaApp::new(settings, localizer, deps, vec![]);
        let fake = crate::hotkeys::fake::Fake::default();
        app.hotkeys = Hotkeys::new(Box::new(fake.clone()));
        app.hotkeys_available = true;
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();

        eframe::App::logic(&mut app, &ctx, &mut frame);
        assert_eq!(fake.registered(), [key("F9")].into());

        // A new soundboard list: its hotkeys are registered, and fire, with nothing drawn.
        let sounds = vec![keyed(1, "Airhorn", "Ctrl+F1")];
        app.handle_event(
            &ctx,
            Event::Soundboard(Arc::new(SoundboardState { sounds })),
        );
        eframe::App::logic(&mut app, &ctx, &mut frame);
        assert_eq!(fake.registered(), [key("F9"), key("Ctrl+F1")].into());
        fake.press(key("Ctrl+F1"));
        eframe::App::logic(&mut app, &ctx, &mut frame);
        assert_eq!(plays_sent(&app, 1), 1);

        // The backend's events are taken in by `logic` too: settings it replaced (here the
        // defaults, after a reset) drop the hotkey of "Stop all".
        woke.store(false, Ordering::SeqCst);
        app.send(Command::ResetSettings);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !woke.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(woke.load(Ordering::SeqCst), "the backend did not answer");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.settings().stop_sounds_hotkey.is_some() && Instant::now() < deadline {
            eframe::App::logic(&mut app, &ctx, &mut frame);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.settings().stop_sounds_hotkey, None);
        assert_eq!(fake.registered(), [key("Ctrl+F1")].into());
    }

    #[test]
    fn a_press_fires_while_the_window_is_hidden() {
        let fake = crate::hotkeys::fake::Fake::default();
        // One frame registers the hotkey; after that the window is minimized and eframe runs
        // only `logic`.
        let harness = hotkey_harness(vec![keyed(1, "Airhorn", "Ctrl+1")], &fake);
        let ctx = harness.ctx.clone();
        let mut app = harness.into_state();
        assert_eq!(fake.registered(), [key("Ctrl+1")].into());

        fake.press(key("Ctrl+1"));
        let mut frame = eframe::Frame::_new_kittest();
        eframe::App::logic(&mut app, &ctx, &mut frame);
        assert_eq!(plays_sent(&app, 1), 1, "the press waited for a drawn frame");
        eframe::App::logic(&mut app, &ctx, &mut frame);
        assert_eq!(plays_sent(&app, 1), 1, "a press fired twice");
    }
}
