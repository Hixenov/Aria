//! What the UI knows of Spotify: the account, the library, the search, what is playing and the
//! devices, as the backend last reported them; and the covers fetched for it.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aria_core::SpDevice;
use aria_core::event::ActivePlayer;
use aria_core::i18n::Localizer;
use aria_core::spotify_player::AriaPlayerStatus;
use aria_core::spotify_service::{
    Cover, SpotifyAccount, SpotifyEvent, SpotifyLibrary, SpotifyPlayback, SpotifyRequest,
    SpotifySearch,
};
use eframe::egui;

use crate::ui::pages::PageAction;
use crate::ui::vinyl::finish::{CoverColours, cover_colours};

/// The most covers kept as textures; past it the oldest go.
const MAX_COVERS: usize = 400;
/// A cover asked for and not received is asked for again after this long (the service drops
/// requests when its queue is full).
const ASK_AGAIN: Duration = Duration::from_secs(10);

/// The Spotify state the pages and the player bar draw from.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotifyUi {
    pub account: SpotifyAccount,
    /// Whether a sign-in is kept across runs (the system has a keyring).
    pub keyring: bool,
    pub library: Arc<SpotifyLibrary>,
    pub search: Arc<SpotifySearch>,
    /// What is typed in the search field.
    pub search_text: String,
    pub playback: Option<Arc<SpotifyPlayback>>,
    pub devices: Vec<SpDevice>,
    /// The player the bar shows.
    pub active: ActivePlayer,
    /// Aria's own Spotify player window.
    pub player: AriaPlayerStatus,
}

impl Default for SpotifyUi {
    fn default() -> SpotifyUi {
        SpotifyUi {
            account: SpotifyAccount::SignedOut { keyring: true },
            keyring: true,
            library: Arc::default(),
            search: Arc::default(),
            search_text: String::new(),
            playback: None,
            devices: Vec::new(),
            active: ActivePlayer::Local,
            player: AriaPlayerStatus::Off,
        }
    }
}

impl SpotifyUi {
    /// Takes in an event. Covers and notices are the app's to handle; they are ignored here.
    pub fn apply(&mut self, event: SpotifyEvent) {
        match event {
            SpotifyEvent::Account(account) => {
                if let SpotifyAccount::SignedOut { keyring } = account {
                    self.keyring = keyring;
                }
                self.account = account;
            }
            SpotifyEvent::Library(library) => self.library = library,
            SpotifyEvent::Search(search) => self.search = search,
            SpotifyEvent::Playback(playback) => self.playback = playback,
            SpotifyEvent::Devices(devices) => self.devices = devices,
            SpotifyEvent::Player(status) => self.player = status,
            SpotifyEvent::Cover(_) | SpotifyEvent::Notice(_) => {}
        }
    }

    pub fn signed_in(&self) -> bool {
        matches!(self.account, SpotifyAccount::SignedIn { .. })
    }
}

/// Spotify's covers as textures, by URL, asked for once each.
#[derive(Default)]
pub struct RemoteCovers {
    textures: HashMap<String, egui::TextureHandle>,
    /// Oldest first.
    order: VecDeque<String>,
    /// When each missing cover was asked for.
    asked: HashMap<String, Instant>,
    /// Each cover's colours, for the full-screen record.
    colours: HashMap<String, CoverColours>,
    /// The full-screen record's large cover: one, for the item it shows.
    large: Option<LargeCover>,
    /// The large cover asked for, and when.
    large_asked: Option<(String, Instant)>,
}

struct LargeCover {
    url: String,
    texture: egui::TextureHandle,
    colours: CoverColours,
}

impl RemoteCovers {
    /// The cover at `url`, or `None` while it is fetched; the first call asks for it.
    pub fn get(&mut self, url: &str, out: &mut Vec<PageAction>) -> Option<&egui::TextureHandle> {
        if self.textures.contains_key(url) {
            return self.textures.get(url);
        }
        let now = Instant::now();
        let due = self
            .asked
            .get(url)
            .is_none_or(|at| now.saturating_duration_since(*at) >= ASK_AGAIN);
        if due {
            if self.asked.len() >= MAX_COVERS * 4 {
                self.asked.clear();
            }
            self.asked.insert(url.to_owned(), now);
            out.push(PageAction::Spotify(SpotifyRequest::Cover(url.to_owned())));
        }
        None
    }

