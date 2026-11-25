//! Aria's update checker, without Aria: reading GitHub's answer about the latest release, comparing
//! versions, reading `SHA256SUMS.txt`, telling an installed Aria from any other, and a small HTTP
//! client that never fetches more than it was told to.
//!
//! Nothing here runs on its own: the update service in `aria-core` decides when to ask.

mod client;
mod release;

pub use client::Client;
pub use release::{
    Asset, InstallKind, Release, install_kind, installer_name, is_newer, parse_release,
    sha256_file, sha256_for,
};
pub use semver::Version;

/// Where Aria asks for releases. Tests point both at a local fake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// The latest full release (no drafts, no pre-releases).
    pub latest: String,
    /// Every file Aria downloads must start with this.
    pub download_prefix: String,
}

impl Endpoints {
    /// GitHub, for Aria's own repository.
    pub fn github() -> Endpoints {
        Endpoints {
            latest: "https://api.github.com/repos/Hixenov/Aria/releases/latest".to_owned(),
            download_prefix: "https://github.com/Hixenov/Aria/releases/download/".to_owned(),
        }
    }
}

/// Why a check or a download did not work. Never carries a URL.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// GitHub could not be reached.
    #[error("GitHub could not be reached")]
    Offline,
    /// GitHub answered something Aria does not understand or will not use.
    #[error("an answer that makes no sense")]
    Answer,
    /// The latest release's tag is not a plain version.
    #[error("the latest release is not a plain version")]
    NotARelease,
    /// The file is larger than Aria will download.
    #[error("the download is too big")]
    TooBig,
    /// The file does not match its checksum.
    #[error("the download does not match its checksum")]
    Checksum,
    /// The file could not be written or read on this computer.
    #[error("the file could not be saved ({0:?})")]
    Disk(std::io::ErrorKind),
}
