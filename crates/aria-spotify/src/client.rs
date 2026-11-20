//! The Web API client: the library, search, and remote control of Spotify's devices.
//!
//! Every API call carries `Authorization: Bearer <access>`; the token is refreshed first when it
//! has expired, and once more on a `401`. Paging follows Spotify's `next` links, but only while
//! they point at the Web API itself, so the token never goes anywhere else.

use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::http::{self, Answer};
use crate::model::{
    Account, Devices, ErrorBody, Following, Page, PlayRequest, PlaybackState, RawAlbum, RawMe,
    RawPlayer, RawPlaylist, RawSearch, RepeatState, SavedAlbum, SavedTrack, SearchResults, SpAlbum,
    SpArtist, SpDevice, SpPlaylist, SpTrack,
};
use crate::{Endpoints, Http, SpotifyError, Tokens, token};

/// A `Retry-After` up to this long is waited out once; a longer one is returned.
const SHORT_WAIT: Duration = Duration::from_secs(5);
/// When a `429` has no `Retry-After`.
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
/// No more pages than this for one list (50 000 items).
const MAX_PAGES: usize = 1000;
const MAX_IMAGE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Method {
    Get,
    Put,
    Post,
}

/// A signed-in session's client. `Debug` shows no token.
#[derive(Debug)]
pub struct Client {
    http: Http,
    endpoints: Endpoints,
    tokens: Tokens,
}

impl Client {
    pub fn new(http: Http, endpoints: Endpoints, tokens: Tokens) -> Client {
        Client {
            http,
            endpoints,
            tokens,
        }
    }

