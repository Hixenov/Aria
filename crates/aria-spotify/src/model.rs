//! What Aria shows of Spotify, and the shape of Spotify's JSON it is read from.
//!
//! The `Raw*` types follow Spotify's Web API reference as of 2026-11 (Development mode: no
//! `popularity`, `followers` or `label`). Unknown fields are ignored and missing lists are
//! empty, so an answer with fewer fields still reads.

use serde::Deserialize;

/// The signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
}

/// A track. `uri` is `None` for a local file (it cannot be played through the Web API).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpTrack {
    pub uri: Option<String>,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_uri: Option<String>,
    pub duration_ms: u64,
    pub image: Option<String>,
    /// The largest cover there is (`https://` only), for the full-screen record.
    pub image_large: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpAlbum {
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpArtist {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpPlaylist {
    pub uri: String,
    pub name: String,
    /// The owner's display name (or id), for showing.
    pub owner: String,
    pub image: Option<String>,
    /// The playlist's id, for the playlist endpoints.
    pub id: String,
    pub owner_id: String,
    /// Whether the owner lets others change it.
    pub collaborative: bool,
    /// The version Spotify last reported, sent along with a removal.
    pub snapshot: Option<String>,
    /// Whether this user may open and change it (owns it, or it is collaborative). Set by
    /// whoever knows the user; `false` as read.
    pub editable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub tracks: Vec<SpTrack>,
    pub albums: Vec<SpAlbum>,
    pub artists: Vec<SpArtist>,
    pub playlists: Vec<SpPlaylist>,
}

impl SearchResults {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
            && self.albums.is_empty()
            && self.artists.is_empty()
            && self.playlists.is_empty()
    }
}

/// One of the account's Spotify devices (a phone, a speaker, the Spotify app).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpDevice {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub restricted: bool,
    pub volume: Option<u8>,
    pub supports_volume: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatState {
    #[default]
    Off,
    Context,
    Track,
}

impl RepeatState {
    pub fn as_str(self) -> &'static str {
        match self {
            RepeatState::Off => "off",
            RepeatState::Context => "context",
            RepeatState::Track => "track",
        }
    }
}

/// What Spotify is playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NowItem {
    Track(SpTrack),
    /// An episode, or anything else that is not a track: shown by its name only.
    Other {
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackState {
    pub device: Option<SpDevice>,
    pub item: Option<NowItem>,
    pub progress_ms: u64,
    pub playing: bool,
    pub shuffle: bool,
    pub repeat: RepeatState,
}

/// What to start playing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayRequest {
    /// An album, artist or playlist; `offset_uri` starts at that track of it.
    Context {
        uri: String,
        offset_uri: Option<String>,
    },
    /// A list of tracks, starting at `start`.
    Tracks { uris: Vec<String>, start: usize },
    /// Go on with what was playing.
    Resume,
}

/// An image Spotify lists for an album, artist or playlist.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Image {
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

/// The smallest image at least 64 pixels wide, else the widest; `https://` only.
pub fn pick_image(images: &[Image]) -> Option<String> {
    let secure = || images.iter().filter(|i| i.url.starts_with("https://"));
    let width = |i: &&Image| i.width.unwrap_or(0);
    secure()
        .filter(|i| width(i) >= 64)
        .min_by_key(width)
        .or_else(|| secure().max_by_key(width))
        .map(|i| i.url.clone())
}

/// The widest image, `https://` only.
pub fn pick_large(images: &[Image]) -> Option<String> {
    images
        .iter()
        .filter(|i| i.url.starts_with("https://"))
        .max_by_key(|i| i.width.unwrap_or(0))
        .map(|i| i.url.clone())
}

// --- Spotify's JSON ---

