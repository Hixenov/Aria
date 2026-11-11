//! What every track row offers: the heart, and the favorite and playlist items of its
//! context menu.

use aria_core::i18n::Localizer;
use aria_library::{Collections, TrackId};
use eframe::egui::Ui;

use super::IconButton;
use crate::ui::{
    pages::{NameFor, PageAction},
    theme::{Palette, icons},
};

/// Edge length of the heart button, and its icon size.
pub const HEART_BUTTON: [f32; 2] = [24.0, 16.0];

/// The favorite item and the "Add to playlist" submenu, for a row's context menu.
pub fn items(
    ui: &mut Ui,
    tr: &Localizer,
    collections: &Collections,
    track: TrackId,
    out: &mut Vec<PageAction>,
) {
    let favorite = collections.is_favorite(track);
    if ui.button(tr.tr(favorite_key(favorite))).clicked() {
        out.push(PageAction::SetFavorite {
            track,
            favorite: !favorite,
        });
    }
    ui.menu_button(tr.tr("track-add-to-playlist"), |ui| {
        for playlist in &collections.playlists {
            if ui.button(&playlist.name).clicked() {
                out.push(PageAction::AddToPlaylist {
                    playlist: playlist.id,
                    track,
                });
            }
        }
        if !collections.playlists.is_empty() {
            ui.separator();
        }
        if ui.button(tr.tr("playlist-new")).clicked() {
            out.push(PageAction::AskPlaylistName(NameFor::New {
                first: Some(track),
            }));
        }
    });
}

/// The heart button. Returns the action when clicked.
///
/// The bundled Phosphor font has only the outline heart, so a favorite shows the same glyph
/// in the accent color.
pub fn heart(
    ui: &mut Ui,
    tr: &Localizer,
    p: &Palette,
    collections: &Collections,
    track: TrackId,
) -> Option<PageAction> {
    let favorite = collections.is_favorite(track);
    let label = tr.tr(favorite_key(favorite));
    let [size, icon] = HEART_BUTTON;
    let button = IconButton::new(icons::HEART, &label, p)
        .size(size, icon)
        .selected(favorite)
        .without_dot();
    ui.add(button).clicked().then_some(PageAction::SetFavorite {
        track,
        favorite: !favorite,
    })
}

/// The key of the item that toggles a track's favorite, by whether it is one now.
pub fn favorite_key(favorite: bool) -> &'static str {
    if favorite {
        "track-unfavorite"
    } else {
        "track-favorite"
    }
}

#[cfg(test)]
mod tests {
    use aria_core::{i18n::Localizer, settings::Language};
    use aria_library::{Collections, Playlist, PlaylistId, TrackId};
    use eframe::egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{NodeT as _, Queryable as _},
    };

    use super::*;

    fn playlist(id: i64, name: &str) -> Playlist {
        Playlist {
            id: PlaylistId(id),
            name: name.to_owned(),
            items: Vec::new(),
            unavailable: 0,
        }
    }

    /// A harness drawing the items for track 1 in a plain `Ui`.
    fn menu(collections: Collections) -> Harness<'static, Vec<PageAction>> {
        let tr = Localizer::new(Language::En).unwrap();
        let mut harness = Harness::new_ui_state(
            move |ui, out: &mut Vec<PageAction>| {
                items(ui, &tr, &collections, TrackId(1), out);
            },
            Vec::new(),
        );
        harness.run();
        harness
    }

    fn english(key: &str) -> String {
        Localizer::new(Language::En).unwrap().tr(key).into_owned()
    }

    #[test]
    fn the_favorite_item_reads_as_what_it_does() {
        let harness = menu(Collections::default());
        let add_label = english("track-favorite");
        let add = harness.get_by_role_and_label(Role::Button, &add_label);
        add.click();
        let mut harness = harness;
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::SetFavorite {
                track: TrackId(1),
                favorite: true,
            }]
        );

        let favorite = Collections::new(vec![TrackId(1)], vec![], vec![]);
        let mut harness = menu(favorite);
        harness
            .get_by_role_and_label(Role::Button, &english("track-unfavorite"))
            .click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::SetFavorite {
                track: TrackId(1),
                favorite: false,
            }]
        );
    }

    #[test]
    fn the_submenu_lists_playlists_in_their_order_then_new() {
        // Not alphabetical: the menu keeps the order it is given.
        let playlists = vec![playlist(2, "Zeta"), playlist(1, "Alpha")];
        let mut harness = menu(Collections::new(vec![], playlists, vec![]));
        harness
            .get_by_role_and_label(Role::Button, &english("track-add-to-playlist"))
            .click();
        harness.run();

        let new = english("playlist-new");
        let labels: Vec<String> = harness
            .query_all_by_role(Role::Button)
            .filter_map(|node| node.accesskit_node().label())
            .filter(|label| ["Zeta", "Alpha", new.as_str()].contains(&label.as_str()))
            .collect();
        assert_eq!(labels, ["Zeta", "Alpha", new.as_str()]);

        harness.get_by_role_and_label(Role::Button, "Alpha").click();
        harness.run();
        harness
            .get_by_role_and_label(Role::Button, &english("track-add-to-playlist"))
            .click();
        harness.run();
        harness.get_by_role_and_label(Role::Button, &new).click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[
                PageAction::AddToPlaylist {
                    playlist: PlaylistId(1),
                    track: TrackId(1),
                },
                PageAction::AskPlaylistName(NameFor::New {
                    first: Some(TrackId(1)),
                }),
            ]
        );
    }

    #[test]
    fn the_heart_toggles_the_favorite() {
        let tr = Localizer::new(Language::En).unwrap();
        let favorite = Collections::new(vec![TrackId(1)], vec![], vec![]);
        let p = crate::ui::theme::Palette::for_theme(
            eframe::egui::Theme::Dark,
            &aria_core::settings::Settings::default(),
        );
        let mut harness = Harness::builder()
            .with_context_setup(crate::ui::theme::fonts::install)
            .build_ui_state(
                move |ui, out: &mut Vec<PageAction>| {
                    out.extend(heart(ui, &tr, &p, &favorite, TrackId(1)));
                },
                Vec::new(),
            );
        harness.run();
        let unfavorite = english("track-unfavorite");
        let button = harness.get_by_role_and_label(Role::Button, &unfavorite);
        assert_eq!(
            button.accesskit_node().toggled(),
            Some(eframe::egui::accesskit::Toggled::True),
            "a favorite shows as one"
        );
        button.click();
        harness.run();
        assert_eq!(
            harness.state(),
            &[PageAction::SetFavorite {
                track: TrackId(1),
                favorite: false,
            }]
        );
    }
}
