//! The floating mini-player: a small window of its own whose cover fills it, with the title,
//! the transport and the seek bar shown over the cover while the pointer is on it.
//!
//! The app owns a [`MiniPlayer`], opens and closes it, and draws it in a viewport of its own
//! from the player bar's data; what the user does comes back as [`MiniAction`]s.

pub mod place;

use std::path::Path;
use std::time::Duration;

use aria_core::TrackId;
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Id, Label, Layout, Mesh, Painter, Pos2, Rect,
    ResizeDirection, RichText, Sense, Shape, TextureHandle, Ui, UiBuilder, ViewportCommand, pos2,
    vec2,
};

use crate::ui::layout::player_bar::{self, PlayerAction, PlayerBar};
use crate::ui::now_shown::{self, Shown};
use crate::ui::theme::fonts::{icon_font, semibold};
use crate::ui::theme::{Palette, icons};
use crate::ui::vinyl::cover::LocalCover;
use crate::ui::widgets::IconButton;

/// The mini-player viewport's name.
pub const MINI_ID: &str = "aria_mini_player";
/// How long the overlay takes to show or hide.
const FADE: f32 = 0.15;
/// The margin inside the window.
const MARGIN: f32 = 12.0;
/// The window buttons' size, and their icons'.
const BUTTON: [f32; 2] = [28.0, 16.0];
/// The resize grip's edge.
const GRIP: f32 = 16.0;
/// The transport and seek bar's height, as the bar's `controls` draws them.
const CONTROLS: f32 = 52.0;
/// The widest the transport and seek bar get.
const CONTROLS_MAX: f32 = 320.0;

/// The mini-player's viewport.
pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of(MINI_ID)
}

/// What the user did in the mini-player. The app carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum MiniAction {
    Player(PlayerAction),
    Close,
    /// Show the main window.
    ShowMain,
    SetOnTop(bool),
}

/// What the mini-player needs beside the player bar.
pub struct MiniInput<'a> {
    /// The current local track and its file, for its large cover.
    pub local: Option<(TrackId, &'a Path)>,
    pub on_top: bool,
    /// Whether it is drawn in a window of its own (not embedded, as in tests): only then does
    /// it read the window's place and close requests, and move or resize it.
    pub own_window: bool,
}

#[derive(Default)]
pub struct MiniPlayer {
    open: bool,
    /// The window's outer rect as the system last reported it.
    window: Option<Rect>,
    local_cover: LocalCover,
}

impl MiniPlayer {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.window = None;
    }

    /// Closes it and lets its large cover go.
    pub fn close(&mut self) {
        self.open = false;
        self.window = None;
        self.local_cover.clear();
    }

    /// The window's outer rect as the system last reported it.
    pub fn window_rect(&self) -> Option<Rect> {
        self.window
    }

    /// Stands for the system reporting where the window is, in tests without a real window.
    #[cfg(test)]
    pub fn report_window(&mut self, rect: Rect) {
        self.window = Some(rect);
    }

    /// Draws the mini-player into `ui`, which is its whole window, and returns what the user
    /// did.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        bar: &mut PlayerBar<'_>,
        input: &MiniInput<'_>,
    ) -> Vec<MiniAction> {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let mut out = Vec::new();
        let mut player = Vec::new();
        if input.own_window {
            if let Some(window) = ui.input(|i| i.viewport().outer_rect) {
                self.window = Some(window);
            }
            if ui.input(|i| i.viewport().close_requested()) {
                out.push(MiniAction::Close);
            }
        }

        // Under everything else, so the buttons take their clicks first.
        let background = ui.interact(rect, Id::unique("aria_mini_background"), Sense::drag());
        if background.drag_started() && input.own_window {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let shown =
            now_shown::what_plays(&ctx, bar, input.local, &mut self.local_cover, &mut player);
        let painter = ui.painter().clone();
        paint_cover(&painter, rect, &shown, bar.palette);
        if shown.title.is_empty() {
            let text = bar.tr.tr("empty-nothing-playing");
            ui.put(
                Rect::from_center_size(rect.center() + vec2(0.0, 40.0), vec2(rect.width(), 24.0)),
                Label::new(RichText::new(&*text).color(bar.palette.text_muted)).selectable(false),
            );
        }

        let hovered = ui.rect_contains_pointer(rect);
        let alpha = ctx.animate_bool_with_time(Id::unique("aria_mini_overlay"), hovered, FADE);
        if alpha > 0.0 {
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.set_opacity(alpha);
                let mut parts = Parts {
                    bar,
                    input,
                    shown: &shown,
                    out: &mut out,
                    player: &mut player,
                };
                overlay(ui, rect, &mut parts);
            });
        }

        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            player.push(player_bar::transport_action(bar, PlayerAction::TogglePlay));
        }
        let playing = bar.spotify.map_or(bar.view.playing, |p| p.state.playing);
        if hovered && playing {
            // The seek bar moves.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        out.extend(player.into_iter().map(MiniAction::Player));
        out
    }
}

