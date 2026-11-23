//! What the UI knows of Spotify: the account, the library, the search, what is playing and the
//! devices, as the backend last reported them; and the covers fetched for it.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aria_core::SpDevice;
use aria_core::event::ActivePlayer;
use aria_core::spotify_service::{
    Cover, SpotifyAccount, SpotifyEvent, SpotifyLibrary, SpotifyPlayback, SpotifyRequest,
    SpotifySearch,
};
use eframe::egui;

use crate::ui::pages::PageAction;

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
        let mut asked = Vec::new();
        let found = self.get(url, &mut asked).is_some();
        out.extend(asked.into_iter().map(|action| match action {
            PageAction::Spotify(SpotifyRequest::Cover(url)) => {
                PageAction::Spotify(SpotifyRequest::LargeCover(url))
            }
            other => other,
        }));
        if found { self.textures.get(url) } else { None }
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
        let texture = ctx.load_texture(
            format!("spotify-cover:{}", cover.url),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.asked.remove(&cover.url);
        if self.textures.insert(cover.url.clone(), texture).is_none() {
            self.order.push_back(cover.url.clone());
        }
        while self.order.len() > MAX_COVERS {
            if let Some(oldest) = self.order.pop_front() {
                self.textures.remove(&oldest);
            }
        }
    }

    /// Forgets every cover (after sign-out).
    pub fn clear(&mut self) {
        self.textures.clear();
        self.order.clear();
        self.asked.clear();
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

/// The page the browser shows after the sign-in, with `text` in it.
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
        }
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
    fn the_signed_in_page_escapes_its_text() {
        let page = signed_in_page("a <b> & c");
        assert!(page.contains("a &lt;b&gt; &amp; c"));
        assert!(page.contains("charset=\"utf-8\""));
    }
}
