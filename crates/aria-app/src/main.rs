//! Aria desktop music player entry point: the startup sequence.
//!
//! Each stage is timed and logged at info as `startup stage=<name> ms=<n>`, the baseline
//! for cold-start measurement (spec §7). The first frame then logs `stage=first_frame` with
//! the time since `main` began. A stage that fails leaves the app running with less: no
//! folders means read-only settings and no library, no logging means no log file, no backend
//! means nothing is saved and no library. Only English translations failing to load, or the
//! window failing to open, stop it: both are logged, written to stderr and, on Windows, shown
//! in a message box.
//!
//! Release builds on Windows use the GUI subsystem, so no console window opens and stderr
//! goes nowhere. Their diagnostics are the log files, `panic.log` and that message box.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow};
use aria_app::{
    APP_NAME,
    app::{AppDeps, AriaApp},
    whole_ms,
};
use aria_core::{
    CpalFactory,
    error::I18nError,
    i18n::{Localizer, os_language},
    logging::{self, LoggingHandle},
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    runtime::{Backend, BackendConfig, BackendServices, LibraryPaths, SettingsSink, Waker},
    settings::{Language, Settings, store::SettingsStore},
};
use eframe::egui;

/// Shown when settings cannot be saved this session.
const READ_ONLY: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "notice-settings-readonly",
    args: Vec::new(),
};

/// Shown when the library cannot run at all this session.
const NO_LIBRARY: Notice = Notice {
    level: NoticeLevel::Error,
    key: "library-unavailable",
    args: Vec::new(),
};

/// Title of the message box shown when the English translations fail to load. Every other
/// text comes from the locale files, but this failure means no localizer exists to ask, and
/// English is what the localizer itself falls back to. So this text is fixed, in English.
const NO_TRANSLATIONS_TITLE: &str = "Aria couldn't start";
/// Body of that message box; the error follows it.
const NO_TRANSLATIONS_BODY: &str = "Aria couldn't load its translations.";

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let mut notices = Vec::new();

    // 1. Paths. Logging needs the log folder, so this stage's timing is logged once
    // logging is up.
    let stage = Instant::now();
    let paths = AppPaths::resolve().and_then(|paths| paths.ensure_dirs().map(|()| paths));
    let paths_elapsed = stage.elapsed();

    // 2. Logging, at the default level until the settings are read.
    let stage = Instant::now();
    let log_dir = paths.as_ref().ok().map(|paths| paths.log_dir.clone());
    let logging = match logging::init(log_dir.as_deref(), Settings::default().log_level) {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("Aria: logging is off: {err}");
            None
        }
    };
    logging::install_panic_hook(log_dir);
    logging::log_startup_banner();
    log_stage("paths", paths_elapsed);
    let paths = match paths {
        Ok(paths) => Some(paths),
        Err(err) => {
            tracing::error!(%err, "Aria's folders are unavailable; settings are read-only");
            push_once(&mut notices, READ_ONLY);
            None
        }
    };
    log_stage("logging", stage.elapsed());

    // 3. Settings.
    let stage = Instant::now();
    let store = paths
        .as_ref()
        .map(|paths| SettingsStore::new(paths.settings_file()));
    let (settings, store) = load_settings(store, &mut notices);
    if let Some(logging) = &logging {
        logging.set_level(settings.log_level);
    }
    log_stage("settings", stage.elapsed());

    // 4. Translations.
    let stage = Instant::now();
    let language = settings.language.unwrap_or_else(os_language);
    let localizer = match localizer_for(language, Localizer::new) {
        Ok(localizer) => localizer,
        Err(err) => {
            tracing::error!("{err:#}");
            eprintln!("Aria: {err:#}");
            show_fatal_error(
                NO_TRANSLATIONS_TITLE,
                &format!("{NO_TRANSLATIONS_BODY}\n\n{err:#}"),
            );
            return Err(err);
        }
    };
    // Translated now: once the app owns the localizer, it may be gone when this is needed.
    let window_failed_title = localizer.tr("error-startup-title").into_owned();
    let window_failed_body = localizer.tr("error-startup-window").into_owned();
    log_stage("i18n", stage.elapsed());

    // 5. Backend. Its waker repaints once the window, and with it the context, exists.
    let stage = Instant::now();
    let egui_ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let sink = store.map(|store| Box::new(store) as Box<dyn SettingsSink>);
    let library = paths.as_ref().map(library_paths);
    if library.is_none() {
        push_once(&mut notices, NO_LIBRARY);
    }
    let services = BackendServices {
        library,
        output: Box::new(CpalFactory),
        settings: settings.clone().sanitized(),
    };
    let backend = match Backend::start(
        sink,
        services,
        repaint_waker(Arc::clone(&egui_ctx)),
        BackendConfig::default(),
    ) {
        Ok(backend) => Some(backend),
        Err(err) => {
            tracing::error!(%err, "could not start the backend; settings are read-only");
            push_once(&mut notices, READ_ONLY);
            push_once(&mut notices, NO_LIBRARY);
            None
        }
    };
    log_stage("backend", stage.elapsed());

    // 6. The window. The app owns the backend from here and stops it in `on_exit`. `main`
    // keeps `logging` itself, so what is logged after the window closes still reaches the
    // file.
    let deps = AppDeps {
        backend,
        logging: logging.as_ref().map(LoggingHandle::level),
        paths,
    };
    let mut parts = Some((settings, localizer, deps, notices));
    let stage = Instant::now();
    tracing::info!(renderer = "glow", "opening the window");
    let result = eframe::run_native(
        APP_NAME,
        native_options(),
        Box::new(|cc| {
            log_stage("window", stage.elapsed());
            if egui_ctx.set(cc.egui_ctx.clone()).is_err() {
                tracing::warn!("the egui context was already set");
            }
            let (settings, localizer, deps, notices) =
                parts.take().ok_or("the app was already created")?;
            let app =
                AriaApp::new(settings, localizer, deps, notices).with_startup_instant(started);
            Ok(Box::new(app))
        }),
    );
    let outcome = match result {
        Ok(()) => Ok(()),
        Err(err) => {
            tracing::error!(%err, "could not open the window");
            eprintln!("Aria: could not open the window: {err}");
            // When the app was never created, `on_exit` cannot run. Dropping the backend's
            // handle stops it as a shutdown would.
            drop(parts);
            show_fatal_error(
                &window_failed_title,
                &format!("{window_failed_body}\n\n{err}"),
            );
            Err(anyhow!("could not open the window: {err}"))
        }
    };
    // Last, so everything above is flushed to the log file.
    drop(logging);
    outcome
}

