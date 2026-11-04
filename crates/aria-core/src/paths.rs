//! Application directory resolution.

use crate::error::PathError;
use directories::ProjectDirs;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Environment variable that overrides the config directory.
const CONFIG_DIR_ENV: &str = "ARIA_CONFIG_DIR";

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
    /// Resolves paths, honouring the `ARIA_CONFIG_DIR` environment variable.
    pub fn resolve() -> Result<Self, PathError> {
        Self::resolve_with(std::env::var_os(CONFIG_DIR_ENV).map(PathBuf::from))
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
