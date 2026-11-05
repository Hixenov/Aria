//! Logging setup, live level reload and the panic hook.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use tracing::warn;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{Builder, Rotation},
};
use tracing_subscriber::{
    EnvFilter, Registry, fmt, layer::SubscriberExt, reload, util::SubscriberInitExt,
};

use crate::{error::LoggingError, settings::LogLevel};

/// Name of the environment variable that overrides the configured level.
const ENV_VAR: &str = "ARIA_LOG";

/// Builds the filter directive: the non-empty env override, else Aria's crates
/// at `level` and everything else at `warn`.
pub fn filter_directive(level: LogLevel, env_override: Option<&str>) -> String {
    match env_override {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => {
            let lvl = level.as_filter();
            format!("warn,aria_core={lvl},aria={lvl}")
        }
    }
}

/// Keeps logging alive: dropping it flushes and stops the file writer.
///
/// `main` holds it to the very end, so errors logged after the window has closed still
/// reach the file. The app gets a [`LogLevelHandle`] from [`Self::level`] instead.
pub struct LoggingHandle {
    _guard: Option<WorkerGuard>,
    level: LogLevelHandle,
}

impl LoggingHandle {
    /// Changes the active log level; a no-op when `ARIA_LOG` was set at startup.
    pub fn set_level(&self, level: LogLevel) {
        self.level.set_level(level);
    }

    /// A handle that changes the log level live, without keeping the file writer alive.
    pub fn level(&self) -> LogLevelHandle {
        self.level.clone()
    }
}

/// Changes the log level of the running subscriber.
#[derive(Clone)]
pub struct LogLevelHandle {
    reload: reload::Handle<EnvFilter, Registry>,
    env_overridden: bool,
}

impl LogLevelHandle {
    /// Changes the active log level; a no-op when a valid `ARIA_LOG` was set at startup.
    pub fn set_level(&self, level: LogLevel) {
        if self.env_overridden {
            return;
        }
        match EnvFilter::try_new(filter_directive(level, None)) {
            Ok(filter) => {
                if let Err(err) = self.reload.reload(filter) {
                    warn!("could not change the log level: {err}");
                }
            }
            Err(err) => warn!("invalid log filter: {err}"),
        }
    }
}

/// The filter logging starts with.
struct StartFilter {
    filter: EnvFilter,
    /// Whether a valid `ARIA_LOG` chose it, so the settings' level must not replace it.
    env_overridden: bool,
    /// An `ARIA_LOG` value that did not parse, and why. The settings' level applies instead.
    rejected: Option<(String, String)>,
}

/// Picks the start filter: a non-empty `env_override` that parses, else Aria's crates at
/// `level`.
fn start_filter(level: LogLevel, env_override: Option<&str>) -> Result<StartFilter, LoggingError> {
    let from_level = || {
        EnvFilter::try_new(filter_directive(level, None))
            .map_err(|err| LoggingError::Init(err.to_string()))
    };
    let Some(value) = env_override.filter(|value| !value.is_empty()) else {
        return Ok(StartFilter {
            filter: from_level()?,
            env_overridden: false,
            rejected: None,
        });
    };
    Ok(match EnvFilter::try_new(value) {
        Ok(filter) => StartFilter {
            filter,
            env_overridden: true,
            rejected: None,
        },
        Err(err) => StartFilter {
            filter: from_level()?,
            env_overridden: false,
            rejected: Some((value.to_owned(), err.to_string())),
        },
    })
}

/// Installs the global subscriber. Logs to a daily-rotated file in `log_dir`
/// (and to stderr in debug builds), or to stderr only when `log_dir` is `None`.
///
/// An `ARIA_LOG` that does not parse is not an error: `level` applies as if it were unset,
/// and a warning says so once logging is up.
pub fn init(log_dir: Option<&Path>, level: LogLevel) -> Result<LoggingHandle, LoggingError> {
    init_with(log_dir, level, std::env::var(ENV_VAR).ok().as_deref())
}

