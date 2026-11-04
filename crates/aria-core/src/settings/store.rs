//! Reading and atomically writing `settings.toml`.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{CURRENT_SCHEMA_VERSION, Settings};
use crate::{
    error::SettingsError,
    notice::{Notice, NoticeLevel},
};

/// Outcome of [`SettingsStore::load`].
#[derive(Clone, Debug, PartialEq)]
pub struct LoadResult {
    /// The settings to use (defaults if nothing usable was on disk).
    pub settings: Settings,
    /// A notice to surface to the user, if recovery happened.
    pub notice: Option<Notice>,
}

/// Loads and saves settings at a fixed path.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// Create a store for the given `settings.toml` path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The settings file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load settings. Never fails: problems yield defaults and a [`Notice`].
    pub fn load(&self) -> LoadResult {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return LoadResult {
                    settings: Settings::default(),
                    notice: None,
                };
            }
            Err(e) => {
                tracing::error!(path = %self.path.display(), error = %e, "cannot read settings file");
                return LoadResult {
                    settings: Settings::default(),
                    notice: Some(Notice {
                        level: NoticeLevel::Warning,
                        key: "notice-settings-readonly",
                    }),
                };
            }
        };
        match parse(&bytes) {
            Ok(settings) => LoadResult {
                settings,
                notice: None,
            },
            Err(e) => {
                tracing::warn!(path = %self.path.display(), error = %e, "settings file is unusable; backing it up");
                self.back_up();
                LoadResult {
                    settings: Settings::default(),
                    notice: Some(Notice {
                        level: NoticeLevel::Warning,
                        key: "notice-settings-recovered",
                    }),
                }
            }
        }
    }

    /// Atomically write settings: temp file, `sync_all`, rename over target.
    pub fn save(&self, s: &Settings) -> Result<(), SettingsError> {
        let text = toml::to_string(s)?;
        let io_err = |path: &Path| {
            let path = path.to_path_buf();
            move |source| SettingsError::Io { path, source }
        };
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let tmp = sibling_with_suffix(&self.path, ".tmp");
        let result = (|| {
            let mut f = File::create(&tmp).map_err(io_err(&tmp))?;
            f.write_all(text.as_bytes()).map_err(io_err(&tmp))?;
            f.sync_all().map_err(io_err(&tmp))?;
            drop(f);
            fs::rename(&tmp, &self.path).map_err(io_err(&self.path))
        })();
        if result.is_err() {
            // Best effort: do not leave a stale temp file behind.
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    fn back_up(&self) {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let dest = unique_backup_path(&self.path, secs);
        if let Err(e) = fs::rename(&self.path, &dest) {
            tracing::error!(from = %self.path.display(), to = %dest.display(), error = %e, "cannot back up corrupt settings");
        }
    }
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// `settings.toml.bak-<secs>`, or `-1`, `-2`, ... if that name is taken.
fn unique_backup_path(path: &Path, secs: u64) -> PathBuf {
    let first = sibling_with_suffix(path, &format!(".bak-{secs}"));
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| sibling_with_suffix(path, &format!(".bak-{secs}-{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn parse(bytes: &[u8]) -> Result<Settings, SettingsError> {
    let text = std::str::from_utf8(bytes).map_err(|e| SettingsError::Io {
        path: PathBuf::new(),
        source: io::Error::new(io::ErrorKind::InvalidData, e),
    })?;
    let table: toml::Table = toml::from_str(text)?;
    let version = match table.get("schema_version") {
        None => 1,
        Some(v) => v
            .as_integer()
            .and_then(|i| u32::try_from(i).ok())
            .ok_or(SettingsError::UnsupportedVersion(u32::MAX))?,
    };
    if version > CURRENT_SCHEMA_VERSION {
        return Err(SettingsError::UnsupportedVersion(version));
    }
    let table = migrate(table, version);
    Ok(table.try_into::<Settings>()?.sanitized())
}

/// Upgrade a settings table from schema version `from` to the current one.
/// Add one arm per historical version when the schema changes.
fn migrate(value: toml::Table, from: u32) -> toml::Table {
    match from {
        // v1 is the current schema: nothing to do.
        1 => value,
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        notice::{Notice, NoticeLevel},
        settings::{Language, Rgb, Theme},
    };
    use std::fs;

    fn recovered() -> Notice {
        Notice {
            level: NoticeLevel::Warning,
            key: "notice-settings-recovered",
        }
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings.toml.bak-")
            })
            .collect()
    }

    #[test]
    fn missing_file_gives_defaults_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(tmp.path().join("settings.toml"));
        let r = store.load();
        assert_eq!(r.settings, Settings::default());
        assert_eq!(r.notice, None);
        assert!(!store.path().exists());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(tmp.path().join("settings.toml"));
        let s = Settings {
            theme: Theme::Dark,
            language: Some(Language::Hy),
            accent_light: Some(Rgb { r: 1, g: 2, b: 3 }),
            ..Settings::default()
        };
        store.save(&s).unwrap();
        let r = store.load();
        assert_eq!(r.settings, s);
        assert_eq!(r.notice, None);
    }

    #[test]
    fn store_roundtrip_non_ascii_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Пользователь/Աria");
        let store = SettingsStore::new(dir.join("settings.toml"));
        let s = Settings {
            theme: Theme::Light,
            ..Settings::default()
        };
        store.save(&s).unwrap();
        assert_eq!(store.load().settings, s);
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        fs::write(&path, "theme = [[[").unwrap();
        let r = SettingsStore::new(path.clone()).load();
        assert_eq!(r.settings, Settings::default());
        assert_eq!(r.notice, Some(recovered()));
        assert!(!path.exists());
        let b = backups(tmp.path());
        assert_eq!(b.len(), 1);
        assert_eq!(fs::read(&b[0]).unwrap(), b"theme = [[[");
    }

    #[test]
    fn newer_schema_is_treated_as_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        fs::write(&path, "schema_version = 99").unwrap();
        let r = SettingsStore::new(path.clone()).load();
        assert_eq!(r.settings, Settings::default());
        assert_eq!(r.notice, Some(recovered()));
        assert!(!path.exists());
        assert_eq!(backups(tmp.path()).len(), 1);
    }

    #[test]
    fn second_recovery_in_same_second_keeps_both_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        let store = SettingsStore::new(path.clone());
        // Pre-create the plain backup name for every plausible current second
        // would be racy, so corrupt twice and rely on the collision rule.
        fs::write(&path, "theme = [[[").unwrap();
        store.load();
        fs::write(&path, "theme = [[[").unwrap();
        store.load();
        assert_eq!(backups(tmp.path()).len(), 2);
    }

    #[test]
    fn backup_names_never_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        let a = unique_backup_path(&path, 7);
        assert_eq!(a.file_name().unwrap(), "settings.toml.bak-7");
        fs::write(&a, "x").unwrap();
        let b = unique_backup_path(&path, 7);
        assert_eq!(b.file_name().unwrap(), "settings.toml.bak-7-1");
        fs::write(&b, "x").unwrap();
        let c = unique_backup_path(&path, 7);
        assert_eq!(c.file_name().unwrap(), "settings.toml.bak-7-2");
    }

    #[test]
    fn save_leaves_no_tmp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(tmp.path().join("settings.toml"));
        store.save(&Settings::default()).unwrap();
        assert!(!tmp.path().join("settings.toml.tmp").exists());
    }

    #[test]
    fn save_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(tmp.path().join("settings.toml"));
        store
            .save(&Settings {
                theme: Theme::Dark,
                ..Settings::default()
            })
            .unwrap();
        store
            .save(&Settings {
                theme: Theme::Light,
                ..Settings::default()
            })
            .unwrap();
        assert_eq!(store.load().settings.theme, Theme::Light);
    }

    #[test]
    fn out_of_range_values_are_clamped_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        fs::write(&path, "sidebar_width = 5000.0").unwrap();
        let r = SettingsStore::new(path).load();
        assert_eq!(r.settings.sidebar_width, 320.0);
        assert_eq!(r.notice, None);
    }

    #[test]
    fn missing_schema_version_loads_as_v1() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        fs::write(&path, "theme = \"dark\"").unwrap();
        let r = SettingsStore::new(path).load();
        assert_eq!(r.settings.theme, Theme::Dark);
        assert_eq!(r.notice, None);
    }
}
