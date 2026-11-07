//! The in-memory view of the library that the UI reads.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::FolderId;
use crate::db::Library;
use crate::error::LibraryError;
use crate::model::{ArtworkId, Folder, Track, TrackId};
use crate::paths::decode_path;

/// Tracks sharing an album, grouped without regard to letter case.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    /// `None` for tracks that have no album tag.
    pub title: Option<Arc<str>>,
    /// The album artist, else the artist, of the first track.
    pub artist: Option<Arc<str>>,
    /// In disc, track and title order.
    pub tracks: Vec<TrackId>,
    pub artwork: Option<ArtworkId>,
    pub year: Option<i32>,
}

/// Tracks of one artist, grouped without regard to letter case.
#[derive(Debug, Clone, PartialEq)]
pub struct Artist {
    /// `None` for tracks that have no artist tag.
    pub name: Option<Arc<str>>,
    /// In id order.
    pub tracks: Vec<TrackId>,
}

/// Everything the UI needs, built from one consistent read of the database.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LibrarySnapshot {
    /// Sorted by id.
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub folders: Vec<Folder>,
}

impl LibrarySnapshot {
    /// The track with this id, by binary search.
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks
            .binary_search_by_key(&id, |t| t.id)
            .ok()
            .map(|i| &self.tracks[i])
    }
}

/// Shares one `Arc<str>` between equal strings.
#[derive(Default)]
struct Interner(HashSet<Arc<str>>);

impl Interner {
    fn get(&mut self, s: &str) -> Arc<str> {
        if let Some(k) = self.0.get(s) {
            return Arc::clone(k);
        }
        let a: Arc<str> = Arc::from(s);
        self.0.insert(Arc::clone(&a));
        a
    }

    fn opt(&mut self, s: Option<String>) -> Option<Arc<str>> {
        s.map(|s| self.get(&s))
    }
}

fn lower(s: &Option<Arc<str>>) -> Option<String> {
    s.as_deref().map(str::to_lowercase)
}

/// Sort key that puts missing values last.
fn none_last(s: &Option<String>) -> (bool, &str) {
    (s.is_none(), s.as_deref().unwrap_or(""))
}

impl Library {
    /// Reads every track and folder in one transaction, so the result is one consistent state.
    pub fn snapshot(&self) -> Result<LibrarySnapshot, LibraryError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut strings = Interner::default();
        let tracks = {
            let mut stmt = tx.prepare(
                "SELECT id, folder_id, path, title, artist, album, album_artist, track_no,
                        disc_no, year, genre, duration_ms, sample_rate, channels, codec,
                        artwork_hash, added_at
                 FROM tracks ORDER BY id",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let path: Vec<u8> = r.get(2)?;
                let codec: String = r.get(14)?;
                out.push(Track {
                    id: TrackId(r.get(0)?),
                    folder: FolderId(r.get(1)?),
                    path: decode_path(&path),
                    title: r.get(3)?,
                    artist: strings.opt(r.get(4)?),
                    album: strings.opt(r.get(5)?),
                    album_artist: strings.opt(r.get(6)?),
                    track_no: r
                        .get::<_, Option<i64>>(7)?
                        .and_then(|n| u32::try_from(n).ok()),
                    disc_no: r
                        .get::<_, Option<i64>>(8)?
                        .and_then(|n| u32::try_from(n).ok()),
                    year: r
                        .get::<_, Option<i64>>(9)?
                        .and_then(|n| i32::try_from(n).ok()),
                    genre: strings.opt(r.get(10)?),
                    duration_ms: u64::try_from(r.get::<_, i64>(11)?).unwrap_or(0),
                    sample_rate: u32::try_from(r.get::<_, i64>(12)?).unwrap_or(0),
                    channels: u16::try_from(r.get::<_, i64>(13)?).unwrap_or(0),
                    codec: strings.get(&codec),
                    artwork: r
                        .get::<_, Option<String>>(15)?
                        .map(|h| ArtworkId(strings.get(&h))),
                    added_at: r.get(16)?,
                });
            }
            out
        };
        let folders = self.folders()?;
        tx.commit()?;
        let (albums, artists) = group(&tracks);
        Ok(LibrarySnapshot {
            tracks,
            albums,
            artists,
            folders,
        })
    }
}

type AlbumKey = (Option<String>, Option<String>);

fn group(tracks: &[Track]) -> (Vec<Album>, Vec<Artist>) {
    // Albums: key is (album artist or artist, album), lowercased.
    let mut album_order: Vec<usize> = (0..tracks.len()).collect();
    album_order.sort_by(|&a, &b| {
        let (a, b) = (&tracks[a], &tracks[b]);
        (a.disc_no.is_none(), a.disc_no)
            .cmp(&(b.disc_no.is_none(), b.disc_no))
            .then((a.track_no.is_none(), a.track_no).cmp(&(b.track_no.is_none(), b.track_no)))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    let mut index: HashMap<AlbumKey, usize> = HashMap::new();
    let mut albums: Vec<(AlbumKey, Album)> = Vec::new();
    for &i in &album_order {
        let t = &tracks[i];
        let artist = t.album_artist.as_ref().or(t.artist.as_ref());
        let key = (lower(&artist.cloned()), lower(&t.album));
        let slot = *index.entry(key.clone()).or_insert_with(|| {
            albums.push((
                key,
                Album {
                    title: t.album.clone(),
                    artist: artist.cloned(),
                    tracks: Vec::new(),
                    artwork: None,
                    year: None,
                },
            ));
            albums.len() - 1
        });
        let album = &mut albums[slot].1;
        album.tracks.push(t.id);
        if album.artwork.is_none() {
            album.artwork.clone_from(&t.artwork);
        }
        if album.year.is_none() {
            album.year = t.year;
        }
    }
    albums.sort_by(|(ka, a), (kb, b)| {
        none_last(&ka.0)
            .cmp(&none_last(&kb.0))
            .then_with(|| none_last(&ka.1).cmp(&none_last(&kb.1)))
            .then(a.tracks.first().cmp(&b.tracks.first()))
    });

    // Artists: by track artist, lowercased; tracks are already in id order.
    let mut artist_index: HashMap<Option<String>, usize> = HashMap::new();
    let mut artists: Vec<(Option<String>, Artist)> = Vec::new();
    for t in tracks {
        let key = lower(&t.artist);
        let slot = *artist_index.entry(key.clone()).or_insert_with(|| {
            artists.push((
                key,
                Artist {
                    name: t.artist.clone(),
                    tracks: Vec::new(),
                },
            ));
            artists.len() - 1
        });
        artists[slot].1.tracks.push(t.id);
    }
    artists.sort_by(|(a, _), (b, _)| none_last(a).cmp(&none_last(b)));

    (
        albums.into_iter().map(|(_, a)| a).collect(),
        artists.into_iter().map(|(_, a)| a).collect(),
    )
}