#[derive(Deserialize)]
pub(crate) struct RawMe {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

impl From<RawMe> for Account {
    fn from(raw: RawMe) -> Account {
        let name = raw
            .display_name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| raw.id.clone());
        Account { id: raw.id, name }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawArtistRef {
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct RawAlbum {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: Vec<RawArtistRef>,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
}

fn names(artists: Vec<RawArtistRef>) -> Vec<String> {
    artists.into_iter().map(|a| a.name).collect()
}

fn image_of(images: Option<Vec<Image>>) -> Option<String> {
    images.as_deref().and_then(pick_image)
}

impl RawAlbum {
    pub fn into_album(self) -> Option<SpAlbum> {
        Some(SpAlbum {
            uri: self.uri?,
            name: self.name,
            artists: names(self.artists),
            image: image_of(self.images),
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct RawTrack {
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artists: Vec<RawArtistRef>,
    #[serde(default)]
    pub album: Option<RawAlbum>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub is_local: bool,
}

impl From<RawTrack> for SpTrack {
    fn from(raw: RawTrack) -> SpTrack {
        let (album, album_uri, image, image_large) = match raw.album {
            Some(a) => {
                let large = a.images.as_deref().and_then(pick_large);
                (a.name, a.uri, image_of(a.images), large)
            }
            None => (String::new(), None, None, None),
        };
        let local = raw.is_local;
        SpTrack {
            uri: raw.uri.filter(|_| !local),
            name: raw.name,
            artists: names(raw.artists),
            album,
            album_uri: album_uri.filter(|_| !local),
            duration_ms: raw.duration_ms,
            image,
            image_large,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawArtist {
    pub uri: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
}

impl From<RawArtist> for SpArtist {
    fn from(raw: RawArtist) -> SpArtist {
        SpArtist {
            uri: raw.uri,
            name: raw.name,
            image: image_of(raw.images),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawOwner {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// A simplified playlist. Its item count (`tracks`, renamed `items` in 2026) is not used.
#[derive(Deserialize)]
pub(crate) struct RawPlaylist {
    pub uri: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub owner: Option<RawOwner>,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
    #[serde(default)]
    pub collaborative: Option<bool>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
}

impl From<RawPlaylist> for SpPlaylist {
    fn from(raw: RawPlaylist) -> SpPlaylist {
        let owner_id = raw.owner.as_ref().map(|o| o.id.clone()).unwrap_or_default();
        let owner = raw
            .owner
            .map(|o| o.display_name.filter(|n| !n.is_empty()).unwrap_or(o.id))
            .unwrap_or_default();
        let id = if raw.id.is_empty() {
            raw.uri
                .strip_prefix("spotify:playlist:")
                .unwrap_or_default()
                .to_owned()
        } else {
            raw.id
        };
        SpPlaylist {
            uri: raw.uri,
            name: raw.name,
            owner,
            image: image_of(raw.images),
            id,
            owner_id,
            collaborative: raw.collaborative.unwrap_or(false),
            snapshot: raw.snapshot_id.filter(|s| !s.is_empty()),
            editable: false,
        }
    }
}

/// One entry of a playlist's items: the track is in `item` since 2026, in `track` before.
#[derive(Deserialize)]
pub(crate) struct PlaylistEntry {
    #[serde(default)]
    pub item: Option<RawTrack>,
    #[serde(default)]
    pub track: Option<RawTrack>,
}

impl PlaylistEntry {
    /// The track, if it is one: episodes and empty entries give `None`. A local file is kept
    /// (without a URI).
    pub fn into_track(self) -> Option<SpTrack> {
        let raw = self.item.or(self.track)?;
        let track = raw.is_local
            || raw
                .uri
                .as_deref()
                .is_some_and(|uri| uri.starts_with("spotify:track:"));
        track.then(|| SpTrack::from(raw))
    }
}

/// What a playlist change answers.
#[derive(Deserialize)]
pub(crate) struct SnapshotAnswer {
    #[serde(default)]
    pub snapshot_id: Option<String>,
}

/// An entry of a list that never fails its page: one Spotify cannot describe as Aria expects
/// (`null`, or a field of the wrong type) is `None` and is skipped.
pub(crate) struct Lenient<T>(pub Option<T>);

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        Ok(Lenient(serde_json::from_value(value).ok()))
    }
}

/// A page of a list. Entries Aria cannot read are skipped.
#[derive(Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub(crate) struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<Lenient<T>>,
    #[serde(default)]
    pub next: Option<String>,
}

impl<T> Default for Page<T> {
    fn default() -> Page<T> {
        Page {
            items: Vec::new(),
            next: None,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct SavedTrack {
    pub track: Option<RawTrack>,
}

#[derive(Deserialize)]
pub(crate) struct SavedAlbum {
    pub album: Option<RawAlbum>,
}

#[derive(Deserialize)]
pub(crate) struct Following {
    pub artists: Page<RawArtist>,
}

#[derive(Deserialize)]
pub(crate) struct RawSearch {
    #[serde(default)]
    pub tracks: Page<RawTrack>,
    #[serde(default)]
    pub albums: Page<RawAlbum>,
    #[serde(default)]
    pub artists: Page<RawArtist>,
    #[serde(default)]
    pub playlists: Page<RawPlaylist>,
}

impl From<RawSearch> for SearchResults {
    fn from(raw: RawSearch) -> SearchResults {
        SearchResults {
            tracks: raw
                .tracks
                .items
                .into_iter()
                .filter_map(|item| item.0)
                .map(SpTrack::from)
                .collect(),
            albums: raw
                .albums
                .items
                .into_iter()
                .filter_map(|item| item.0)
                .filter_map(RawAlbum::into_album)
                .collect(),
            artists: raw
                .artists
                .items
                .into_iter()
                .filter_map(|item| item.0)
                .map(SpArtist::from)
                .collect(),
            playlists: raw
                .playlists
                .items
                .into_iter()
                .filter_map(|item| item.0)
                .map(SpPlaylist::from)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct RawDevice {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub is_restricted: bool,
    #[serde(default)]
    pub volume_percent: Option<u8>,
    #[serde(default)]
    pub supports_volume: bool,
}

impl From<RawDevice> for SpDevice {
    fn from(raw: RawDevice) -> SpDevice {
        SpDevice {
            id: raw.id,
            name: raw.name,
            kind: raw.kind,
            active: raw.is_active,
            restricted: raw.is_restricted,
            volume: raw.volume_percent,
            supports_volume: raw.supports_volume,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct Devices {
    #[serde(default)]
    pub devices: Vec<RawDevice>,
}

#[derive(Deserialize)]
pub(crate) struct RawPlayer {
    #[serde(default)]
    pub device: Option<RawDevice>,
    #[serde(default)]
    pub progress_ms: Option<u64>,
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default)]
    pub shuffle_state: bool,
    #[serde(default)]
    pub repeat_state: RepeatState,
    #[serde(default)]
    pub item: Option<serde_json::Value>,
}

impl From<RawPlayer> for PlaybackState {
    fn from(raw: RawPlayer) -> PlaybackState {
        let item = raw.item.map(|value| {
            let is_track = value.get("type").and_then(|t| t.as_str()) == Some("track");
            let name = value
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_owned();
            if is_track {
                match serde_json::from_value::<RawTrack>(value) {
                    Ok(track) => NowItem::Track(track.into()),
                    Err(_) => NowItem::Other { name },
                }
            } else {
                NowItem::Other { name }
            }
        });
        PlaybackState {
            device: raw.device.map(SpDevice::from),
            item,
            progress_ms: raw.progress_ms.unwrap_or(0),
            playing: raw.is_playing,
            shuffle: raw.shuffle_state,
            repeat: raw.repeat_state,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Deserialize)]
pub(crate) struct ErrorDetail {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub reason: Option<String>,
}