/// What the overlay draws from, and where what the user does goes.
struct Parts<'s, 'b, 'i> {
    bar: &'s mut PlayerBar<'b>,
    input: &'s MiniInput<'i>,
    shown: &'s Shown,
    out: &'s mut Vec<MiniAction>,
    player: &'s mut Vec<PlayerAction>,
}

/// The gradients, the title, the transport, the window's buttons and the grip.
fn overlay(ui: &mut Ui, rect: Rect, parts: &mut Parts<'_, '_, '_>) {
    let Parts {
        bar,
        input,
        shown,
        out,
        player,
    } = parts;
    {
        let painter = ui.painter().clone();
        let shade = Color32::from_black_alpha(190);
        let bottom = Rect::from_min_max(
            pos2(rect.left(), rect.top() + rect.height() * 0.25),
            rect.max,
        );
        gradient(&painter, bottom, Color32::TRANSPARENT, shade);
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), 56.0));
        gradient(
            &painter,
            top,
            Color32::from_black_alpha(120),
            Color32::TRANSPARENT,
        );

        // The overlay is light on dark whatever the theme.
        let dark = Palette::for_theme(egui::Theme::Dark, bar.settings);
        let width = (rect.width() - 2.0 * MARGIN).min(CONTROLS_MAX);
        let controls = Rect::from_min_size(
            pos2(
                rect.center().x - width / 2.0,
                rect.bottom() - MARGIN - CONTROLS,
            ),
            vec2(width, CONTROLS),
        );
        {
            let mut over = PlayerBar {
                tr: bar.tr,
                palette: &dark,
                view: bar.view,
                now: bar.now,
                position_ms: bar.position_ms,
                settings: bar.settings,
                queue_open: bar.queue_open,
                mini_open: bar.mini_open,
                artwork: bar.artwork.as_deref_mut(),
                collections: bar.collections,
                spotify: bar.spotify,
                spotify_devices: bar.spotify_devices,
                covers: bar.covers.as_deref_mut(),
            };
            player_bar::controls(ui, controls, &mut over, player, false);
        }

        if !shown.title.is_empty() {
            let text_width = rect.width() - 2.0 * MARGIN;
            let artist_rect = Rect::from_min_size(
                pos2(rect.left() + MARGIN, controls.top() - 22.0),
                vec2(text_width, 18.0),
            );
            let title_rect = Rect::from_min_size(
                pos2(rect.left() + MARGIN, artist_rect.top() - 24.0),
                vec2(text_width, 22.0),
            );
            ui.put(
                title_rect,
                Label::new(
                    RichText::new(&shown.title)
                        .font(semibold(16.0))
                        .color(Color32::WHITE),
                )
                .truncate()
                .selectable(false),
            );
            if !shown.artist.is_empty() {
                ui.put(
                    artist_rect,
                    Label::new(
                        RichText::new(&shown.artist)
                            .size(13.0)
                            .color(Color32::from_white_alpha(190)),
                    )
                    .truncate()
                    .selectable(false),
                );
            }
        }

        let [size, icon] = BUTTON;
        let row = Rect::from_min_size(
            rect.min + vec2(MARGIN / 2.0, MARGIN / 2.0),
            vec2(rect.width() - MARGIN, size),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(row)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                let back = bar.tr.tr("mini-back");
                if ui
                    .add(IconButton::new(icons::ARROW_SQUARE_OUT, &back, &dark).size(size, icon))
                    .clicked()
                {
                    out.push(MiniAction::ShowMain);
                }
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(row)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                let close = bar.tr.tr("mini-close");
                if ui
                    .add(IconButton::new(icons::X, &close, &dark).size(size, icon))
                    .clicked()
                {
                    out.push(MiniAction::Close);
                }
                let pin = bar.tr.tr("mini-on-top");
                let glyph = if input.on_top {
                    icons::PUSH_PIN
                } else {
                    icons::PUSH_PIN_SLASH
                };
                let button = IconButton::new(glyph, &pin, &dark)
                    .size(size, icon)
                    .selected(input.on_top);
                if ui.add(button).clicked() {
                    out.push(MiniAction::SetOnTop(!input.on_top));
                }
            },
        );

        let grip = Rect::from_min_max(rect.max - vec2(GRIP, GRIP), rect.max);
        let response = ui
            .interact(grip, Id::unique("aria_mini_grip"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeSouthEast);
        if response.drag_started() && input.own_window {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(ResizeDirection::SouthEast));
        }
        let stroke = egui::Stroke::new(1.2, Color32::from_white_alpha(150));
        for k in [4.0, 8.0, 12.0] {
            painter.line_segment(
                [
                    pos2(grip.right() - k, grip.bottom() - 2.0),
                    pos2(grip.right() - 2.0, grip.bottom() - k),
                ],
                stroke,
            );
        }
    }
}