/// Loads the settings from `store`, if there is one. The store comes back only when saving
/// may replace its file: a file that could not be read, or was unusable and could not be
/// backed up, is the user's only copy, so it is left alone and settings are read-only.
fn load_settings(
    store: Option<SettingsStore>,
    notices: &mut Vec<Notice>,
) -> (Settings, Option<SettingsStore>) {
    let Some(store) = store else {
        return (Settings::default(), None);
    };
    let loaded = store.load();
    if let Some(notice) = loaded.notice {
        push_once(notices, notice);
    }
    if !loaded.writable {
        tracing::warn!("the settings file is left untouched; settings are read-only");
        push_once(notices, READ_ONLY);
        return (loaded.settings, None);
    }
    (loaded.settings, Some(store))
}

/// Where the library keeps its database and its artwork cache.
fn library_paths(paths: &AppPaths) -> LibraryPaths {
    LibraryPaths {
        db: paths.data_dir.join("library.db"),
        artwork_dir: paths.cache_dir.join("artwork"),
    }
}

/// Shows a fatal startup error in a blocking message box. A release build has no console on
/// Windows, so without it the user would see nothing at all.
#[cfg(windows)]
fn show_fatal_error(title: &str, body: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Elsewhere the error reaches the user through stderr and the log only.
#[cfg(not(windows))]
fn show_fatal_error(_title: &str, _body: &str) {}

/// The window: titled Aria, 1200×780 (at least 900×600), drawn with glow, and remembered
/// between runs.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([900.0, 600.0]),
        persist_window: true,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}

/// Logs how long a startup stage took.
fn log_stage(stage: &'static str, elapsed: Duration) {
    tracing::info!(stage = %stage, ms = whole_ms(elapsed), "startup");
}

/// Adds `notice` unless the same one is already waiting.
fn push_once(notices: &mut Vec<Notice>, notice: Notice) {
    if !notices.contains(&notice) {
        notices.push(notice);
    }
}

/// Builds the localizer for `language` with `build`, falling back to English. Only English
/// failing is an error.
fn localizer_for(
    language: Language,
    build: impl Fn(Language) -> Result<Localizer, I18nError>,
) -> anyhow::Result<Localizer> {
    match build(language) {
        Ok(localizer) => Ok(localizer),
        Err(err) if language != Language::En => {
            tracing::warn!(%err, ?language, "falling back to English");
            build(Language::En).context("cannot load the English translations")
        }
        Err(err) => Err(err).context("cannot load the English translations"),
    }
}