    /// The large cover at `url` (for the full-screen record), asked for like [`Self::get`].
    pub fn get_large(
        &mut self,
        url: &str,
        out: &mut Vec<PageAction>,
    ) -> Option<&egui::TextureHandle> {
        if self.large.as_ref().is_some_and(|large| large.url == url) {
            return self.large.as_ref().map(|large| &large.texture);
        }
        let now = Instant::now();
        let due = self.large_asked.as_ref().is_none_or(|(asked, at)| {
            asked != url || now.saturating_duration_since(*at) >= ASK_AGAIN
        });
        if due {
            self.large_asked = Some((url.to_owned(), now));
            out.push(PageAction::Spotify(SpotifyRequest::LargeCover(
                url.to_owned(),
            )));
        }
        None
    }

    pub fn insert(&mut self, ctx: &egui::Context, cover: &Cover) {
        let size = [
            usize::try_from(cover.width).unwrap_or(0),
            usize::try_from(cover.height).unwrap_or(0),
        ];
        if size[0] * size[1] * 4 != cover.rgba.len() {
            return;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &cover.rgba);
        let colours = cover_colours(&cover.rgba, cover.width, cover.height);
        if cover.large {
            let texture = ctx.load_texture(
                format!("spotify-large-cover:{}", cover.url),
                image,
                crate::ui::vinyl::draw::TURNING,
            );
            if self
                .large_asked
                .as_ref()
                .is_some_and(|(asked, _)| *asked == cover.url)
            {
                self.large_asked = None;
            }
            self.large = Some(LargeCover {
                url: cover.url.clone(),
                texture,
                colours,
            });
            return;
        }
        let texture = ctx.load_texture(
            format!("spotify-cover:{}", cover.url),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.asked.remove(&cover.url);
        self.colours.insert(cover.url.clone(), colours);
        if self.textures.insert(cover.url.clone(), texture).is_none() {
            self.order.push_back(cover.url.clone());
        }
        while self.order.len() > MAX_COVERS {
            if let Some(oldest) = self.order.pop_front() {
                self.textures.remove(&oldest);
                self.colours.remove(&oldest);
            }
        }
    }

    /// The colours of the cover at `url`, once it has arrived.
    pub fn colours(&self, url: &str) -> Option<CoverColours> {
        match &self.large {
            Some(large) if large.url == url => Some(large.colours),
            _ => self.colours.get(url).copied(),
        }
    }

    /// Forgets every cover (after sign-out).
    pub fn clear(&mut self) {
        self.clear_large();
        self.colours.clear();
        self.textures.clear();
        self.order.clear();
        self.asked.clear();
    }

    /// Whether a large cover is held.
    pub fn has_large(&self) -> bool {
        self.large.is_some()
    }

    /// Forgets the large cover (the full-screen record closed).
    pub fn clear_large(&mut self) {
        self.large = None;
        self.large_asked = None;
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// The page the browser shows after the sign-in, with `text` in it.
/// The texts of Aria's Spotify player page, in the interface's language, as a JSON object.
pub fn player_page_strings(tr: &Localizer) -> String {
    let keys = [
        "player-page-title",
        "player-page-playing",
        "player-page-starting",
        "player-page-ready",
        "player-page-failed",
        "player-page-start",
        "player-page-reload",
    ];
    let texts: serde_json::Map<String, serde_json::Value> = keys
        .into_iter()
        .map(|key| {
            (
                key.to_owned(),
                serde_json::Value::from(tr.tr(key).into_owned()),
            )
        })
        .collect();
    serde_json::Value::Object(texts).to_string()
}

pub fn signed_in_page(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Aria</title></head>\
         <body style=\"font-family:sans-serif;margin:4em;text-align:center\">\
         <h1>Aria</h1><p>{escaped}</p></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_core::spotify_service::SpotifyEvent;

    fn cover(url: &str) -> Cover {
        Cover {
            url: url.to_owned(),
            width: 2,
            height: 2,
            rgba: vec![255; 16],
            large: false,
        }
    }

    fn large(url: &str) -> Cover {
        Cover {
            large: true,
            width: 4,
            height: 4,
            rgba: vec![255; 64],
            ..cover(url)
        }
    }

    #[test]
    fn a_large_cover_is_not_the_small_one() {
        // An item with one image: the bar's 128 px cover of the same address is not enough.
        let ctx = egui::Context::default();
        let mut covers = RemoteCovers::default();
        covers.insert(&ctx, &cover("https://i.scdn.co/a"));
        let mut out = Vec::new();
        assert!(covers.get_large("https://i.scdn.co/a", &mut out).is_none());
        assert_eq!(
            out,
            [PageAction::Spotify(SpotifyRequest::LargeCover(
                "https://i.scdn.co/a".to_owned()
            ))]
        );
        covers.insert(&ctx, &large("https://i.scdn.co/a"));
        let got = covers
            .get_large("https://i.scdn.co/a", &mut out)
            .map(|t| t.size());
        assert_eq!(got, Some([4, 4]));
        let small = covers
            .get("https://i.scdn.co/a", &mut out)
            .map(|t| t.size());
        assert_eq!(small, Some([2, 2]));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn one_large_cover_is_kept() {
        let ctx = egui::Context::default();
        let mut covers = RemoteCovers::default();
        covers.insert(&ctx, &large("https://i.scdn.co/a"));
        covers.insert(&ctx, &large("https://i.scdn.co/b"));
        let mut out = Vec::new();
        assert!(covers.get_large("https://i.scdn.co/a", &mut out).is_none());
        assert!(covers.get_large("https://i.scdn.co/b", &mut out).is_some());
        assert!(covers.colours("https://i.scdn.co/b").is_some());
        assert!(covers.is_empty(), "large covers are not in the small map");
        covers.clear_large();
        assert!(!covers.has_large());
        assert!(covers.colours("https://i.scdn.co/b").is_none());
    }

    #[test]
    fn covers_are_asked_for_once() {
        let mut covers = RemoteCovers::default();
        let mut out = Vec::new();
        assert!(covers.get("https://i.scdn.co/a", &mut out).is_none());
        assert!(covers.get("https://i.scdn.co/a", &mut out).is_none());
        assert_eq!(
            out,
            [PageAction::Spotify(SpotifyRequest::Cover(
                "https://i.scdn.co/a".to_owned()
            ))]
        );
        let ctx = egui::Context::default();
        covers.insert(&ctx, &cover("https://i.scdn.co/a"));
        assert!(covers.get("https://i.scdn.co/a", &mut out).is_some());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn the_oldest_covers_go_past_the_limit() {
        let ctx = egui::Context::default();
        let mut covers = RemoteCovers::default();
        for i in 0..=MAX_COVERS {
            covers.insert(&ctx, &cover(&format!("https://i.scdn.co/{i}")));
        }
        assert_eq!(covers.len(), MAX_COVERS);
        let mut out = Vec::new();
        assert!(covers.get("https://i.scdn.co/0", &mut out).is_none());
        assert!(covers.get("https://i.scdn.co/1", &mut out).is_some());
    }

    #[test]
    fn a_bad_cover_is_dropped() {
        let ctx = egui::Context::default();
        let mut covers = RemoteCovers::default();
        let mut bad = cover("https://i.scdn.co/a");
        bad.rgba.pop();
        covers.insert(&ctx, &bad);
        assert!(covers.is_empty());
    }

    #[test]
    fn events_fill_the_state() {
        let mut ui = SpotifyUi::default();
        ui.apply(SpotifyEvent::Account(SpotifyAccount::SignedOut {
            keyring: false,
        }));
        assert!(!ui.keyring);
        ui.apply(SpotifyEvent::Account(SpotifyAccount::SignedIn {
            name: "Ann".to_owned(),
        }));
        assert!(ui.signed_in());
        assert!(!ui.keyring, "kept from the signed-out state");
        ui.apply(SpotifyEvent::Devices(vec![]));
        assert!(ui.devices.is_empty());
    }

    #[test]
    fn page_strings_are_json_with_every_key() {
        let tr = Localizer::new(aria_core::settings::Language::En).unwrap();
        let strings: serde_json::Value = serde_json::from_str(&player_page_strings(&tr)).unwrap();
        for key in [
            "player-page-title",
            "player-page-playing",
            "player-page-starting",
            "player-page-ready",
            "player-page-failed",
            "player-page-start",
            "player-page-reload",
        ] {
            let text = strings.get(key).and_then(|v| v.as_str()).unwrap_or("");
            assert!(!text.is_empty(), "{key}");
        }
    }

    #[test]
    fn the_player_status_is_kept() {
        let mut ui = SpotifyUi::default();
        assert_eq!(ui.player, AriaPlayerStatus::Off);
        ui.apply(SpotifyEvent::Player(AriaPlayerStatus::Starting));
        assert_eq!(ui.player, AriaPlayerStatus::Starting);
    }

    #[test]
    fn the_signed_in_page_escapes_its_text() {
        let page = signed_in_page("a <b> & c");
        assert!(page.contains("a &lt;b&gt; &amp; c"));
        assert!(page.contains("charset=\"utf-8\""));
    }
}
