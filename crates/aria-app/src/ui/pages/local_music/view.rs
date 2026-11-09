//! The visible track list: a scope, a search and a sort applied to a library snapshot, and the
//! result kept until one of them changes.

use std::{
    cmp::Ordering,
    sync::{Arc, Weak},
};

use aria_library::{LibrarySnapshot, Track, TrackId};

/// What the track list is sorted by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Title,
    Artist,
    Album,
    Duration,
    Added,
}

/// Which tracks the list draws from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// The whole library.
    #[default]
    All,
    /// The tracks of `snapshot.albums[i]`.
    Album(usize),
    /// The tracks of `snapshot.artists[i]`.
    Artist(usize),
}

/// The page's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Tracks,
    Albums,
    Artists,
}

/// The lowercase text of a track that a search and a sort read. Lowercasing 50,000 tracks on
/// every keystroke would be wasted work, so it is done once per snapshot.
#[derive(Clone, Debug)]
struct Lowered {
    title: String,
    artist: Option<String>,
    album: Option<String>,
}

impl Lowered {
    fn of(track: &Track) -> Self {
        Self {
            title: track.title.to_lowercase(),
            artist: track.artist.as_deref().map(str::to_lowercase),
            album: track.album.as_deref().map(str::to_lowercase),
        }
    }

    /// Whether `query`, already lowercase, occurs in the title, artist or album.
    fn contains(&self, query: &str) -> bool {
        self.title.contains(query)
            || self.artist.as_deref().is_some_and(|s| s.contains(query))
            || self.album.as_deref().is_some_and(|s| s.contains(query))
    }
}

/// The tracks to show, as indexes into `snapshot.tracks`.
#[derive(Clone, Debug)]
pub struct TrackView {
    /// Trimmed and lowercase.
    query: String,
    key: SortKey,
    ascending: bool,
    scope: Scope,
    /// The snapshot `lowered` and `rows` were made from. Weak, so that remembering it neither
    /// keeps an old library alive nor lets a new one pass for it: while a weak pointer
    /// exists, the address stays taken.
    source: Weak<LibrarySnapshot>,
    lowered: Vec<Lowered>,
    rows: Vec<usize>,
    /// The query, sort or scope changed since `rows` was made.
    stale: bool,
    #[cfg(test)]
    recomputes: usize,
}

impl Default for TrackView {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackView {
    /// Everything, by title.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            key: SortKey::default(),
            ascending: true,
            scope: Scope::default(),
            source: Weak::new(),
            lowered: Vec::new(),
            rows: Vec::new(),
            stale: true,
            #[cfg(test)]
            recomputes: 0,
        }
    }

    /// Keeps only the tracks whose title, artist or album contains `q`, ignoring case.
    pub fn set_query(&mut self, q: &str) {
        let q = q.trim().to_lowercase();
        if q != self.query {
            self.query = q;
            self.stale = true;
        }
    }

    /// Sorts by `key`. Tracks that have no value for it come last, either way round.
    pub fn set_sort(&mut self, key: SortKey, ascending: bool) {
        if (key, ascending) != (self.key, self.ascending) {
            (self.key, self.ascending) = (key, ascending);
            self.stale = true;
        }
    }

    /// Draws from `scope` instead of the whole library.
    pub fn set_scope(&mut self, scope: Scope) {
        if scope != self.scope {
            self.scope = scope;
            self.stale = true;
        }
    }

    /// The sort in effect: the key, and whether it is ascending.
    pub fn sort(&self) -> (SortKey, bool) {
        (self.key, self.ascending)
    }

    /// The scope in effect.
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Whether a search is in effect.
    pub fn is_filtered(&self) -> bool {
        !self.query.is_empty()
    }

    /// The tracks to show, in order, as indexes into `snapshot.tracks`. Redone only when the
    /// query, sort or scope changed, or `snapshot` is not the one it was last made from.
    pub fn rows(&mut self, snapshot: &Arc<LibrarySnapshot>) -> &[usize] {
        if !Weak::ptr_eq(&self.source, &Arc::downgrade(snapshot)) {
            self.source = Arc::downgrade(snapshot);
            self.lowered = snapshot.tracks.iter().map(Lowered::of).collect();
            self.stale = true;
        }
        if self.stale {
            self.rows = self.compute(snapshot);
            self.stale = false;
            #[cfg(test)]
            {
                self.recomputes += 1;
            }
        }
        &self.rows
    }

    fn compute(&self, snapshot: &LibrarySnapshot) -> Vec<usize> {
        let mut rows = match self.scope {
            Scope::All => (0..snapshot.tracks.len()).collect(),
            Scope::Album(i) => positions(snapshot, snapshot.albums.get(i).map(|a| &a.tracks[..])),
            Scope::Artist(i) => positions(snapshot, snapshot.artists.get(i).map(|a| &a.tracks[..])),
        };
        if !self.query.is_empty() {
            rows.retain(|&i| self.lowered[i].contains(&self.query));
        }
        // A stable sort, with the direction inside the comparison: reversing the result
        // instead would also reverse the order of equal tracks.
        rows.sort_by(|&a, &b| self.compare(snapshot, a, b));
        rows
    }

    fn compare(&self, snapshot: &LibrarySnapshot, a: usize, b: usize) -> Ordering {
        let (lower_a, lower_b) = (&self.lowered[a], &self.lowered[b]);
        let (track_a, track_b) = (&snapshot.tracks[a], &snapshot.tracks[b]);
        let up = self.ascending;
        match self.key {
            SortKey::Title => directed(lower_a.title.cmp(&lower_b.title), up),
            SortKey::Artist => missing_last(lower_a.artist.as_ref(), lower_b.artist.as_ref(), up),
            SortKey::Album => missing_last(lower_a.album.as_ref(), lower_b.album.as_ref(), up),
            // A length of 0 is one nobody has learned yet.
            SortKey::Duration => missing_last(
                Some(track_a.duration_ms).filter(|&ms| ms > 0).as_ref(),
                Some(track_b.duration_ms).filter(|&ms| ms > 0).as_ref(),
                up,
            ),
            SortKey::Added => directed(track_a.added_at.cmp(&track_b.added_at), up),
        }
    }
}