/// A waker that repaints the window once `ctx` holds its context, and does nothing before.
fn repaint_waker(ctx: Arc<OnceLock<egui::Context>>) -> Waker {
    Arc::new(move || {
        if let Some(ctx) = ctx.get() {
            ctx.request_repaint();
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn failed(lang: &'static str) -> I18nError {
        I18nError::Parse {
            lang,
            detail: "broken".into(),
        }
    }

    #[test]
    fn notices_are_not_repeated() {
        let recovered = Notice {
            level: NoticeLevel::Warning,
            key: "notice-settings-recovered",
            args: Vec::new(),
        };
        let mut notices = Vec::new();
        push_once(&mut notices, READ_ONLY);
        push_once(&mut notices, recovered.clone());
        push_once(&mut notices, READ_ONLY);
        assert_eq!(notices, [READ_ONLY, recovered]);
    }

    #[test]
    fn a_broken_language_falls_back_to_english() {
        let localizer = localizer_for(Language::Ru, |lang| match lang {
            Language::Ru => Err(failed("ru")),
            lang => Localizer::new(lang),
        })
        .unwrap();
        assert_eq!(localizer.language(), Language::En);
    }

    #[test]
    fn working_language_is_used() {
        let localizer = localizer_for(Language::Hy, Localizer::new).unwrap();
        assert_eq!(localizer.language(), Language::Hy);
    }

    #[test]
    fn broken_english_is_fatal() {
        let tried = AtomicUsize::new(0);
        let result = localizer_for(Language::Ru, |lang| {
            tried.fetch_add(1, Ordering::SeqCst);
            Err(failed(lang.code()))
        });
        assert!(result.is_err());
        assert_eq!(tried.load(Ordering::SeqCst), 2, "Russian, then English");
        assert!(localizer_for(Language::En, |_| Err(failed("en"))).is_err());
    }

    #[test]
    fn waker_repaints_only_once_the_context_exists() {
        let cell: Arc<OnceLock<egui::Context>> = Arc::default();
        let waker = repaint_waker(Arc::clone(&cell));
        // Before the window exists there is nothing to wake.
        waker();

        let ctx = egui::Context::default();
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        cell.set(ctx).unwrap();
        waker();
        assert_eq!(repaints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn readable_settings_keep_their_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.toml"));
        let mut notices = Vec::new();
        let (settings, store) = load_settings(Some(store), &mut notices);
        assert_eq!(settings, Settings::default());
        assert!(store.is_some(), "a missing file may be written");
        assert!(notices.is_empty());
    }

    #[test]
    fn unwritable_settings_lose_their_store() {
        // A directory where the file should be cannot be read, so it must not be replaced.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::create_dir(&path).unwrap();
        let mut notices = Vec::new();
        let (settings, store) = load_settings(Some(SettingsStore::new(path)), &mut notices);
        assert_eq!(settings, Settings::default());
        assert!(store.is_none(), "the backend must get no sink");
        assert_eq!(notices, [READ_ONLY]);
    }

    #[test]
    fn no_store_means_defaults() {
        let mut notices = Vec::new();
        let (settings, store) = load_settings(None, &mut notices);
        assert_eq!(settings, Settings::default());
        assert!(store.is_none());
        assert!(notices.is_empty());
    }

    #[test]
    fn the_library_lives_in_the_data_and_cache_folders() {
        let paths = AppPaths {
            config_dir: "config".into(),
            data_dir: "data".into(),
            cache_dir: "cache".into(),
            log_dir: "logs".into(),
        };
        let library = library_paths(&paths);
        assert_eq!(library.db, std::path::Path::new("data").join("library.db"));
        assert_eq!(
            library.artwork_dir,
            std::path::Path::new("cache").join("artwork")
        );
    }

    #[test]
    fn window_options() {
        let options = native_options();
        assert_eq!(options.viewport.title.as_deref(), Some(APP_NAME));
        assert_eq!(options.viewport.inner_size, Some(egui::vec2(1200.0, 780.0)));
        assert_eq!(
            options.viewport.min_inner_size,
            Some(egui::vec2(900.0, 600.0))
        );
        assert!(options.persist_window);
        assert_eq!(options.renderer, eframe::Renderer::Glow);
    }
}
