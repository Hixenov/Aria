//! Logging at the Trace level. In a test binary of its own, since the subscriber is global.

use aria_core::logging;
use aria_core::settings::LogLevel;

#[test]
fn trace_never_reaches_crates_that_log_on_the_audio_thread() {
    // ARIA_LOG would replace the level, so it must not be set here.
    if std::env::var_os("ARIA_LOG").is_some_and(|value| !value.is_empty()) {
        return;
    }
    let _handle = logging::init(None, LogLevel::Trace).expect("logging starts");
    // `wasapi` logs through the `log` crate on every period of the output; at Trace each of
    // those calls would reach the subscriber, and its lock, before the filter drops it.
    assert!(
        !log::log_enabled!(target: "wasapi::api", log::Level::Trace),
        "a trace line of the log crate reaches the subscriber"
    );
    assert!(log::max_level() <= log::LevelFilter::Debug);
    // Aria's own trace lines still do.
    assert!(tracing::enabled!(target: "aria_core", tracing::Level::TRACE));
}