/// The cover filling `rect`, cropped to its shape; without one, the placeholder.
fn paint_cover(painter: &Painter, rect: Rect, shown: &Shown, p: &Palette) {
    match shown.large.as_ref().or(shown.small.as_ref()) {
        Some(texture) => paint_filled(painter, rect, texture),
        None => {
            painter.rect_filled(rect, 0.0, p.surface_raised);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                icons::MUSIC_NOTES,
                icon_font((rect.width().min(rect.height()) * 0.25).max(24.0)),
                p.text_muted,
            );
        }
    }
}

/// `texture` over all of `rect`, its middle cut to `rect`'s shape.
fn paint_filled(painter: &Painter, rect: Rect, texture: &TextureHandle) {
    let [w, h] = texture.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let image = w / h;
    let window = (rect.width() / rect.height().max(1.0)).max(f32::EPSILON);
    let (kx, ky) = if image > window {
        (window / image, 1.0)
    } else {
        (1.0, image / window)
    };
    let uv = Rect::from_min_max(
        pos2((1.0 - kx) / 2.0, (1.0 - ky) / 2.0),
        pos2((1.0 + kx) / 2.0, (1.0 + ky) / 2.0),
    );
    painter.image(texture.id(), rect, uv, Color32::WHITE);
}

/// A vertical gradient over `rect`, from `top` to `bottom`.
fn gradient(painter: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    let corners: [(Pos2, Color32); 4] = [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ];
    for (at, colour) in corners {
        mesh.colored_vertex(at, colour);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use aria_core::{
        NowItem, PlaybackState, RepeatState,
        i18n::Localizer,
        playback_service::PlayerView,
        settings::{Language, Settings},
        spotify_service::{SpotifyPlayback, SpotifyRequest},
    };
    use aria_library::Collections;
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable as _};

    use super::*;
    use crate::ui::layout::player_bar::NowPlaying;
    use crate::ui::theme::{Palette, fonts};

    struct State {
        tr: Localizer,
        view: PlayerView,
        now: Option<NowPlaying>,
        spotify: Option<SpotifyPlayback>,
        mini: MiniPlayer,
        on_top: bool,
        out: Vec<MiniAction>,
    }

    fn en(key: &str) -> String {
        Localizer::new(Language::En).unwrap().tr(key).into_owned()
    }

    fn state(view: PlayerView, spotify: Option<SpotifyPlayback>) -> State {
        State {
            tr: Localizer::new(Language::En).unwrap(),
            now: view.current.map(|_| NowPlaying {
                title: "Song".into(),
                artist: "Singer".into(),
                artwork: None,
            }),
            view,
            spotify,
            mini: MiniPlayer::default(),
            on_top: true,
            out: Vec::new(),
        }
    }

    fn local() -> State {
        state(
            PlayerView {
                current: Some(TrackId(1)),
                duration_ms: 60_000,
                ..PlayerView::default()
            },
            None,
        )
    }

    fn spotify() -> State {
        let track = aria_core::SpTrack {
            uri: Some("spotify:track:1".into()),
            name: "Remote".into(),
            artists: vec!["Band".into()],
            album: "Album".into(),
            album_uri: None,
            duration_ms: 60_000,
            image: None,
            image_large: None,
        };
        let playback = SpotifyPlayback {
            state: PlaybackState {
                device: None,
                item: Some(NowItem::Track(track)),
                progress_ms: 0,
                playing: true,
                shuffle: false,
                repeat: RepeatState::Off,
            },
            at: Instant::now(),
        };
        state(PlayerView::default(), Some(playback))
    }

    fn harness(state: State) -> Harness<'static, State> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(300.0, 300.0))
            .with_step_dt(0.05)
            .with_context_setup(fonts::install)
            .build_ui_state(
                |ui, s: &mut State| {
                    let palette = Palette::dark();
                    let settings = Settings::default();
                    let collections = Collections::default();
                    let mut bar = PlayerBar {
                        tr: &s.tr,
                        palette: &palette,
                        view: &s.view,
                        now: s.now.as_ref(),
                        position_ms: 0,
                        settings: &settings,
                        queue_open: false,
                        mini_open: false,
                        artwork: None,
                        collections: &collections,
                        spotify: s.spotify.as_ref(),
                        spotify_devices: None,
                        covers: None,
                    };
                    let input = MiniInput {
                        local: None,
                        on_top: s.on_top,
                        own_window: true,
                    };
                    let actions = s.mini.show(ui, &mut bar, &input);
                    s.out.extend(actions);
                },
                state,
            );
        harness.run_steps(2);
        harness
    }

    fn hover(h: &mut Harness<'_, State>) {
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(150.0, 150.0)));
        h.run_steps(6);
    }

    fn click(h: &mut Harness<'_, State>, key: &str) {
        h.get_by_role_and_label(Role::Button, &en(key)).click();
        h.run_steps(2);
    }

    #[test]
    fn without_the_pointer_only_the_cover_shows() {
        let mut h = harness(local());
        h.run_steps(20);
        assert!(h.query_by_label(&en("player-next")).is_none());
        assert!(h.query_by_label(&en("mini-close")).is_none());
    }

    #[test]
    fn hovering_shows_the_controls() {
        let mut h = harness(local());
        hover(&mut h);
        for key in [
            "player-previous",
            "player-play",
            "player-next",
            "mini-back",
            "mini-on-top",
            "mini-close",
        ] {
            assert!(h.query_by_label(&en(key)).is_some(), "{key}");
        }
        assert!(h.query_by_label(&en("player-shuffle")).is_none());
        assert!(h.query_by_label("Song").is_some());
    }

    #[test]
    fn the_controls_act_like_the_bars() {
        let mut h = harness(local());
        hover(&mut h);
        click(&mut h, "player-next");
        assert_eq!(h.state().out, [MiniAction::Player(PlayerAction::Next)]);

        let mut h = harness(spotify());
        hover(&mut h);
        click(&mut h, "player-next");
        assert_eq!(
            h.state().out,
            [MiniAction::Player(PlayerAction::Spotify(
                SpotifyRequest::Next
            ))]
        );
    }

    #[test]
    fn space_plays_and_pauses() {
        let mut h = harness(local());
        h.key_press(egui::Key::Space);
        h.run_steps(2);
        assert_eq!(
            h.state().out,
            [MiniAction::Player(PlayerAction::TogglePlay)]
        );
    }

    #[test]
    fn the_window_buttons_send_their_actions() {
        let mut h = harness(local());
        hover(&mut h);
        click(&mut h, "mini-back");
        click(&mut h, "mini-on-top");
        click(&mut h, "mini-close");
        assert_eq!(
            h.state().out,
            [
                MiniAction::ShowMain,
                MiniAction::SetOnTop(false),
                MiniAction::Close
            ]
        );
    }

    #[test]
    fn nothing_playing_says_so() {
        let h = harness(state(PlayerView::default(), None));
        assert!(h.query_by_label(&en("empty-nothing-playing")).is_some());
    }

    #[test]
    fn an_os_close_request_closes_it() {
        let mut h = harness(local());
        h.input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        h.step();
        assert!(
            h.state().out.contains(&MiniAction::Close),
            "{:?}",
            h.state().out
        );
    }
}
