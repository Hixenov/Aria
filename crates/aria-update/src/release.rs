//! Reading a release, versions, checksums and the kind of install.

use std::io::{self, Read};
use std::path::Path;

use semver::Version;
use sha2::{Digest, Sha256};

use crate::{Endpoints, UpdateError};

/// The name of the checksums file of every release.
const SUMS: &str = "SHA256SUMS.txt";
/// The uninstaller Inno Setup puts beside `aria.exe`.
const UNINSTALLER: &str = "unins000.exe";
/// The folder that makes Aria portable (as in `aria-core`'s paths).
const PORTABLE_DIR: &str = "AriaData";

/// One file of a release. Its size is learned while downloading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// The latest release, as far as Aria cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// Its page on GitHub.
    pub page: String,
    /// Where `Aria-<version>-setup.exe` would be. It may not be there.
    pub installer: Asset,
    /// Where `SHA256SUMS.txt` would be. It is not there while the release is being published.
    pub sums: Asset,
}

/// The release tagged `tag`, whose page is `page`. Refuses a tag that is not a plain version.
pub fn release_for_tag(
    tag: &str,
    page: String,
    endpoints: &Endpoints,
) -> Result<Release, UpdateError> {
    let plain = tag.strip_prefix('v').unwrap_or(tag);
    let version = Version::parse(plain).map_err(|_| UpdateError::NotARelease)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(UpdateError::NotARelease);
    }
    // A plain version has no `/`, `?` or `#`, so the tag is safe to put in a path.
    let asset = |name: String| Asset {
        url: format!("{}{tag}/{name}", endpoints.download_prefix),
        name,
    };
    Ok(Release {
        installer: asset(installer_name(&version)),
        sums: asset(SUMS.to_owned()),
        version,
        page,
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

    fn ep(base: &str) -> Endpoints {
        Endpoints {
            latest: format!("{base}/Hixenov/Aria/releases/latest"),
            tag_prefix: format!("{base}/Hixenov/Aria/releases/tag/"),
            download_prefix: format!("{base}/Hixenov/Aria/releases/download/"),
        }
    }

    #[test]
    fn a_tag_gives_the_release_and_its_files() {
        let r = release_for_tag("v0.3.6", "page".to_owned(), &ep("http://x")).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 6));
        assert_eq!(r.page, "page");
        assert_eq!(
            r.installer,
            Asset {
                name: "Aria-0.3.6-setup.exe".to_owned(),
                url: "http://x/Hixenov/Aria/releases/download/v0.3.6/Aria-0.3.6-setup.exe"
                    .to_owned(),
            }
        );
        assert_eq!(
            r.sums.url,
            "http://x/Hixenov/Aria/releases/download/v0.3.6/SHA256SUMS.txt"
        );
    }

    #[test]
    fn odd_tags_are_not_offered() {
        for tag in [
            "v0.4.0-beta.1",
            "latest",
            "",
            "v0.3.6+build",
            "v0.3.6/../x",
            "v0.3.6?x",
        ] {
            assert_eq!(
                release_for_tag(tag, String::new(), &ep("http://x")),
                Err(UpdateError::NotARelease),
                "{tag}"
            );
        }
        // No leading v is fine too; the files keep the tag as it is.
        let r = release_for_tag("0.3.6", String::new(), &ep("http://x")).unwrap();
        assert_eq!(r.version, Version::new(0, 3, 6));
        assert_eq!(
            r.sums.url,
            "http://x/Hixenov/Aria/releases/download/0.3.6/SHA256SUMS.txt"
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
