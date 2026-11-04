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