    /// The current tokens, so a rotated refresh token can be saved.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    pub fn me(&mut self) -> Result<Account, SpotifyError> {
        let url = self.url("/me", &[]);
        Ok(self.get::<RawMe>(&url)?.into())
    }

    pub fn saved_tracks(&mut self) -> Result<Vec<SpTrack>, SpotifyError> {
        let first = self.url("/me/tracks", &[("limit", "50")]);
        let saved: Vec<SavedTrack> = self.all_pages(first, |page| page)?;
        Ok(saved
            .into_iter()
            .filter_map(|s| s.track)
            .map(SpTrack::from)
            .collect())
    }

    pub fn saved_albums(&mut self) -> Result<Vec<SpAlbum>, SpotifyError> {
        let first = self.url("/me/albums", &[("limit", "50")]);
        let saved: Vec<SavedAlbum> = self.all_pages(first, |page| page)?;
        Ok(saved
            .into_iter()
            .filter_map(|s| s.album)
            .filter_map(RawAlbum::into_album)
            .collect())
    }

    pub fn followed_artists(&mut self) -> Result<Vec<SpArtist>, SpotifyError> {
        let first = self.url("/me/following", &[("type", "artist"), ("limit", "50")]);
        let artists = self.all_pages(first, |following: Following| following.artists)?;
        Ok(artists.into_iter().map(SpArtist::from).collect())
    }

    pub fn playlists(&mut self) -> Result<Vec<SpPlaylist>, SpotifyError> {
        let first = self.url("/me/playlists", &[("limit", "50")]);
        let playlists: Vec<RawPlaylist> = self.all_pages(first, |page| page)?;
        Ok(playlists.into_iter().map(SpPlaylist::from).collect())
    }

    /// Up to 10 of each kind: Spotify's limit for Development-mode apps.
    pub fn search(&mut self, query: &str) -> Result<SearchResults, SpotifyError> {
        let url = self.url(
            "/search",
            &[
                ("q", query),
                ("type", "track,album,artist,playlist"),
                ("limit", "10"),
            ],
        );
        Ok(self.get::<RawSearch>(&url)?.into())
    }

    /// What is playing, or `None` when nothing is (`204`).
    pub fn player(&mut self) -> Result<Option<PlaybackState>, SpotifyError> {
        let url = self.url("/me/player", &[("additional_types", "episode")]);
        let answer = self.send(Method::Get, &url, None)?;
        if answer.status == 204 || answer.body.trim().is_empty() {
            return Ok(None);
        }
        let raw: RawPlayer = parse(&answer.body)?;
        Ok(Some(raw.into()))
    }

    pub fn devices(&mut self) -> Result<Vec<SpDevice>, SpotifyError> {
        let url = self.url("/me/player/devices", &[]);
        let devices: Devices = self.get(&url)?;
        Ok(devices.devices.into_iter().map(SpDevice::from).collect())
    }

    pub fn play(
        &mut self,
        device: Option<&str>,
        request: &PlayRequest,
    ) -> Result<(), SpotifyError> {
        let body = match request {
            PlayRequest::Context { uri, offset_uri } => Some(match offset_uri {
                Some(offset) => json!({ "context_uri": uri, "offset": { "uri": offset } }),
                None => json!({ "context_uri": uri }),
            }),
            PlayRequest::Tracks { uris, start } => {
                Some(json!({ "uris": uris, "offset": { "position": start } }))
            }
            PlayRequest::Resume => None,
        };
        self.command(Method::Put, "/me/player/play", device, &[], body)
    }

    pub fn pause(&mut self, device: Option<&str>) -> Result<(), SpotifyError> {
        self.command(Method::Put, "/me/player/pause", device, &[], None)
    }

    pub fn next(&mut self, device: Option<&str>) -> Result<(), SpotifyError> {
        self.command(Method::Post, "/me/player/next", device, &[], None)
    }

    pub fn previous(&mut self, device: Option<&str>) -> Result<(), SpotifyError> {
        self.command(Method::Post, "/me/player/previous", device, &[], None)
    }

    pub fn seek(&mut self, device: Option<&str>, position_ms: u64) -> Result<(), SpotifyError> {
        let position = position_ms.to_string();
        let query = [("position_ms", position.as_str())];
        self.command(Method::Put, "/me/player/seek", device, &query, None)
    }

    pub fn volume(&mut self, device: Option<&str>, percent: u8) -> Result<(), SpotifyError> {
        let percent = percent.min(100).to_string();
        let query = [("volume_percent", percent.as_str())];
        self.command(Method::Put, "/me/player/volume", device, &query, None)
    }

    pub fn shuffle(&mut self, device: Option<&str>, on: bool) -> Result<(), SpotifyError> {
        let query = [("state", if on { "true" } else { "false" })];
        self.command(Method::Put, "/me/player/shuffle", device, &query, None)
    }

    pub fn repeat(&mut self, device: Option<&str>, state: RepeatState) -> Result<(), SpotifyError> {
        let query = [("state", state.as_str())];
        self.command(Method::Put, "/me/player/repeat", device, &query, None)
    }

    /// Moves playback to `device_id` (and starts it if `play`).
    pub fn transfer(&mut self, device_id: &str, play: bool) -> Result<(), SpotifyError> {
        let body = json!({ "device_ids": [device_id], "play": play });
        self.command(Method::Put, "/me/player", None, &[], Some(body))
    }

    /// A cover image, fetched without the token. `https://` only, at most 2 MiB.
    pub fn image(&mut self, url: &str) -> Result<Vec<u8>, SpotifyError> {
        fetch_image(&self.http, &self.endpoints, url)
    }

    // --- plumbing ---

    fn url(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut url = format!("{}{path}", self.endpoints.api);
        if !query.is_empty() {
            let encoded = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query.iter().copied())
                .finish();
            url.push('?');
            url.push_str(&encoded);
        }
        url
    }

    fn command(
        &mut self,
        method: Method,
        path: &str,
        device: Option<&str>,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<(), SpotifyError> {
        let mut pairs: Vec<(&str, &str)> = query.to_vec();
        if let Some(device) = device {
            pairs.push(("device_id", device));
        }
        let url = self.url(path, &pairs);
        self.send(method, &url, body).map(|_| ())
    }

    fn get<T: DeserializeOwned>(&mut self, url: &str) -> Result<T, SpotifyError> {
        let answer = self.send(Method::Get, url, None)?;
        parse(&answer.body)
    }

    /// Reads every page from `first` on, following `next` while it stays on the Web API.
    fn all_pages<P: DeserializeOwned, T>(
        &mut self,
        first: String,
        page_of: impl Fn(P) -> Page<T>,
    ) -> Result<Vec<T>, SpotifyError> {
        let own = format!("{}/", self.endpoints.api);
        let mut items = Vec::new();
        let mut next = Some(first);
        let mut pages = 0;
        while let Some(url) = next.take() {
            pages += 1;
            if pages > MAX_PAGES {
                break;
            }
            let page = page_of(self.get::<P>(&url)?);
            items.extend(page.items.into_iter().flatten());
            if let Some(link) = page.next {
                if !link.starts_with(&own) {
                    return Err(SpotifyError::BadAnswer("a next link elsewhere".to_owned()));
                }
                next = Some(link);
            }
        }
        Ok(items)
    }

    fn refresh(&mut self) -> Result<(), SpotifyError> {
        self.tokens = token::refresh(&self.http, &self.endpoints, &self.tokens.refresh)?;
        Ok(())
    }

    fn send(
        &mut self,
        method: Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<Answer, SpotifyError> {
        if Instant::now() >= self.tokens.expires_at {
            self.refresh()?;
        }
        let (mut refreshed, mut waited) = (false, false);
        loop {
            let answer = self.once(method, url, body.as_ref())?;
            match answer.status {
                200..=299 => return Ok(answer),
                401 if !refreshed => {
                    refreshed = true;
                    self.refresh()?;
                }
                401 => return Err(SpotifyError::SessionEnded),
                429 => {
                    let retry_after = answer.retry_after.unwrap_or(DEFAULT_WAIT);
                    let (_, reason) = error_of(&answer.body);
                    if reason.as_deref() == Some("QUOTA_EXCEEDED") {
                        return Err(SpotifyError::QuotaExceeded { retry_after });
                    }
                    if retry_after <= SHORT_WAIT && !waited {
                        waited = true;
                        thread::sleep(retry_after);
                    } else {
                        return Err(SpotifyError::RateLimited { retry_after });
                    }
                }
                status => return Err(api_error(status, &answer.body)),
            }
        }
    }

    fn once(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Answer, SpotifyError> {
        let bearer = format!("Bearer {}", self.tokens.access.expose());
        let agent = self.http.agent();
        let result = match method {
            Method::Get => agent.get(url).header("Authorization", &bearer).call(),
            Method::Put => {
                let request = agent.put(url).header("Authorization", &bearer);
                match body {
                    Some(body) => request.send_json(body),
                    None => request.send_empty(),
                }
            }
            Method::Post => {
                let request = agent.post(url).header("Authorization", &bearer);
                match body {
                    Some(body) => request.send_json(body),
                    None => request.send_empty(),
                }
            }
        };
        http::read(result)
    }
}

/// A cover image, fetched without any token. `https://` only (or the fake's own address in
/// tests), at most 2 MiB.
pub fn fetch_image(http: &Http, endpoints: &Endpoints, url: &str) -> Result<Vec<u8>, SpotifyError> {
    let own = format!("{}/", endpoints.accounts);
    if !url.starts_with("https://") && !url.starts_with(&own) {
        return Err(SpotifyError::BadAnswer(
            "an image not over https".to_owned(),
        ));
    }
    let mut response = http
        .agent()
        .get(url)
        .call()
        .map_err(|e| http::transport(&e))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(SpotifyError::Api {
            status,
            message: "image".to_owned(),
        });
    }
    response
        .body_mut()
        .with_config()
        .limit(MAX_IMAGE)
        .read_to_vec()
        .map_err(|e| http::transport(&e))
}

fn parse<T: DeserializeOwned>(body: &str) -> Result<T, SpotifyError> {
    serde_json::from_str(body).map_err(|e| SpotifyError::BadAnswer(format!("JSON: {e}")))
}

/// Spotify's `error.message` and `error.reason`, or the plain-text body.
fn error_of(body: &str) -> (String, Option<String>) {
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(e) => (e.error.message, e.error.reason),
        Err(_) => (body.trim().chars().take(200).collect(), None),
    }
}

