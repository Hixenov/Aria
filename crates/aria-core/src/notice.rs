//! User-visible notices raised by core logic.

/// Severity of a [`Notice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Informational.
    Info,
    /// Something went wrong but the app recovered.
    Warning,
    /// A failure the user should know about.
    Error,
}

/// A notice to show the user; `key` is a localization key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Severity.
    pub level: NoticeLevel,
    /// Localization key of the message.
    pub key: &'static str,
}