/// [`init`], with the value of `ARIA_LOG` passed in.
fn init_with(
    log_dir: Option<&Path>,
    level: LogLevel,
    env_override: Option<&str>,
) -> Result<LoggingHandle, LoggingError> {
    let StartFilter {
        filter,
        env_overridden,
        rejected,
    } = start_filter(level, env_override)?;
    let (filter_layer, reload) = reload::Layer::new(filter);

    let (file_layer, guard) = match log_dir {
        Some(dir) => {
            let appender = Builder::new()
                .rotation(Rotation::DAILY)
                .filename_prefix("aria")
                .filename_suffix("log")
                .max_log_files(7)
                .build(dir)
                .map_err(|err| LoggingError::Init(err.to_string()))?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer().with_ansi(false).with_writer(writer);
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    let stderr_layer = (log_dir.is_none() || cfg!(debug_assertions))
        .then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(|err| LoggingError::Init(err.to_string()))?;

    if let Some((value, error)) = rejected {
        warn!(
            %value,
            %error,
            "{ENV_VAR} is not a valid log filter; using the log level from the settings"
        );
    }
    Ok(LoggingHandle {
        _guard: guard,
        level: LogLevelHandle {
            reload,
            env_overridden,
        },
    })
}

/// Formats the single human-readable line written for a panic.
pub(crate) fn format_panic_line(thread: &str, msg: &str, location: &str) -> String {
    format!("panic in thread '{thread}' at {location}: {msg}")
}

/// Synchronously appends `line` to `dir/panic.log`, creating `dir` if needed.
pub(crate) fn append_panic_line(dir: &Path, line: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("panic.log"))?;
    writeln!(file, "{line}")?;
    file.flush()
}

/// Logs panics through `tracing` and, when `log_dir` is given, also appends them
/// synchronously to `panic.log` (the non-blocking writer cannot be flushed from
/// inside a hook). The previous hook runs afterwards.
pub fn install_panic_hook(log_dir: Option<PathBuf>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let location = info.location().map_or_else(
            || "<unknown>".to_owned(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let line = format_panic_line(thread_name, msg, &location);
        tracing::error!("{line}");
        if let Some(dir) = &log_dir {
            // A failure to write the panic log must not mask the original panic.
            let _ = append_panic_line(dir, &line);
        }
        previous(info);
    }));
}

/// Logs the application version and platform once at startup.
pub fn log_startup_banner() {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "Aria starting"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LogLevel;

    #[test]
    fn directive_uses_level_when_no_env() {
        assert_eq!(
            filter_directive(LogLevel::Debug, None),
            "warn,aria_core=debug,aria=debug"
        );
    }

    #[test]
    fn directive_prefers_env() {
        assert_eq!(filter_directive(LogLevel::Info, Some("trace")), "trace");
        assert_eq!(
            filter_directive(LogLevel::Info, Some("")),
            "warn,aria_core=info,aria=info"
        );
    }

    /// The filter built from `level` alone, as text.
    fn level_filter(level: LogLevel) -> String {
        EnvFilter::try_new(filter_directive(level, None))
            .unwrap()
            .to_string()
    }

    #[test]
    fn valid_env_filter_overrides_the_level() {
        let start = start_filter(LogLevel::Info, Some("aria=trace")).unwrap();
        assert!(start.env_overridden);
        assert!(start.rejected.is_none());
        assert_eq!(start.filter.to_string(), "aria=trace");
    }

    #[test]
    fn invalid_env_filter_falls_back_to_the_level() {
        let start = start_filter(LogLevel::Debug, Some("aria=loud")).unwrap();
        assert!(
            !start.env_overridden,
            "a rejected ARIA_LOG must not pin the level"
        );
        let (value, _) = start.rejected.expect("the rejected value is reported");
        assert_eq!(value, "aria=loud");
        assert_eq!(start.filter.to_string(), level_filter(LogLevel::Debug));
    }

    #[test]
    fn empty_env_filter_counts_as_unset() {
        let start = start_filter(LogLevel::Warn, Some("")).unwrap();
        assert!(!start.env_overridden);
        assert!(start.rejected.is_none());
        assert_eq!(start.filter.to_string(), level_filter(LogLevel::Warn));
    }

    // The only test in this crate that installs the global subscriber, which can happen
    // once per process.
    #[test]
    fn init_survives_an_invalid_env_filter() {
        let handle = init_with(None, LogLevel::Info, Some("aria=loud"))
            .expect("an invalid ARIA_LOG must not stop logging");
        assert!(!handle.level().env_overridden);
    }

    #[test]
    fn panic_line_is_written_to_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        let line = format_panic_line("worker", "boom", "src/x.rs:1:2");
        assert_eq!(line, "panic in thread 'worker' at src/x.rs:1:2: boom");
        append_panic_line(&logs, &line).expect("append");
        append_panic_line(&logs, "second").expect("append again");
        let text = std::fs::read_to_string(logs.join("panic.log")).expect("read");
        assert!(text.contains("boom"));
        assert_eq!(text.lines().count(), 2);
    }
}
