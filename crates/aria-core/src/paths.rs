//! Application directory resolution.

use crate::error::PathError;
use directories::ProjectDirs;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// Environment variable that overrides the config directory.
const CONFIG_DIR_ENV: &str = "ARIA_CONFIG_DIR";

/// A folder of this name beside the program makes Aria portable: everything it stores goes in
/// there, and nothing in the user's own folders.
pub const PORTABLE_DIR: &str = "AriaData";

/// Resolved per-user directories used by Aria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Directory holding user settings.
    pub config_dir: PathBuf,
    /// Directory holding persistent application data.
    pub data_dir: PathBuf,
    /// Directory holding disposable cache data.
    pub cache_dir: PathBuf,
    /// Directory holding log files.
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Resolves paths, honouring the `ARIA_CONFIG_DIR` environment variable. An empty
    /// value counts as unset. With an [`PORTABLE_DIR`] folder beside the program, everything is
    /// in there.
    pub fn resolve() -> Result<Self, PathError> {
        let config = config_override(std::env::var_os(CONFIG_DIR_ENV));
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        match beside.as_deref().and_then(portable_root) {
            Some(root) => Ok(Self::portable(&root, config)),
            None => Self::resolve_with(config),
        }
    }

    /// The paths of a portable Aria, all inside `root`; `config_override` replaces only
    /// `config_dir`.
    pub fn portable(root: &Path, config_override: Option<PathBuf>) -> Self {
        let data_dir = root.join("data");
        Self {
            config_dir: config_override.unwrap_or_else(|| root.join("config")),
            cache_dir: root.join("cache"),
            log_dir: data_dir.join("logs"),
            data_dir,
        }
    }

    /// Resolves paths; `config_override` replaces only `config_dir`.
    pub fn resolve_with(config_override: Option<PathBuf>) -> Result<Self, PathError> {
        let dirs = ProjectDirs::from("", "", "Aria").ok_or(PathError::NoHome)?;
        let data_dir = dirs.data_dir().to_path_buf();
        Ok(Self {
            config_dir: config_override.unwrap_or_else(|| dirs.config_dir().to_path_buf()),
            cache_dir: dirs.cache_dir().to_path_buf(),
            log_dir: data_dir.join("logs"),
            data_dir,
        })
    }

    /// Path of the settings file inside `config_dir`.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// Creates all four directories (and parents) if missing.
    pub fn ensure_dirs(&self) -> Result<(), PathError> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.log_dir,
        ] {
            create(dir)?;
        }
        Ok(())
    }
}

/// The portable folder beside the program in `exe_dir`, if there is one.
pub fn portable_root(exe_dir: &Path) -> Option<PathBuf> {
    let root = exe_dir.join(PORTABLE_DIR);
    root.is_dir().then_some(root)
}

/// The config directory named by the value of `ARIA_CONFIG_DIR`, if it names one.
fn config_override(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn create(path: &Path) -> Result<(), PathError> {
    fs::create_dir_all(path).map_err(|source| PathError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_replaces_config_dir_only() {
        let p = AppPaths::resolve_with(Some("/tmp/x".into())).unwrap();
        assert_eq!(p.config_dir, PathBuf::from("/tmp/x"));
        assert_eq!(p.settings_file(), PathBuf::from("/tmp/x/settings.toml"));
        assert!(p.log_dir.ends_with("logs"));
    }

    #[test]
    fn empty_override_counts_as_unset() {
        assert_eq!(config_override(Some(OsString::new())), None);
        assert_eq!(config_override(None), None);
        assert_eq!(
            config_override(Some("portable".into())),
            Some(PathBuf::from("portable"))
        );
    }

    #[test]
    fn a_portable_folder_beside_the_program_holds_everything() {
        let t = tempfile::tempdir().unwrap();
        // Without the folder, Aria is not portable.
        assert_eq!(portable_root(t.path()), None);
        std::fs::create_dir(t.path().join(PORTABLE_DIR)).unwrap();
        let root = portable_root(t.path()).expect("the folder makes Aria portable");
        assert_eq!(root, t.path().join(PORTABLE_DIR));

        let p = AppPaths::portable(&root, None);
        for dir in [&p.config_dir, &p.data_dir, &p.cache_dir, &p.log_dir] {
            assert!(dir.starts_with(&root), "{} is outside", dir.display());
        }
        assert_eq!(p.settings_file(), root.join("config").join("settings.toml"));
        assert!(p.log_dir.starts_with(&p.data_dir));
        // ARIA_CONFIG_DIR still moves the settings.
        let moved = AppPaths::portable(&root, Some("/elsewhere".into()));
        assert_eq!(moved.config_dir, PathBuf::from("/elsewhere"));
        assert!(moved.data_dir.starts_with(&root));
    }

    #[test]
    fn a_file_named_like_the_portable_folder_is_not_one() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(PORTABLE_DIR), b"").unwrap();
        assert_eq!(portable_root(t.path()), None);
    }

    #[test]
    fn ensure_dirs_creates_all() {
        let t = tempfile::tempdir().unwrap();
        let p = AppPaths {
            config_dir: t.path().join("c"),
            data_dir: t.path().join("d"),
            cache_dir: t.path().join("k"),
            log_dir: t.path().join("d").join("logs"),
        };
        p.ensure_dirs().unwrap();
        assert!(p.config_dir.is_dir());
        assert!(p.data_dir.is_dir());
        assert!(p.cache_dir.is_dir());
        assert!(p.log_dir.is_dir());
    }
}
