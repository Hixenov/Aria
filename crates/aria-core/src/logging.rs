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
pub struct LoggingHandle {
    _guard: Option<WorkerGuard>,
    reload: reload::Handle<EnvFilter, Registry>,
    env_overridden: bool,
}

impl LoggingHandle {
    /// Changes the active log level; a no-op when `ARIA_LOG` was set at startup.
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

/// Installs the global subscriber. Logs to a daily-rotated file in `log_dir`
/// (and to stderr in debug builds), or to stderr only when `log_dir` is `None`.
pub fn init(log_dir: Option<&Path>, level: LogLevel) -> Result<LoggingHandle, LoggingError> {
    let env = std::env::var(ENV_VAR).ok();
    let env_overridden = env.as_deref().is_some_and(|v| !v.is_empty());
    let directive = filter_directive(level, env.as_deref());
    let filter =
        EnvFilter::try_new(&directive).map_err(|err| LoggingError::Init(err.to_string()))?;
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

    Ok(LoggingHandle {
        _guard: guard,
        reload,
        env_overridden,
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
