//! Application state and the per-frame shell: player bar, sidebar, right panel and page.

use std::{mem, time::Duration};

use aria_core::{
    command::Command,
    event::Event,
    i18n::{Localizer, os_language},
    logging::LoggingHandle,
    notice::Notice,
    paths::AppPaths,
    runtime::BackendHandle,
    settings::{SIDEBAR_MAX, SIDEBAR_MIN, Settings},
};
use eframe::egui::{
    self, CentralPanel, Frame, InputState, Key, KeyboardShortcut, Margin, Modifiers, PointerButton,
    UiBuilder, accesskit::Role,
};

use crate::{
    nav::{History, Route},
    ui::{
        layout::{
            player_bar, right_panel,
            sidebar::{self, SidebarAction},
        },
        pages::{self, PageAction, PageCtx},
        theme::{
            Palette, fonts,
            visuals::{self, SPACE},
        },
    },
};

/// How long [`eframe::App::on_exit`] waits for the backend to save and stop.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Ctrl+Alt+R (Cmd+Alt+R on macOS) toggles the right panel.
const TOGGLE_RIGHT_PANEL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::R);
/// Alt+Left steps back in history.
const BACK: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft);
/// Alt+Right steps forward in history.
const FORWARD: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight);
/// With Ctrl (Cmd on macOS), these jump to the matching entry of [`Route::ALL`].
const ROUTE_KEYS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

/// The handles the shell drives. Each is optional, so tests, and a start-up that lost one,
/// still run.
#[derive(Default)]
pub struct AppDeps {
    /// The backend thread; without it nothing is saved.
    pub backend: Option<BackendHandle>,
    /// Changes the log level live.
    pub logging: Option<LoggingHandle>,
    /// Aria's directories, when they could be resolved.
    pub paths: Option<AppPaths>,
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
    /// Notices waiting to be shown, oldest first.
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
}

/// What the keyboard and mouse asked for this frame.
#[derive(Default)]
struct Shortcuts {
    jump: Option<Route>,
    back: bool,
    forward: bool,
    toggle_right_panel: bool,
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
        }
    }

    /// The route on screen.
    pub fn current_route(&self) -> Route {
        self.history.current()
    }

    /// The settings in effect.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Notices waiting to be shown, oldest first.
    pub fn pending_notices(&self) -> &[Notice] {
        &self.pending_notices
    }

    /// Draws one whole frame into the root `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.prepare_style(ui);
        let ctx = ui.ctx().clone();
        self.drain_events(&ctx);
        self.handle_shortcuts(&ctx);

        let palette = Palette::for_theme(ctx.theme(), &self.settings);
        // The player bar goes first so it spans the full width of the window.
        if player_bar::show(
            ui,
            &self.localizer,
            &palette,
            self.settings.right_panel_open,
        ) {
            self.toggle_right_panel(&ctx);
        }
        self.show_sidebar(ui, &ctx, &palette);
        right_panel::show(
            ui,
            &self.localizer,
            &palette,
            self.settings.right_panel_open,
        );
        self.show_page(ui, &ctx, &palette);
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
            match event {
                Event::Notice(notice) => self.pending_notices.push(notice),
                Event::SettingsReplaced(settings) => self.apply_settings(ctx, settings),
                Event::SettingsSaved => tracing::trace!("settings saved"),
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
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
            out: &mut actions,
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
                self.send(Command::ResetSettings);
            }
            // No page offers these yet: opening folders arrives with the settings page.
            PageAction::OpenConfigDir | PageAction::OpenLogDir => {
                tracing::debug!(?action, "opening folders is not available yet");
            }
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

/// Reads and consumes this frame's shortcuts, so a focused text field doesn't act on them
/// too.
fn read_shortcuts(input: &mut InputState) -> Shortcuts {
    let mut shortcuts = Shortcuts {
        toggle_right_panel: input.consume_shortcut(&TOGGLE_RIGHT_PANEL),
        ..Shortcuts::default()
    };
    for (route, key) in Route::ALL.into_iter().zip(ROUTE_KEYS) {
        if consume_ctrl_digit(input, key) {
            shortcuts.jump = Some(route);
        }
    }
    shortcuts.back =
        input.consume_shortcut(&BACK) | input.pointer.button_pressed(PointerButton::Extra1);
    shortcuts.forward =
        input.consume_shortcut(&FORWARD) | input.pointer.button_pressed(PointerButton::Extra2);
    shortcuts
}

/// Consumes Ctrl+`digit` (Cmd on macOS) pressed with no other modifier.
///
/// `consume_shortcut` ignores extra Alt and Shift, but Windows reports AltGr as Ctrl+Alt,
/// and AltGr+digit types a character on many keyboard layouts.
fn consume_ctrl_digit(input: &mut InputState, digit: Key) -> bool {
    let exact = input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == digit && modifiers.matches_exact(Modifiers::COMMAND)
        )
    });
    exact && input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, digit))
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
    use aria_core::{notice::NoticeLevel, settings::Language};

    use super::*;

    fn app(notices: Vec<Notice>) -> AriaApp {
        let settings = Settings {
            language: Some(Language::En),
            ..Settings::default()
        };
        let localizer = Localizer::new(Language::En).unwrap();
        AriaApp::new(settings, localizer, AppDeps::none(), notices)
    }

    #[test]
    fn startup_notices_wait_to_be_shown() {
        let notice = Notice {
            level: NoticeLevel::Warning,
            key: "notice-settings-recovered",
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

    #[test]
    fn plain_ctrl_digit_is_consumed() {
        let mut input = InputState::default();
        input.events.push(key_event(Key::Num3, Modifiers::COMMAND));
        assert!(consume_ctrl_digit(&mut input, Key::Num3));
        assert!(input.events.is_empty(), "the shortcut was not consumed");
    }

    #[test]
    fn altgr_digit_is_left_alone() {
        let mut input = InputState::default();
        let altgr = Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND;
        input.events.push(key_event(Key::Num3, altgr));
        assert!(!consume_ctrl_digit(&mut input, Key::Num3));
        assert_eq!(input.events.len(), 1);
    }

    fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
}
