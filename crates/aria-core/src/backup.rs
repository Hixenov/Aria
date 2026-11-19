//! Backing up the library and the settings, and restoring them.
//!
//! A backup is a folder, "Aria backup <date and time>", with `library.db` (a consistent copy,
//! made while Aria runs) and `settings.toml`. It holds the music folders' paths, the tracks'
//! tags, favorites, playlists, history, sounds and the queue, but no music and no covers: the
//! files stay where they are, and the covers are read again by the next scan.
//!
//! A database cannot be swapped under the running library, so a restore is prepared now and
//! done at the next start, before the library opens: the backup's files are copied into a
//! `restore` folder in the data folder, and at the next start they take the place of the
//! current ones, which are kept beside them with `.before-restore` added to their names.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aria_library::{Library, LibraryError};

/// A backup was made. Takes `name`, the backup folder's name.
pub const BACKUP_DONE: &str = "backup-done";
/// A backup could not be made.
pub const BACKUP_FAILED: &str = "backup-failed";
/// A restore is ready, and happens when Aria starts again.
pub const RESTORE_STAGED: &str = "restore-staged";
/// The chosen folder holds no backup.
pub const RESTORE_NOT_A_BACKUP: &str = "restore-not-a-backup";
/// The backup comes from a newer Aria.
pub const RESTORE_NEWER: &str = "restore-newer";
/// The backup could not be copied for the restore.
pub const RESTORE_FAILED: &str = "restore-failed";
/// The restore was done at this start.
pub const RESTORE_DONE: &str = "restore-done";

/// The database's file name, in the data folder and in a backup.
pub const LIBRARY_FILE: &str = "library.db";
/// The settings' file name, in the config folder and in a backup.
pub const SETTINGS_FILE: &str = "settings.toml";
/// The folder in the data folder where a restore waits for the next start.
const STAGED: &str = "restore";
/// Added to the name of each file a restore replaces.
const BEFORE: &str = ".before-restore";

/// Why a backup cannot be restored.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// The folder has no library database, or one that is damaged or not Aria's.
    #[error("the folder holds no Aria library backup")]
    NotABackup,
    /// The backup comes from a newer Aria.
    #[error("the backup comes from a newer version of Aria")]
    Newer,
    /// The files could not be copied.
    #[error("the backup could not be copied")]
    Io(#[from] io::Error),
}

