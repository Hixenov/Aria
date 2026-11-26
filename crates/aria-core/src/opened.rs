//! Files played from outside the library, for this session only.
//!
//! A file opened with Aria (a double-click, "Open with", the command line or a drop) that is not
//! in the library plays without being added to it. It gets a track of its own, read from the
//! file, under a negative id that no library row can have. Such a track is kept in memory until
//! Aria closes: it is never saved with the queue, and it gets no heart and no history.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aria_library::{
    FolderId, LibraryError, QueueRecord, SUPPORTED_EXTENSIONS, Track, TrackId, TrackTags,
};

/// Whether `id` names a track opened from outside the library.
pub fn is_opened(id: TrackId) -> bool {
    id.0 < 0
}

/// The argument the system starts Aria with at login ("Start with the system", since 0.4.3).
pub const AUTOSTART_ARG: &str = "--autostart";

/// Whether Aria was started at login: [`AUTOSTART_ARG`] among the arguments.
pub fn is_autostart(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().any(|arg| arg == AUTOSTART_ARG)
}

/// The files and folders Aria was started with, from the arguments after the program's own name:
/// each made whole against `base`, the folder it was started in, so that another copy of Aria,
/// started elsewhere, can open them. Empty arguments and [`AUTOSTART_ARG`] are left out.
pub fn launch_paths(args: impl IntoIterator<Item = OsString>, base: &Path) -> Vec<PathBuf> {
    args.into_iter()
        .filter(|arg| !arg.is_empty() && arg != AUTOSTART_ARG)
        .map(|arg| base.join(arg))
        .collect()
}

/// Whether `path` has the extension of a file Aria plays, in any letter case.
pub(crate) fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| supported.eq_ignore_ascii_case(extension))
        })
}

/// The form of a path that two spellings of one file share: on Windows, where paths are compared
/// without regard to letter case, the lower-case text.
pub(crate) fn path_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// `record` without the tracks opened from outside the library, which are not kept from one run
/// to the next: a later run would give their ids to other files. The context index then names
/// the last kept track at or before the one it named, and an opened current track leaves
/// nothing current, so the restore goes on with what came after it.
pub(crate) fn without_opened(record: QueueRecord) -> QueueRecord {
    let QueueRecord {
        context,
        original,
        user,
        current,
        position_ms,
        shuffle_seed,
        context_index,
    } = record;
    let context_index = context_index.and_then(|index| {
        let kept = context
            .iter()
            .take(index.saturating_add(1))
            .filter(|id| !is_opened(**id))
            .count();
        kept.checked_sub(1)
    });
    let keep = |ids: Vec<TrackId>| -> Vec<TrackId> {
        ids.into_iter().filter(|id| !is_opened(*id)).collect()
    };
    let current_opened = current.is_some_and(is_opened);
    QueueRecord {
        context: keep(context),
        original: keep(original),
        user: keep(user),
        current: current.filter(|id| !is_opened(*id)),
        position_ms: if current_opened { 0 } else { position_ms },
        shuffle_seed,
        context_index,
    }
}

/// The tracks opened from outside the library this session.
#[derive(Debug, Default)]
pub(crate) struct OpenedTracks {
    /// In the order they were opened; the first has id -1.
    tracks: Vec<Track>,
    by_path: HashMap<PathBuf, TrackId>,
}

impl OpenedTracks {
    /// The track `id`, if it was opened this session.
    pub(crate) fn track(&self, id: TrackId) -> Option<&Track> {
        if !is_opened(id) {
            return None;
        }
        let index = usize::try_from(-(id.0 + 1)).ok()?;
        self.tracks.get(index)
    }

    /// Every track opened so far.
    pub(crate) fn all(&self) -> Arc<[Track]> {
        Arc::from(self.tracks.as_slice())
    }

