//! Playlists as M3U files: read from one, and written to one.
//!
//! An M3U file lists one file per line, with `#` lines for notes; `#EXTINF:<seconds>,<name>`
//! names the line after it. Aria writes `.m3u8`, UTF-8 with full paths, and reads `.m3u` and
//! `.m3u8`, with full or relative paths and `file://` addresses. Web addresses are skipped.

use std::path::{Path, PathBuf};

use crate::collections::PlaylistId;
use crate::db::Library;
use crate::error::LibraryError;
use crate::paths::{decode_path, encode_path};
use crate::playlist_name;

/// The files an M3U playlist lists, in order. Relative paths are taken from `base`, the folder
/// the playlist is in. Notes, blank lines and web addresses are left out.
pub fn parse_m3u(text: &str, base: &Path) -> Vec<PathBuf> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| entry_path(line, base))
        .collect()
}

/// The file one line of a playlist names.
fn entry_path(line: &str, base: &Path) -> Option<PathBuf> {
    if let Some(rest) = line.strip_prefix("file://") {
        let decoded = percent_decode(rest)?;
        // file:///C:/Music on Windows, file:///home/me on Unix.
        let path = if cfg!(windows) {
            decoded.trim_start_matches('/').to_owned()
        } else {
            decoded
        };
        return Some(PathBuf::from(path));
    }
    if line.contains("://") {
        return None;
    }
    let path = PathBuf::from(line);
    Some(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

/// `%20`-style escapes undone; `None` for text that is not UTF-8 once undone.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 3 <= bytes.len() => {
                let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// One entry of a playlist to write: its file and, when the library knows the track, its length
/// in seconds and the name to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M3uEntry {
    pub path: PathBuf,
    pub info: Option<(u64, String)>,
}

/// A playlist written as an extended M3U, with full paths, for a `.m3u8` file.
pub fn write_m3u(entries: &[M3uEntry]) -> String {
    let mut text = String::from("#EXTM3U\n");
    for entry in entries {
        if let Some((seconds, name)) = &entry.info {
            // A line break in a tag would end the line early.
            let name = name.replace(['\r', '\n'], " ");
            text.push_str(&format!("#EXTINF:{seconds},{name}\n"));
        }
        text.push_str(&entry.path.to_string_lossy());
        text.push('\n');
    }
    text
}

impl Library {
    /// Makes a playlist called `name` with `paths` in it, in order, whether or not their files
    /// are in the library now: entries are kept by path, and show once their folder is added.
    pub fn import_playlist(
        &mut self,
        name: &str,
        paths: &[PathBuf],
        now: i64,
    ) -> Result<PlaylistId, LibraryError> {
        let name = playlist_name(name).ok_or(LibraryError::InvalidPlaylistName)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO playlists (name, created_at) VALUES (?1, ?2)",
            (name, now),
        )?;
        let id = PlaylistId(tx.last_insert_rowid());
        {
            let mut insert = tx.prepare(
                "INSERT INTO playlist_items (playlist_id, position, path) VALUES (?1, ?2, ?3)",
            )?;
            for (position, path) in paths.iter().enumerate() {
                let position = i64::try_from(position).unwrap_or(i64::MAX);
                insert.execute((id.0, position, encode_path(path)))?;
            }
        }
        tx.commit()?;
        Ok(id)
    }

    /// A playlist's name and every entry of it, in order, with what the library knows of each
    /// track; `None` for an unknown playlist. Entries whose file is not in the library are
    /// written too, without a name.
    pub fn playlist_entries(
        &self,
        id: PlaylistId,
    ) -> Result<Option<(String, Vec<M3uEntry>)>, LibraryError> {
        let name: Option<String> = rusqlite::OptionalExtension::optional(self.conn.query_row(
            "SELECT name FROM playlists WHERE id = ?1",
            [id.0],
            |row| row.get(0),
        ))?;
        let Some(name) = name else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT i.path, t.duration_ms, t.title, t.artist
             FROM playlist_items i LEFT JOIN tracks t ON t.path = i.path
             WHERE i.playlist_id = ?1 ORDER BY i.position",
        )?;
        let entries = stmt
            .query_map([id.0], |row| {
                let path: Vec<u8> = row.get(0)?;
                let duration: Option<i64> = row.get(1)?;
                let title: Option<String> = row.get(2)?;
                let artist: Option<String> = row.get(3)?;
                Ok((path, duration, title, artist))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(path, duration, title, artist)| M3uEntry {
                path: decode_path(&path),
                info: title.map(|title| {
                    let seconds = duration.map_or(0, |ms| u64::try_from(ms).unwrap_or(0) / 1_000);
                    let name = match artist.filter(|artist| !artist.trim().is_empty()) {
                        Some(artist) => format!("{artist} - {title}"),
                        None => title,
                    };
                    (seconds, name)
                }),
            })
            .collect();
        Ok(Some((name, entries)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_blank_lines_and_web_addresses_are_left_out() {
        let base = Path::new("/music/lists");
        let text = "\u{feff}#EXTM3U\n#EXTINF:123,Artist - Song\n/music/a.mp3\n\n  \nhttp://radio.example/stream\nalbum/b.flac\r\n";
        let paths = parse_m3u(text, base);
        assert_eq!(
            paths,
            [
                PathBuf::from("/music/a.mp3"),
                base.join("album").join("b.flac")
            ]
        );
    }

    #[test]
    fn file_addresses_are_read_with_their_escapes() {
        let paths = parse_m3u("file:///music/My%20Song%20%C3%A9.mp3\n", Path::new("/"));
        let expected = if cfg!(windows) {
            "music/My Song é.mp3"
        } else {
            "/music/My Song é.mp3"
        };
        assert_eq!(paths, [PathBuf::from(expected)]);
    }

    #[test]
    fn a_written_playlist_names_its_tracks_and_reads_back() {
        let entries = [
            M3uEntry {
                path: PathBuf::from("/music/a.mp3"),
                info: Some((215, "Someone - A\nsong".into())),
            },
            M3uEntry {
                path: PathBuf::from("/music/gone.flac"),
                info: None,
            },
        ];
        let text = write_m3u(&entries);
        assert_eq!(
            text,
            "#EXTM3U\n#EXTINF:215,Someone - A song\n/music/a.mp3\n/music/gone.flac\n"
        );
        assert_eq!(
            parse_m3u(&text, Path::new("/elsewhere")),
            [
                PathBuf::from("/music/a.mp3"),
                PathBuf::from("/music/gone.flac")
            ]
        );
    }
}
