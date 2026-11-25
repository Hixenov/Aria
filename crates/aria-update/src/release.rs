//! Reading a release, versions, checksums and the kind of install.

use std::io::{self, Read};
use std::path::Path;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Endpoints, UpdateError};

/// The name of the checksums file of every release.
const SUMS: &str = "SHA256SUMS.txt";
/// The uninstaller Inno Setup puts beside `aria.exe`.
const UNINSTALLER: &str = "unins000.exe";
/// The folder that makes Aria portable (as in `aria-core`'s paths).
const PORTABLE_DIR: &str = "AriaData";

/// One file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// The latest release, as far as Aria cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// Its page on GitHub.
    pub page: String,
    /// `Aria-<version>-setup.exe`, if attached.
    pub installer: Option<Asset>,
    /// `SHA256SUMS.txt`, if attached yet.
    pub sums: Option<Asset>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// Reads GitHub's answer for the latest release. Refuses a tag that is not a plain version and any
/// file Aria would use that is not under `endpoints.download_prefix`.
pub fn parse_release(body: &str, endpoints: &Endpoints) -> Result<Release, UpdateError> {
    let raw: RawRelease = serde_json::from_str(body).map_err(|_| UpdateError::Answer)?;
    let tag = raw.tag_name.strip_prefix('v').unwrap_or(&raw.tag_name);
    let version = Version::parse(tag).map_err(|_| UpdateError::NotARelease)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(UpdateError::NotARelease);
    }
    let wanted = installer_name(&version);
    let mut installer = None;
    let mut sums = None;
    for asset in raw.assets {
        let slot = if asset.name == wanted {
            &mut installer
        } else if asset.name == SUMS {
            &mut sums
        } else {
            continue;
        };
        if !asset
            .browser_download_url
            .starts_with(&endpoints.download_prefix)
        {
            return Err(UpdateError::Answer);
        }
        *slot = Some(Asset {
            name: asset.name,
            size: asset.size,
            url: asset.browser_download_url,
        });
    }
    Ok(Release {
        version,
        page: raw.html_url,
        installer,
        sums,
    })
}

/// Whether `candidate` is a later version than `running`.
pub fn is_newer(candidate: &Version, running: &Version) -> bool {
    candidate > running
}

/// The Windows installer's file name for `version`.
pub fn installer_name(version: &Version) -> String {
    format!("Aria-{version}-setup.exe")
}

/// The SHA-256 that a `sha256sum` listing gives for exactly `file_name`.
pub fn sha256_for(sums: &str, file_name: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim_end().split_once(' ')?;
        let name = name.strip_prefix(' ').or_else(|| name.strip_prefix('*'))?;
        if name != file_name || hash.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(hash, &mut out).ok()?;
        Some(out)
    })
}

/// The SHA-256 of the file at `path`.
pub fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Whether Aria can update itself with the installer, or only tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by Aria's Windows installer.
    Installed,
    /// Portable, a loose `aria.exe`, macOS or Linux.
    TellOnly,
}