    /// The id of the file at `path`. One opened before keeps its id, and is not read again; a
    /// new one is read with `read`. `None` when it cannot be read.
    pub(crate) fn open(
        &mut self,
        path: &Path,
        read: impl FnOnce(&Path) -> Result<TrackTags, LibraryError>,
    ) -> Option<TrackId> {
        let key = path_key(path);
        if let Some(id) = self.by_path.get(&key) {
            return Some(*id);
        }
        let tags = match read(path) {
            Ok(tags) => tags,
            Err(error) => {
                tracing::debug!(%error, "an opened file could not be read");
                return None;
            }
        };
        let id = TrackId(-(i64::try_from(self.tracks.len()).ok()? + 1));
        self.tracks.push(Track {
            id,
            folder: FolderId(0),
            path: path.to_path_buf(),
            title: tags.title,
            artist: tags.artist,
            album: tags.album,
            album_artist: tags.album_artist,
            track_no: tags.track_no,
            disc_no: tags.disc_no,
            year: tags.year,
            genre: tags.genre,
            duration_ms: tags.duration_ms,
            sample_rate: tags.sample_rate,
            channels: tags.channels,
            bits_per_sample: tags.bits_per_sample,
            codec: tags.codec,
            // Thumbnails are made by the library's scan, which this file is not part of.
            artwork: None,
            added_at: 0,
        });
        self.by_path.insert(key, id);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(title: &str) -> TrackTags {
        TrackTags {
            title: title.into(),
            artist: Some(Arc::from("Someone")),
            album: None,
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: 2_000,
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: 16,
            codec: Arc::from("flac"),
            picture: None,
        }
    }

    #[test]
    fn the_autostart_argument_is_not_a_file() {
        let args = || vec![OsString::from("--autostart"), OsString::from("song.flac")];
        assert_eq!(
            launch_paths(args(), Path::new("/base")),
            vec![PathBuf::from("/base/song.flac")]
        );
        assert!(is_autostart(args()));
        assert!(!is_autostart(vec![OsString::from("song.flac")]));
        assert_eq!(AUTOSTART_ARG, "--autostart");
    }

    #[test]
    fn opened_tracks_get_negative_ids_in_order() {
        let mut opened = OpenedTracks::default();
        let a = opened.open(Path::new("/music/a.flac"), |_| Ok(tags("A")));
        let b = opened.open(Path::new("/music/b.flac"), |_| Ok(tags("B")));
        assert_eq!((a, b), (Some(TrackId(-1)), Some(TrackId(-2))));
        assert!(is_opened(TrackId(-1)) && !is_opened(TrackId(1)) && !is_opened(TrackId(0)));
        let track = opened.track(TrackId(-2)).expect("the second track");
        assert_eq!(track.title, "B");
        assert_eq!(track.path, Path::new("/music/b.flac"));
        assert_eq!(track.duration_ms, 2_000);
        assert_eq!(opened.track(TrackId(-3)), None);
        assert_eq!(opened.track(TrackId(1)), None);
        assert_eq!(opened.all().len(), 2);
    }

    #[test]
    fn a_file_opened_again_keeps_its_id_and_is_not_read_again() {
        let mut opened = OpenedTracks::default();
        let first = opened.open(Path::new("/music/a.flac"), |_| Ok(tags("A")));
        let again = opened.open(Path::new("/music/a.flac"), |_| panic!("read a second time"));
        assert_eq!(first, again);
        assert_eq!(opened.all().len(), 1);
    }

    #[test]
    fn a_file_that_cannot_be_read_gets_no_track() {
        let mut opened = OpenedTracks::default();
        let none = opened.open(Path::new("/music/broken.mp3"), |_| {
            Err(LibraryError::Tags("broken".into()))
        });
        assert_eq!(none, None);
        assert_eq!(opened.all().len(), 0);
        // The next one still gets -1.
        let next = opened.open(Path::new("/music/a.flac"), |_| Ok(tags("A")));
        assert_eq!(next, Some(TrackId(-1)));
    }

    #[test]
    fn a_saved_queue_keeps_no_opened_track() {
        let ids = |ids: &[i64]| ids.iter().copied().map(TrackId).collect::<Vec<_>>();
        let record = QueueRecord {
            context: ids(&[1, -1, 2, -2, 3]),
            original: ids(&[-1, 1, 2, 3, -2]),
            user: ids(&[-3, 4]),
            current: Some(TrackId(-2)),
            position_ms: 5_000,
            shuffle_seed: 7,
            context_index: Some(3),
        };
        let saved = without_opened(record);
        assert_eq!(saved.context, ids(&[1, 2, 3]));
        assert_eq!(saved.original, ids(&[1, 2, 3]));
        assert_eq!(saved.user, ids(&[4]));
        assert_eq!(saved.current, None);
        assert_eq!(saved.position_ms, 0);
        // The opened -2 at index 3 is gone; the last kept track before it is 2, now index 1.
        assert_eq!(saved.context_index, Some(1));
        assert_eq!(saved.shuffle_seed, 7);

        // A library track stays current, at its position.
        let kept = without_opened(QueueRecord {
            context: ids(&[-1, 5]),
            current: Some(TrackId(5)),
            position_ms: 1_200,
            context_index: Some(1),
            ..QueueRecord::default()
        });
        assert_eq!(
            (
                kept.context,
                kept.current,
                kept.position_ms,
                kept.context_index
            ),
            (ids(&[5]), Some(TrackId(5)), 1_200, Some(0))
        );
        // Only opened tracks before it: no index.
        let none = without_opened(QueueRecord {
            context: ids(&[-1, 5]),
            context_index: Some(0),
            ..QueueRecord::default()
        });
        assert_eq!(none.context_index, None);
    }

    #[test]
    fn launch_arguments_become_whole_paths() {
        let base = if cfg!(windows) {
            Path::new(r"C:\Users\me\Downloads")
        } else {
            Path::new("/home/me/Downloads")
        };
        let absolute = if cfg!(windows) {
            r"D:\Music\a.flac"
        } else {
            "/srv/music/a.flac"
        };
        let args = ["song.mp3", absolute, ""].map(std::ffi::OsString::from);
        assert_eq!(
            launch_paths(args, base),
            [base.join("song.mp3"), PathBuf::from(absolute)]
        );
    }

    #[test]
    fn audio_files_are_told_by_their_extension_in_any_case() {
        for name in ["a.mp3", "b.FLAC", "c.Opus", "d.aif", "e.m4a"] {
            assert!(is_audio_file(Path::new(name)), "{name}");
        }
        for name in ["a.txt", "b", "c.jpg", "mp3"] {
            assert!(!is_audio_file(Path::new(name)), "{name}");
        }
    }
}
