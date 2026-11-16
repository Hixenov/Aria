//! Soundboard sounds: a path, a name, a volume and an optional hotkey each.
//!
//! A sound is a reference to a file; Aria never copies, changes or deletes the file, and adding
//! a sound does not check that the file exists (a missing file is shown as such by the UI).

use std::path::PathBuf;

use crate::db::Library;
use crate::error::LibraryError;
use crate::paths::{decode_path, encode_path};

/// How many sounds are stored at most.
pub const SOUNDS_MAX: usize = 100;
/// The longest sound name, in characters.
pub const SOUND_NAME_MAX: usize = 50;

/// The name as it would be stored, or `None` if it breaks the rules: trimmed, 1 to
/// [`SOUND_NAME_MAX`] characters.
pub fn sound_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let chars = name.chars().count();
    (1..=SOUND_NAME_MAX)
        .contains(&chars)
        .then(|| name.to_owned())
}

/// The id of a sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId(pub i64);

/// One stored sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub id: SoundId,
    pub path: PathBuf,
    pub name: String,
    /// 0.0 to 1.0.
    pub volume: f32,
    /// An opaque hotkey string; the library does not interpret it.
    pub hotkey: Option<String>,
}

/// The volume as it is stored: clamped to 0.0..=1.0, and 1.0 (the default) for NaN.
fn clamp_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        1.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

/// The default name of a sound: the file name without its extension, cut to
/// [`SOUND_NAME_MAX`] characters.
fn default_name(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cut: String = stem.trim().chars().take(SOUND_NAME_MAX).collect();
    sound_name(&cut).unwrap_or_else(|| "Sound".to_owned())
}

impl Library {
    /// Every sound, in the order they were added.
    pub fn sounds(&self) -> Result<Vec<Sound>, LibraryError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, name, volume, hotkey FROM sounds ORDER BY position, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Sound {
                id: SoundId(r.get(0)?),
                path: decode_path(&r.get::<_, Vec<u8>>(1)?),
                name: r.get(2)?,
                volume: clamp_volume(r.get::<_, f64>(3)? as f32),
                hotkey: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Adds sounds named after their files, in the order given, until [`SOUNDS_MAX`] are
    /// stored. The same file may be added again. Returns how many were added.
    pub fn add_sounds(&mut self, paths: &[PathBuf]) -> Result<usize, LibraryError> {
        let tx = self.conn.transaction()?;
        let stored: i64 = tx.query_row("SELECT COUNT(*) FROM sounds", [], |r| r.get(0))?;
        let room = SOUNDS_MAX.saturating_sub(usize::try_from(stored).unwrap_or(SOUNDS_MAX));
        let mut added = 0;
        {
            let mut insert = tx.prepare(
                "INSERT INTO sounds (path, name, volume, hotkey, position)
                 VALUES (?1, ?2, 1.0, NULL, (SELECT COALESCE(MAX(position) + 1, 0) FROM sounds))",
            )?;
            for path in paths.iter().take(room) {
                insert.execute((encode_path(path), default_name(path)))?;
                added += 1;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Renames a sound. Returns `false` when the id does not exist.
    pub fn rename_sound(&mut self, id: SoundId, name: &str) -> Result<bool, LibraryError> {
        let name = sound_name(name).ok_or(LibraryError::InvalidSoundName)?;
        let tx = self.conn.transaction()?;
        let changed = tx.execute("UPDATE sounds SET name = ?1 WHERE id = ?2", (name, id.0))?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Sets a sound's volume, clamped to 0.0..=1.0 (NaN becomes 1.0). Returns `false` when the
    /// id does not exist.
    pub fn set_sound_volume(&mut self, id: SoundId, volume: f32) -> Result<bool, LibraryError> {
        let volume = f64::from(clamp_volume(volume));
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE sounds SET volume = ?1 WHERE id = ?2",
            (volume, id.0),
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Sets or clears a sound's hotkey. The string is stored as given; whether it is valid or
    /// free is decided elsewhere. Returns `false` when the id does not exist.
    pub fn set_sound_hotkey(
        &mut self,
        id: SoundId,
        hotkey: Option<&str>,
    ) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE sounds SET hotkey = ?1 WHERE id = ?2",
            (hotkey, id.0),
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Removes a sound; the others keep their order. Returns `false` when the id does not
    /// exist.
    pub fn remove_sound(&mut self, id: SoundId) -> Result<bool, LibraryError> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute("DELETE FROM sounds WHERE id = ?1", [id.0])?;
        tx.commit()?;
        Ok(changed > 0)
    }
}