/// The name of a backup made at `now`, in UTC: "Aria backup 2026-11-20 14.05".
pub fn backup_folder_name(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "Aria backup {year:04}-{month:02}-{day:02} {:02}.{:02}",
        rest / 3_600,
        rest / 60 % 60
    )
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// A new folder for a backup inside `parent`, named for `now`, with a number added if that name
/// is taken.
///
/// # Errors
///
/// Fails if the folder cannot be made.
pub fn new_backup_folder(parent: &Path, now: SystemTime) -> io::Result<PathBuf> {
    let name = backup_folder_name(now);
    for n in 1..100 {
        let folder = if n == 1 {
            parent.join(&name)
        } else {
            parent.join(format!("{name} ({n})"))
        };
        match fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// Checks the backup in `folder` and copies it into the data folder, to be restored at the next
/// start. A restore prepared before is replaced.
///
/// # Errors
///
/// Fails if the folder holds no library this version can open, or the files cannot be copied.
pub fn stage_restore(folder: &Path, data_dir: &Path) -> Result<(), RestoreError> {
    let library = folder.join(LIBRARY_FILE);
    match Library::check_backup(&library) {
        Ok(()) => {}
        Err(LibraryError::NewerSchema { .. }) => return Err(RestoreError::Newer),
        Err(_) => return Err(RestoreError::NotABackup),
    }
    let staged = data_dir.join(STAGED);
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    fs::create_dir_all(&staged)?;
    fs::copy(&library, staged.join(LIBRARY_FILE))?;
    let settings = folder.join(SETTINGS_FILE);
    if settings.is_file() {
        fs::copy(&settings, staged.join(SETTINGS_FILE))?;
    }
    Ok(())
}

/// Whether a restore waits for the next start.
pub fn restore_pending(data_dir: &Path) -> bool {
    data_dir.join(STAGED).join(LIBRARY_FILE).is_file()
}

/// Puts a prepared restore in place, before the library opens: the backup's database replaces
/// `data_dir`'s, and its settings, if it has them, replace those in `config_dir`. What they
/// replace is kept beside them, named with `.before-restore`. Returns whether there was one.
///
/// # Errors
///
/// Fails if the files cannot be moved; what was moved stays moved, and the rest is left for the
/// next start.
pub fn apply_pending_restore(data_dir: &Path, config_dir: &Path) -> io::Result<bool> {
    let staged = data_dir.join(STAGED);
    let library = staged.join(LIBRARY_FILE);
    if !library.is_file() {
        return Ok(false);
    }
    let current = data_dir.join(LIBRARY_FILE);
    for suffix in ["", "-wal", "-shm"] {
        let from = with_suffix(&current, suffix);
        if from.exists() {
            fs::rename(&from, with_suffix(&current, &format!("{BEFORE}{suffix}")))?;
        }
    }
    let settings = staged.join(SETTINGS_FILE);
    if settings.is_file() {
        let current = config_dir.join(SETTINGS_FILE);
        if current.exists() {
            fs::rename(&current, with_suffix(&current, BEFORE))?;
        }
        fs::create_dir_all(config_dir)?;
        fs::rename(&settings, &current)?;
    }
    fs::rename(&library, &current)?;
    fs::remove_dir_all(&staged)?;
    Ok(true)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A library with one folder in it, at `dir/library.db`, as a backup holds one.
    fn library_in(dir: &Path, folder: &Path) {
        let mut opened = Library::open(&dir.join(LIBRARY_FILE), &dir.join("art"))
            .expect("the library opens")
            .library;
        fs::create_dir_all(folder).unwrap();
        opened.add_folder(folder).unwrap();
    }

    #[test]
    fn a_backup_folder_is_named_for_its_time() {
        let at = UNIX_EPOCH + Duration::from_secs(1_795_788_300); // 2026-11-27 14:05 UTC
        assert_eq!(backup_folder_name(at), "Aria backup 2026-11-27 14.05");
        assert_eq!(
            backup_folder_name(UNIX_EPOCH),
            "Aria backup 1970-01-01 00.00"
        );
        let parent = tempfile::tempdir().unwrap();
        let first = new_backup_folder(parent.path(), at).unwrap();
        let second = new_backup_folder(parent.path(), at).unwrap();
        assert!(first.is_dir() && second.is_dir());
        assert_eq!(
            second.file_name().unwrap(),
            "Aria backup 2026-11-27 14.05 (2)"
        );
    }

    #[test]
    fn a_restore_waits_for_the_next_start_and_keeps_what_it_replaces() {
        let root = tempfile::tempdir().unwrap();
        let (backup, data, config) = (
            root.path().join("backup"),
            root.path().join("data"),
            root.path().join("config"),
        );
        fs::create_dir_all(&backup).unwrap();
        library_in(&backup, &root.path().join("Old music"));
        fs::write(backup.join(SETTINGS_FILE), "theme = \"light\"\n").unwrap();
        // The library and settings in use now.
        library_in(&data, &root.path().join("New music"));
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join(SETTINGS_FILE), "theme = \"dark\"\n").unwrap();

        stage_restore(&backup, &data).unwrap();
        assert!(restore_pending(&data));
        // Nothing in use has changed yet.
        assert_eq!(
            fs::read_to_string(config.join(SETTINGS_FILE)).unwrap(),
            "theme = \"dark\"\n"
        );

        assert!(apply_pending_restore(&data, &config).unwrap());
        assert!(!restore_pending(&data));
        assert_eq!(
            fs::read_to_string(config.join(SETTINGS_FILE)).unwrap(),
            "theme = \"light\"\n"
        );
        assert!(config.join("settings.toml.before-restore").is_file());
        assert!(data.join("library.db.before-restore").is_file());
        let restored = Library::open(&data.join(LIBRARY_FILE), &data.join("art"))
            .unwrap()
            .library;
        let folders = restored.folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert!(folders[0].path.ends_with("Old music"));
        // Done once.
        assert!(!apply_pending_restore(&data, &config).unwrap());
    }

    #[test]
    fn a_folder_without_a_library_is_not_a_backup() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        assert!(matches!(
            stage_restore(root.path(), &data),
            Err(RestoreError::NotABackup)
        ));
        fs::write(root.path().join(LIBRARY_FILE), b"not a database").unwrap();
        assert!(matches!(
            stage_restore(root.path(), &data),
            Err(RestoreError::NotABackup)
        ));
        assert!(!restore_pending(&data));
    }

    #[test]
    fn a_backup_without_settings_restores_the_library_only() {
        let root = tempfile::tempdir().unwrap();
        let (backup, data, config) = (
            root.path().join("backup"),
            root.path().join("data"),
            root.path().join("config"),
        );
        fs::create_dir_all(&backup).unwrap();
        library_in(&backup, &root.path().join("Music"));
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join(SETTINGS_FILE), "theme = \"dark\"\n").unwrap();
        stage_restore(&backup, &data).unwrap();
        assert!(apply_pending_restore(&data, &config).unwrap());
        assert_eq!(
            fs::read_to_string(config.join(SETTINGS_FILE)).unwrap(),
            "theme = \"dark\"\n"
        );
        assert!(data.join(LIBRARY_FILE).is_file());
    }
}
