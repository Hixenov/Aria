//! Core domain logic for Aria.

pub mod command;
pub mod error;
pub mod event;
pub mod i18n;
pub mod library_service;
pub mod logging;
pub mod notice;
pub mod paths;
pub mod player;
pub mod redact;
pub mod runtime;
pub mod settings;

// What `aria-app` needs from the crates below it, which it reaches only through this one.
pub use aria_audio::{CpalFactory, DeviceId, OutputFactory, OutputFormat, fake::FakeFactory};
pub use aria_library::{Folder, FolderId, LibrarySnapshot, ScanProgress};