/// What kind of Aria runs from `exe_dir`: installed only on Windows, with the installer's
/// uninstaller beside it and no portable data folder.
pub fn install_kind(exe_dir: &Path, windows: bool) -> InstallKind {
    let installed =
        windows && exe_dir.join(UNINSTALLER).is_file() && !exe_dir.join(PORTABLE_DIR).exists();
    if installed {
        InstallKind::Installed
    } else {
        InstallKind::TellOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/release.json");

    fn ep(base: &str) -> Endpoints {
        Endpoints {
            latest: format!("{base}/repos/Hixenov/Aria/releases/latest"),
            download_prefix: format!("{base}/Hixenov/Aria/releases/download/"),
        }
    }

    fn body(base: &str) -> String {
        FIXTURE.replace("{base}", base)
    }

    fn with_tag(tag: &str) -> String {
        body("http://x").replace("\"v0.3.6\"", &format!("\"{tag}\""))
    }

    #[test]
    fn the_release_is_read() {
        let r = parse_release(&body("http://x"), &ep("http://x")).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 6));
        assert_eq!(
            r.installer.as_ref().map(|a| (a.name.as_str(), a.size)),
            Some(("Aria-0.3.6-setup.exe", 11))
        );
        assert_eq!(
            r.sums.as_ref().map(|a| a.name.as_str()),
            Some("SHA256SUMS.txt")
        );
        assert_eq!(
            r.page,
            "https://github.com/Hixenov/Aria/releases/tag/v0.3.6"
        );
    }

    #[test]
    fn odd_tags_are_not_offered() {
        for tag in ["v0.4.0-beta.1", "latest", "", "v0.3.6+build"] {
            assert_eq!(
                parse_release(&with_tag(tag), &ep("http://x")),
                Err(UpdateError::NotARelease),
                "{tag}"
            );
        }
        // No leading v is fine too; the installer is then looked up by the same version.
        let r = parse_release(&with_tag("0.3.6"), &ep("http://x")).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 6));
        assert!(r.installer.is_some());
    }

    #[test]
    fn an_asset_elsewhere_is_refused() {
        let body = body("http://x").replace(
            "http://x/Hixenov/Aria/releases/download/v0.3.6/Aria-0.3.6-setup.exe",
            "https://evil.example/Aria-0.3.6-setup.exe",
        );
        assert_eq!(
            parse_release(&body, &ep("http://x")),
            Err(UpdateError::Answer)
        );
    }

    #[test]
    fn a_release_without_its_files_has_none() {
        let body = r#"{"tag_name":"v0.3.6","html_url":"p","assets":[]}"#;
        let r = parse_release(body, &ep("http://x")).unwrap();
        assert_eq!((r.installer, r.sums), (None, None));
    }

    #[test]
    fn garbage_is_an_odd_answer() {
        assert_eq!(
            parse_release("<html>", &ep("http://x")),
            Err(UpdateError::Answer)
        );
    }

    #[test]
    fn only_higher_versions_are_newer() {
        let run = Version::new(0, 3, 5);
        assert!(is_newer(&Version::new(0, 3, 6), &run));
        assert!(is_newer(&Version::new(1, 0, 0), &run));
        assert!(!is_newer(&Version::new(0, 3, 5), &run));
        assert!(!is_newer(&Version::new(0, 3, 4), &run));
    }

    #[test]
    fn the_checksum_line_is_found_by_exact_name() {
        let sums = format!(
            "{}  Aria-0.3.6-setup.exe\n{} *aria.exe\n",
            "aa".repeat(32),
            "bb".repeat(32)
        );
        assert_eq!(sha256_for(&sums, "Aria-0.3.6-setup.exe"), Some([0xaa; 32]));
        assert_eq!(sha256_for(&sums, "aria.exe"), Some([0xbb; 32]));
        assert_eq!(sha256_for(&sums, "Aria-0.3.6-setup"), None);
        assert_eq!(
            sha256_for("zz  Aria-0.3.6-setup.exe", "Aria-0.3.6-setup.exe"),
            None
        );
        let bad_hex = format!("{}  Aria-0.3.6-setup.exe", "zz".repeat(32));
        assert_eq!(sha256_for(&bad_hex, "Aria-0.3.6-setup.exe"), None);
    }

    #[test]
    fn a_file_hashes_like_sha256sum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hex::encode(sha256_file(&path).unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn only_an_inno_install_on_windows_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install_kind(dir.path(), true), InstallKind::TellOnly);
        std::fs::write(dir.path().join("unins000.exe"), b"").unwrap();
        assert_eq!(install_kind(dir.path(), true), InstallKind::Installed);
        assert_eq!(install_kind(dir.path(), false), InstallKind::TellOnly);
        std::fs::create_dir(dir.path().join("AriaData")).unwrap();
        assert_eq!(install_kind(dir.path(), true), InstallKind::TellOnly);
    }
}