/// The indexes in `snapshot.tracks` of `ids`, in their order. Ids that are gone are skipped.
fn positions(snapshot: &LibrarySnapshot, ids: Option<&[TrackId]>) -> Vec<usize> {
    ids.unwrap_or_default()
        .iter()
        .filter_map(|&id| snapshot.tracks.binary_search_by_key(&id, |t| t.id).ok())
        .collect()
}

fn directed(order: Ordering, ascending: bool) -> Ordering {
    if ascending { order } else { order.reverse() }
}

/// Orders by value, with a missing value after every present one whichever way it goes.
fn missing_last<T: Ord>(a: Option<&T>, b: Option<&T>, ascending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => directed(a.cmp(b), ascending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use aria_library::{Album, Artist, FolderId};

    use super::*;

    fn track(id: i64, title: &str, artist: Option<&str>, album: Option<&str>, ms: u64) -> Track {
        Track {
            id: TrackId(id),
            folder: FolderId(1),
            path: format!("{id}.flac").into(),
            title: title.to_owned(),
            artist: artist.map(Arc::from),
            album: album.map(Arc::from),
            album_artist: None,
            track_no: None,
            disc_no: None,
            year: None,
            genre: None,
            duration_ms: ms,
            sample_rate: 44_100,
            channels: 2,
            codec: Arc::from("flac"),
            artwork: None,
            added_at: id,
        }
    }

    fn snapshot(tracks: Vec<Track>) -> Arc<LibrarySnapshot> {
        Arc::new(LibrarySnapshot {
            tracks,
            ..LibrarySnapshot::default()
        })
    }

    /// The ids of the rows, in order.
    fn ids(view: &mut TrackView, snapshot: &Arc<LibrarySnapshot>) -> Vec<i64> {
        view.rows(snapshot)
            .iter()
            .map(|&i| snapshot.tracks[i].id.0)
            .collect()
    }

    #[test]
    fn search_matches_cyrillic_and_armenian_ignoring_case() {
        let snap = snapshot(vec![
            track(1, "Привет, мир", Some("Земфира"), None, 1000),
            track(2, "Other", Some("Someone"), Some("Ալբոմ Հայաստան"), 1000),
            track(3, "Nothing", None, None, 1000),
        ]);
        let mut view = TrackView::new();
        view.set_query("ПРИВЕТ");
        assert_eq!(ids(&mut view, &snap), [1]);
        view.set_query("земфира");
        assert_eq!(ids(&mut view, &snap), [1], "the artist is searched too");
        view.set_query("ՀԱՅԱՍՏԱՆ");
        assert_eq!(ids(&mut view, &snap), [2], "the album is searched too");
        view.set_query("  someone ");
        assert_eq!(ids(&mut view, &snap), [2], "the query is trimmed");
        view.set_query("");
        assert_eq!(ids(&mut view, &snap), [3, 2, 1], "by title");
    }

    #[test]
    fn sort_puts_missing_values_last() {
        let snap = snapshot(vec![
            track(1, "a", None, None, 0),
            track(2, "b", Some("Zed"), Some("Zoo"), 5000),
            track(3, "c", Some("abe"), Some("Apple"), 1000),
        ]);
        let mut view = TrackView::new();
        for ascending in [true, false] {
            view.set_sort(SortKey::Artist, ascending);
            let order = ids(&mut view, &snap);
            assert_eq!(order.last(), Some(&1), "artist, ascending {ascending}");
            view.set_sort(SortKey::Album, ascending);
            assert_eq!(
                ids(&mut view, &snap).last(),
                Some(&1),
                "album, ascending {ascending}"
            );
            view.set_sort(SortKey::Duration, ascending);
            assert_eq!(
                ids(&mut view, &snap).last(),
                Some(&1),
                "duration, ascending {ascending}"
            );
        }
        // Without regard to case, and each way round.
        view.set_sort(SortKey::Artist, true);
        assert_eq!(ids(&mut view, &snap), [3, 2, 1]);
        view.set_sort(SortKey::Artist, false);
        assert_eq!(ids(&mut view, &snap), [2, 3, 1]);
    }

    #[test]
    fn sort_is_stable() {
        let snap = snapshot(vec![
            track(1, "x", Some("Same"), None, 1000),
            track(2, "y", Some("same"), None, 1000),
            track(3, "z", Some("Other"), None, 1000),
            track(4, "w", Some("SAME"), None, 1000),
        ]);
        let mut view = TrackView::new();
        view.set_sort(SortKey::Artist, true);
        assert_eq!(ids(&mut view, &snap), [3, 1, 2, 4]);
        view.set_sort(SortKey::Artist, false);
        assert_eq!(
            ids(&mut view, &snap),
            [1, 2, 4, 3],
            "equal tracks must keep their order when descending"
        );
        view.set_sort(SortKey::Duration, true);
        assert_eq!(ids(&mut view, &snap), [1, 2, 3, 4]);
    }

    #[test]
    fn rows_are_cached_until_something_changes() {
        let snap = snapshot(vec![
            track(1, "a", Some("x"), None, 1000),
            track(2, "b", Some("y"), None, 2000),
        ]);
        let mut view = TrackView::new();
        view.rows(&snap);
        assert_eq!(view.recomputes, 1);
        view.rows(&snap);
        assert_eq!(view.recomputes, 1, "the same inputs were computed again");

        // Setting what is already set changes nothing.
        view.set_query("");
        view.set_query("   ");
        view.set_sort(SortKey::Title, true);
        view.set_scope(Scope::All);
        view.rows(&snap);
        assert_eq!(view.recomputes, 1, "an unchanged setting recomputed");

        view.set_query("a");
        view.rows(&snap);
        assert_eq!(view.recomputes, 2, "a new query");
        view.set_sort(SortKey::Added, false);
        view.rows(&snap);
        assert_eq!(view.recomputes, 3, "a new sort");
        view.set_scope(Scope::Album(0));
        view.rows(&snap);
        assert_eq!(view.recomputes, 4, "a new scope");

        // An equal snapshot in another Arc is a new snapshot.
        let same = Arc::new((*snap).clone());
        view.rows(&same);
        assert_eq!(view.recomputes, 5, "a new snapshot");
        view.rows(&same);
        assert_eq!(view.recomputes, 5);
    }

    #[test]
    fn a_new_snapshot_is_searched_with_its_own_text() {
        let old = snapshot(vec![track(1, "old song", None, None, 1000)]);
        let new = snapshot(vec![track(1, "new song", None, None, 1000)]);
        let mut view = TrackView::new();
        view.set_query("old");
        assert_eq!(ids(&mut view, &old), [1]);
        assert!(
            ids(&mut view, &new).is_empty(),
            "the lowercase cache was kept"
        );
    }

    #[test]
    fn scope_limits_rows() {
        let mut snap = LibrarySnapshot {
            tracks: vec![
                track(1, "one", Some("A"), Some("X"), 1000),
                track(2, "two", Some("B"), Some("Y"), 1000),
                track(3, "three", Some("A"), Some("X"), 1000),
            ],
            ..LibrarySnapshot::default()
        };
        snap.albums = vec![Album {
            title: Some(Arc::from("X")),
            artist: Some(Arc::from("A")),
            tracks: vec![TrackId(1), TrackId(3), TrackId(99)],
            artwork: None,
            year: None,
        }];
        snap.artists = vec![Artist {
            name: Some(Arc::from("B")),
            tracks: vec![TrackId(2)],
        }];
        let snap = Arc::new(snap);
        let mut view = TrackView::new();
        assert_eq!(
            ids(&mut view, &snap),
            [1, 3, 2],
            "by title: one, three, two"
        );

        view.set_scope(Scope::Album(0));
        assert_eq!(
            ids(&mut view, &snap),
            [1, 3],
            "a track that is gone is skipped"
        );
        view.set_query("thr");
        assert_eq!(
            ids(&mut view, &snap),
            [3],
            "the search applies within the scope"
        );
        view.set_query("");
        view.set_scope(Scope::Artist(0));
        assert_eq!(ids(&mut view, &snap), [2]);
        view.set_scope(Scope::Artist(5));
        assert!(
            ids(&mut view, &snap).is_empty(),
            "a scope that is gone shows nothing"
        );
    }
}
