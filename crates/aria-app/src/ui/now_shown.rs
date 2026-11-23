//! What plays now, as the full-screen record and the mini-player show it: the title, the
//! artists, and the large and small covers, for local and for Spotify playback.

use std::path::Path;

use aria_core::{NowItem, TrackId};
use eframe::egui::{self, TextureHandle};

use crate::ui::layout::player_bar::{PlayerAction, PlayerBar};
use crate::ui::pages::PageAction;
use crate::ui::vinyl::cover::LocalCover;
use crate::ui::vinyl::finish::CoverColours;

/// What is shown of the playing track.
pub struct Shown {
    pub title: String,
    pub artist: String,
    /// The track's identity: the Spotify URI, or the file's path.
    pub identity: String,
    pub large: Option<TextureHandle>,
    pub small: Option<TextureHandle>,
    pub colours: CoverColours,
}

impl Default for Shown {
    fn default() -> Shown {
        Shown {
            title: String::new(),
            artist: String::new(),
            identity: String::new(),
            large: None,
            small: None,
            colours: CoverColours::neutral(),
        }
    }
}

/// What plays now, and the covers it asks for: a local track's large cover through
/// `local_cover`, a Spotify item's through the bar's covers, whose requests go to `player`.
pub fn what_plays(
    ctx: &egui::Context,
    bar: &mut PlayerBar<'_>,
    local: Option<(TrackId, &Path)>,
    local_cover: &mut LocalCover,
    player: &mut Vec<PlayerAction>,
) -> Shown {
    let mut shown = Shown::default();
    let mut asked = Vec::new();
    if let Some(playback) = bar.spotify {
        match &playback.state.item {
            Some(NowItem::Track(track)) => {
                shown.title.clone_from(&track.name);
                shown.artist = track.artists.join(", ");
                shown.identity = track.uri.clone().unwrap_or_else(|| track.name.clone());
                if let Some(covers) = bar.covers.as_deref_mut() {
                    if let Some(url) = &track.image_large {
                        shown.large = covers.get_large(url, &mut asked).cloned();
                        shown.colours = covers.colours(url).unwrap_or(shown.colours);
                    }
                    if let Some(url) = &track.image {
                        shown.small = covers.get(url, &mut asked).cloned();
                        if track.image_large.is_none() {
                            shown.colours = covers.colours(url).unwrap_or(shown.colours);
                        }
                    }
                }
            }
            Some(NowItem::Other { name }) => {
                shown.title.clone_from(name);
                shown.artist = "Spotify".to_owned();
                shown.identity.clone_from(name);
            }
            None => {}
        }
    } else if let (Some(now), Some(id)) = (bar.now, bar.view.current) {
        shown.title.clone_from(&now.title);
        shown.artist.clone_from(&now.artist);
        match local {
            Some((key, path)) if key == id => {
                shown.identity = path.to_string_lossy().into_owned();
                local_cover.request(ctx, key, path.to_path_buf());
                shown.large = local_cover.texture(ctx, key).cloned();
                shown.colours = local_cover.colours(key).unwrap_or(shown.colours);
            }
            _ => shown.identity = format!("{}", id.0),
        }
        shown.small = now
            .artwork
            .as_ref()
            .and_then(|art| bar.artwork.as_deref_mut()?.get(art).cloned());
    }
    player.extend(asked.into_iter().filter_map(|action| match action {
        PageAction::Spotify(request) => Some(PlayerAction::Spotify(request)),
        _ => None,
    }));
    shown
}
