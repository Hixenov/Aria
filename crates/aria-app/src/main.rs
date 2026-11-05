//! Aria desktop music player entry point: the startup sequence.
//!
//! Each stage is timed and logged at info as `startup stage=<name> ms=<n>`, the baseline
//! for cold-start measurement (spec §7). The first frame then logs `stage=first_frame` with
//! the time since `main` began. A stage that fails leaves the app running with less: no
//! folders means read-only settings, no logging means no log file, no backend means nothing
//! is saved. Only English translations failing to load, or the window failing to open,
//! stop it.

use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow};
use aria_app::{
    APP_NAME,
    app::{AppDeps, AriaApp},
};
use aria_core::{
    error::I18nError,
    i18n::{Localizer, os_language},
    logging,
    notice::{Notice, NoticeLevel},
    paths::AppPaths,
    runtime::{Backend, BackendConfig, SettingsSink, Waker},
    settings::{Language, Settings, store::SettingsStore},
};
use eframe::egui;

/// Shown when settings cannot be saved this session.
const READ_ONLY: Notice = Notice {
    level: NoticeLevel::Warning,
    key: "notice-settings-readonly",
};

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
    let settings = match &store {
        Some(store) => {
            let loaded = store.load();
            if let Some(notice) = loaded.notice {
                push_once(&mut notices, notice);
            }
            loaded.settings
        }
        None => Settings::default(),
    };
    if let Some(logging) = &logging {
        logging.set_level(settings.log_level);
    }
    log_stage("settings", stage.elapsed());

    // 4. Translations.
    let stage = Instant::now();
    let language = settings.language.unwrap_or_else(os_language);
    let localizer = localizer_for(language, Localizer::new).inspect_err(|err| {
        tracing::error!("{err:#}");
        eprintln!("Aria: {err:#}");
    })?;
    log_stage("i18n", stage.elapsed());

    // 5. Backend. Its waker repaints once the window, and with it the context, exists.
    let stage = Instant::now();
    let egui_ctx: Arc<OnceLock<egui::Context>> = Arc::default();
    let sink = store.map(|store| Box::new(store) as Box<dyn SettingsSink>);
    let backend = match Backend::start(
        sink,
        repaint_waker(Arc::clone(&egui_ctx)),
        BackendConfig::default(),
    ) {
        Ok(backend) => Some(backend),
        Err(err) => {
            tracing::error!(%err, "could not start the backend; settings are read-only");
            push_once(&mut notices, READ_ONLY);
            None
        }
    };
    log_stage("backend", stage.elapsed());

    // 6. The window. The app owns the backend from here and stops it in `on_exit`.
    let deps = AppDeps {
        backend,
        logging,
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
    if let Err(err) = result {
        tracing::error!(%err, "could not open the window");
        eprintln!("Aria: could not open the window: {err}");
        // When the app was never created, `on_exit` cannot run. Dropping the backend's
        // handle stops it as a shutdown would, and logging goes last so the error above is
        // written.
        drop(parts);
        return Err(anyhow!("could not open the window: {err}"));
    }
    Ok(())
}

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

/// `elapsed` in whole milliseconds, saturating.
fn whole_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
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
    fn whole_ms_truncates_and_saturates() {
        assert_eq!(whole_ms(Duration::from_micros(1999)), 1);
        assert_eq!(whole_ms(Duration::MAX), u64::MAX);
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