/// Spotify's answer for an account not on a Development-mode app's allow-list: "User not
/// registered in the Developer Dashboard" or "... the user may not be registered."
fn is_not_registered(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not registered") || message.contains("not be registered")
}

fn api_error(status: u16, body: &str) -> SpotifyError {
    let (message, reason) = error_of(body);
    match (status, reason.as_deref()) {
        (403, Some("PREMIUM_REQUIRED")) => SpotifyError::PremiumRequired,
        (404, Some("NO_ACTIVE_DEVICE")) => SpotifyError::NoActiveDevice,
        (403, _) if is_not_registered(&message) => SpotifyError::NotAllowListed,
        _ => SpotifyError::Api { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Secret;
    use crate::fake::{FakeSpotify, Reply};
    use crate::model::{Image, NowItem, pick_image};
    use std::time::{Duration, Instant};

    fn client(fake: &FakeSpotify) -> Client {
        let tokens = Tokens {
            access: Secret::new("access-1".to_owned()),
            refresh: Secret::new("refresh-1".to_owned()),
            expires_at: Instant::now() + Duration::from_secs(3600),
        };
        Client::new(Http::new(), fake.endpoints(), tokens)
    }

    fn json(value: &str) -> serde_json::Value {
        serde_json::from_str(value).unwrap()
    }

    #[test]
    fn me_uses_the_display_name() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
        let me = client(&fake).me().unwrap();
        assert_eq!(me.name, "Aria Tester");
        assert_eq!(me.id, "aria-tester");
    }

    #[test]
    fn paging_follows_next() {
        let fake = FakeSpotify::start();
        for page in ["saved-tracks-1", "saved-tracks-2", "saved-tracks-3"] {
            fake.reply("GET", "/v1/me/tracks", Reply::fixture(200, page));
        }
        let tracks = client(&fake).saved_tracks().unwrap();
        assert_eq!(tracks.len(), 107);
        assert_eq!(tracks[0].name, "Song 0");
        assert_eq!(tracks[106].name, "Song 106");
        assert_eq!(tracks[1].uri.as_deref(), Some("spotify:track:track0001"));
        assert_eq!(tracks[1].album_uri.as_deref(), Some("spotify:album:album1"));
        let seen = fake.requests();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].param("limit"), Some("50"));
        assert_eq!(seen[2].param("offset"), Some("100"));
        assert!(
            seen.iter()
                .all(|s| s.authorization.as_deref() == Some("Bearer access-1"))
        );
    }

    #[test]
    fn saved_albums_and_playlists_are_read() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me/albums", Reply::fixture(200, "saved-albums"));
        fake.reply("GET", "/v1/me/playlists", Reply::fixture(200, "playlists"));
        let mut client = client(&fake);
        let albums = client.saved_albums().unwrap();
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[1].artists, vec!["Band B", "Guest"]);
        assert_eq!(albums[1].image, None);
        let playlists = client.playlists().unwrap();
        assert_eq!(playlists.len(), 2);
        assert_eq!(playlists[0].owner, "Aria Tester");
        assert_eq!(playlists[1].owner, "friend");
        assert_eq!(playlists[1].image, None);
    }

    #[test]
    fn followed_artists_follow_the_cursor() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me/following",
            Reply::fixture(200, "followed-artists"),
        );
        fake.reply(
            "GET",
            "/v1/me/following",
            Reply::fixture(200, "followed-artists-2"),
        );
        let artists = client(&fake).followed_artists().unwrap();
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Band A", "Band B", "Band C"]);
        let seen = fake.requests();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].param("type"), Some("artist"));
        assert_eq!(seen[1].param("after"), Some("artistB"));
    }

    #[test]
    fn search_asks_for_ten_of_four_types() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/search", Reply::fixture(200, "search"));
        let results = client(&fake).search("daft punk & co").unwrap();
        let seen = &fake.requests()[0];
        assert_eq!(seen.param("q"), Some("daft punk & co"));
        assert_eq!(seen.param("limit"), Some("10"));
        assert_eq!(seen.param("type"), Some("track,album,artist,playlist"));
        assert_eq!(results.tracks.len(), 2, "the null entry is skipped");
        assert_eq!(results.albums.len(), 1);
        assert_eq!(results.artists.len(), 1);
        assert_eq!(results.playlists.len(), 1);
    }

    #[test]
    fn nothing_playing_is_none() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me/player", Reply::empty(204));
        assert_eq!(client(&fake).player().unwrap(), None);
        assert_eq!(
            fake.requests()[0].param("additional_types"),
            Some("episode")
        );
    }

    #[test]
    fn the_player_state_is_read() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me/player", Reply::fixture(200, "player"));
        let state = client(&fake).player().unwrap().unwrap();
        assert!(state.playing && state.shuffle);
        assert_eq!(state.repeat, RepeatState::Context);
        assert_eq!(state.progress_ms, 42000);
        assert_eq!(state.device.unwrap().name, "My Phone");
        match state.item {
            Some(NowItem::Track(track)) => assert_eq!(track.name, "Song 1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_episode_or_local_file_is_shown_by_name() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me/player",
            Reply::fixture(200, "player-episode"),
        );
        fake.reply("GET", "/v1/me/player", Reply::fixture(200, "player-local"));
        let mut client = client(&fake);
        let episode = client.player().unwrap().unwrap();
        assert_eq!(
            episode.item,
            Some(NowItem::Other {
                name: "A Podcast Episode".to_owned()
            })
        );
        let local = client.player().unwrap().unwrap();
        match local.item {
            Some(NowItem::Track(track)) => {
                assert_eq!(track.name, "Tape One");
                assert_eq!(track.uri, None);
                assert_eq!(track.image, None);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(local.repeat, RepeatState::Track);
    }

    #[test]
    fn devices_are_listed() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me/player/devices",
            Reply::fixture(200, "devices"),
        );
        let devices = client(&fake).devices().unwrap();
        assert_eq!(devices.len(), 3);
        assert!(devices[0].active && devices[0].supports_volume);
        assert_eq!(devices[0].volume, Some(60));
        assert!(devices[2].restricted && devices[2].id.is_none());
    }

    #[test]
    fn play_sends_the_context_and_offset() {
        let fake = FakeSpotify::start();
        fake.reply("PUT", "/v1/me/player/play", Reply::empty(204));
        let request = PlayRequest::Context {
            uri: "spotify:album:a".to_owned(),
            offset_uri: Some("spotify:track:t".to_owned()),
        };
        client(&fake).play(Some("dev1"), &request).unwrap();
        let seen = &fake.requests()[0];
        assert_eq!(seen.param("device_id"), Some("dev1"));
        assert_eq!(
            json(&seen.body),
            json(r#"{"context_uri":"spotify:album:a","offset":{"uri":"spotify:track:t"}}"#)
        );
    }

    #[test]
    fn play_sends_tracks_with_a_position() {
        let fake = FakeSpotify::start();
        fake.reply("PUT", "/v1/me/player/play", Reply::empty(204));
        let request = PlayRequest::Tracks {
            uris: vec!["spotify:track:a".to_owned(), "spotify:track:b".to_owned()],
            start: 1,
        };
        let mut client = client(&fake);
        client.play(None, &request).unwrap();
        client.play(None, &PlayRequest::Resume).unwrap();
        let seen = fake.requests();
        assert_eq!(seen[0].param("device_id"), None);
        assert_eq!(
            json(&seen[0].body),
            json(r#"{"uris":["spotify:track:a","spotify:track:b"],"offset":{"position":1}}"#)
        );
        assert_eq!(seen[1].body, "", "resume sends no body");
    }

    #[test]
    fn next_and_previous_are_posts() {
        let fake = FakeSpotify::start();
        fake.reply("POST", "/v1/me/player/next", Reply::empty(204));
        fake.reply("POST", "/v1/me/player/previous", Reply::empty(204));
        let mut client = client(&fake);
        client.next(None).unwrap();
        client.previous(Some("d")).unwrap();
        let seen = fake.requests();
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[1].path, "/v1/me/player/previous");
        assert_eq!(seen[1].param("device_id"), Some("d"));
    }

    #[test]
    fn the_other_controls_send_their_values() {
        let fake = FakeSpotify::start();
        for path in ["seek", "volume", "shuffle", "repeat", "pause"] {
            fake.reply("PUT", &format!("/v1/me/player/{path}"), Reply::empty(204));
        }
        fake.reply("PUT", "/v1/me/player", Reply::empty(204));
        let mut client = client(&fake);
        client.seek(None, 61000).unwrap();
        client.volume(None, 35).unwrap();
        client.shuffle(None, true).unwrap();
        client.repeat(None, RepeatState::Track).unwrap();
        client.pause(None).unwrap();
        client.transfer("dev9", true).unwrap();
        let seen = fake.requests();
        assert_eq!(seen[0].param("position_ms"), Some("61000"));
        assert_eq!(seen[1].param("volume_percent"), Some("35"));
        assert_eq!(seen[2].param("state"), Some("true"));
        assert_eq!(seen[3].param("state"), Some("track"));
        assert_eq!(seen[4].path, "/v1/me/player/pause");
        assert_eq!(
            json(&seen[5].body),
            json(r#"{"device_ids":["dev9"],"play":true}"#)
        );
    }

    #[test]
    fn a_401_refreshes_once_and_repeats() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me", Reply::fixture(401, "error-401"));
        fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        let mut client = client(&fake);
        assert_eq!(client.me().unwrap().name, "Aria Tester");
        let seen = fake.requests();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[1].path, "/api/token");
        assert_eq!(
            seen[2].authorization.as_deref(),
            Some("Bearer fixture-access-token-1")
        );
        assert_eq!(client.tokens().refresh.expose(), "fixture-refresh-token-1");

        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me", Reply::fixture(401, "error-401"));
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        assert_eq!(
            super::tests::client(&fake).me().err(),
            Some(SpotifyError::SessionEnded)
        );
    }

    #[test]
    fn an_expired_token_is_refreshed_first() {
        let fake = FakeSpotify::start();
        fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
        fake.reply("POST", "/api/token", Reply::fixture(200, "token"));
        let tokens = Tokens {
            access: Secret::new("old".to_owned()),
            refresh: Secret::new("refresh-1".to_owned()),
            expires_at: Instant::now(),
        };
        Client::new(Http::new(), fake.endpoints(), tokens)
            .me()
            .unwrap();
        let seen = fake.requests();
        assert_eq!(seen[0].path, "/api/token");
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn a_short_retry_after_is_waited_once() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me",
            Reply::fixture(429, "error-rate").header("Retry-After", "1"),
        );
        fake.reply("GET", "/v1/me", Reply::fixture(200, "me"));
        let start = Instant::now();
        assert!(client(&fake).me().is_ok());
        assert!(start.elapsed() >= Duration::from_millis(900));
        assert_eq!(fake.requests().len(), 2);
    }

    #[test]
    fn a_long_retry_after_is_returned() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me",
            Reply::fixture(429, "error-rate").header("Retry-After", "120"),
        );
        assert_eq!(
            client(&fake).me().err(),
            Some(SpotifyError::RateLimited {
                retry_after: Duration::from_secs(120)
            })
        );
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn quota_exceeded_is_its_own_error() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me",
            Reply::fixture(429, "error-quota").header("Retry-After", "3"),
        );
        assert_eq!(
            client(&fake).me().err(),
            Some(SpotifyError::QuotaExceeded {
                retry_after: Duration::from_secs(3)
            })
        );
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn premium_no_device_and_allow_list_errors_are_named() {
        let fake = FakeSpotify::start();
        fake.reply(
            "PUT",
            "/v1/me/player/play",
            Reply::fixture(403, "error-premium"),
        );
        fake.reply(
            "PUT",
            "/v1/me/player/pause",
            Reply::fixture(404, "error-no-device"),
        );
        fake.reply("GET", "/v1/me", Reply::fixture(403, "error-not-registered"));
        fake.reply(
            "GET",
            "/v1/me/player/devices",
            Reply::json(500, r#"{"error":{"status":500,"message":"Server error"}}"#),
        );
        let mut client = client(&fake);
        assert_eq!(
            client.play(None, &PlayRequest::Resume).err(),
            Some(SpotifyError::PremiumRequired)
        );
        assert_eq!(client.pause(None).err(), Some(SpotifyError::NoActiveDevice));
        assert_eq!(client.me().err(), Some(SpotifyError::NotAllowListed));
        assert_eq!(
            client.devices().err(),
            Some(SpotifyError::Api {
                status: 500,
                message: "Server error".to_owned()
            })
        );
    }

    #[test]
    fn images_are_https_only() {
        let fake = FakeSpotify::start();
        assert!(matches!(
            client(&fake).image("http://example.com/x.jpg"),
            Err(SpotifyError::BadAnswer(_))
        ));
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn images_are_fetched_without_the_token() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/image/abc",
            Reply::bytes(200, "image/png", vec![1, 2, 3]),
        );
        let url = format!("{}/image/abc", fake.base());
        assert_eq!(client(&fake).image(&url).unwrap(), vec![1, 2, 3]);
        assert_eq!(fake.requests()[0].authorization, None);
    }

    #[test]
    fn a_next_link_elsewhere_is_not_followed() {
        let fake = FakeSpotify::start();
        fake.reply(
            "GET",
            "/v1/me/albums",
            Reply::json(
                200,
                r#"{"items":[],"next":"https://evil.example/v1/me/albums?offset=50"}"#,
            ),
        );
        assert!(matches!(
            client(&fake).saved_albums(),
            Err(SpotifyError::BadAnswer(_))
        ));
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn pick_image_prefers_the_smallest_over_64() {
        let image = |url: &str, width| Image {
            url: url.to_owned(),
            width: Some(width),
            height: Some(width),
        };
        let images = [
            image("https://i.scdn.co/640", 640),
            image("https://i.scdn.co/300", 300),
            image("http://i.scdn.co/100", 100),
            image("https://i.scdn.co/60", 60),
        ];
        assert_eq!(
            pick_image(&images).as_deref(),
            Some("https://i.scdn.co/300")
        );
        let small = [image("https://a/32", 32), image("https://a/48", 48)];
        assert_eq!(pick_image(&small).as_deref(), Some("https://a/48"));
        assert_eq!(pick_image(&[image("http://a/640", 640)]), None);
    }
}
