//! Error types for aria-core.

use std::{io, path::PathBuf};

/// Errors raised while resolving or creating application directories.
#[derive(thiserror::Error, Debug)]
pub enum PathError {
    /// No home directory could be determined for the current user.
    #[error("could not determine the user's home directory")]
    NoHome,
    /// An I/O operation on a directory failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path the operation was applied to.
        path: PathBuf,
        /// The underlying I/O error.
        source: io::Error,
    },
}

/// Errors raised while loading or saving settings.
#[derive(thiserror::Error, Debug)]
pub enum SettingsError {
    /// An I/O operation failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path the operation was applied to.
        path: PathBuf,
        /// The underlying I/O error.
        source: io::Error,
    },
    /// The settings file could not be parsed.
    #[error("settings parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// The settings could not be serialized.
    #[error("settings serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// The settings file has a schema version newer than supported.
    #[error("unsupported settings schema version {0}")]
    UnsupportedVersion(u32),
}

/// Errors raised while building localization bundles.
#[derive(thiserror::Error, Debug)]
pub enum I18nError {
    /// A locale's Fluent source has syntax errors or defines an id twice.
    #[error("cannot load the {lang} translations: {detail}")]
    Parse {
        /// Code of the language that failed, e.g. `ru`.
        lang: &'static str,
        /// The parser or bundle errors.
        detail: String,
    },
}

/// Errors raised while setting up logging.
#[derive(thiserror::Error, Debug)]
pub enum LoggingError {
    /// The logging subscriber or its file appender could not be initialised.
    #[error("logging initialisation failed: {0}")]
    Init(String),
    /// An I/O operation failed.
    #[error("logging I/O error: {0}")]
    Io(#[from] io::Error),
}
