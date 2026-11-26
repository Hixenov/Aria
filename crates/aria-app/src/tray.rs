//! The tray icon and its menu: Windows' notification area, macOS's menu bar, and the
//! StatusNotifierItem tray on Linux (D-Bus, through `ksni`; no GTK).
//!
//! [`tooltip`] and [`TrayLabels`] decide what the tray shows, and [`menu_event`] what a click on
//! the menu means. The system behind them is a [`TrayBackend`]: `tray-icon` in the app, a fake in
//! tests. The icon's events arrive on the UI thread's queue and wake the window, so they are
//! handled even while it is hidden.

use std::sync::mpsc::Sender;

use aria_core::i18n::Localizer;
use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::media_controls::NowPlaying;

/// The longest tooltip, in UTF-16 units: Windows keeps 128 with the closing zero.
const TOOLTIP_UNITS: usize = 127;
/// The tray icon's side in pixels; the system scales it to the tray's size.
const ICON_SIDE: u32 = 64;
/// The tooltip when nothing plays, and the icon's id.
const NAME: &str = "Aria";

/// The menu items' ids.
const TOGGLE: &str = "aria-tray-toggle";
const PLAY: &str = "aria-tray-play";
const NEXT: &str = "aria-tray-next";
const PREVIOUS: &str = "aria-tray-previous";
const QUIT: &str = "aria-tray-quit";

/// What a click in the tray asks of Aria.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Show the main window if it is hidden or minimized, hide it otherwise.
    Toggle,
    TogglePlay,
    Next,
    Previous,
    /// Quit for real, whatever "Close to tray" says.
    Quit,
}

/// The menu's words, in Aria's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayLabels {
    pub show: String,
    pub hide: String,
    pub play: String,
    pub pause: String,
    pub next: String,
    pub previous: String,
    pub quit: String,
}

impl TrayLabels {
    pub fn new(tr: &Localizer) -> TrayLabels {
        let text = |key: &str| tr.tr(key).into_owned();
        TrayLabels {
            show: text("tray-show"),
            hide: text("tray-hide"),
            play: text("tray-play"),
            pause: text("tray-pause"),
            next: text("tray-next"),
            previous: text("tray-previous"),
            quit: text("tray-quit"),
        }
    }
}

/// Everything the tray shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    /// The main window is on screen (neither hidden nor minimized).
    pub window_shown: bool,
    pub playing: bool,
    pub tooltip: String,
    pub labels: TrayLabels,
}

impl TrayState {
    fn toggle_label(&self) -> &str {
        if self.window_shown {
            &self.labels.hide
        } else {
            &self.labels.show
        }
    }

    fn play_label(&self) -> &str {
        if self.playing {
            &self.labels.pause
        } else {
            &self.labels.play
        }
    }
}

/// The tray icon's tooltip: "Title – Artist", the title alone, or "Aria" when nothing plays.
pub fn tooltip(now: Option<&NowPlaying>) -> String {
    let Some(now) = now else {
        return NAME.to_owned();
    };
    let text = if now.artist.trim().is_empty() {
        now.title.clone()
    } else {
        format!("{} – {}", now.title, now.artist)
    };
    if text.trim().is_empty() {
        return NAME.to_owned();
    }
    shorten(&text, TOOLTIP_UNITS)
}

/// `text` in at most `units` UTF-16 units, cut between characters and ending in "…" when cut.
fn shorten(text: &str, units: usize) -> String {
    if text.encode_utf16().count() <= units {
        return text.to_owned();
    }
    let ellipsis = '…';
    let mut used = ellipsis.len_utf16();
    let mut out = String::new();
    for c in text.chars() {
        used += c.len_utf16();
        if used > units {
            break;
        }
        out.push(c);
    }
    out.push(ellipsis);
    out
}

/// What a click on the menu item `id` means.
pub fn menu_event(id: &str) -> Option<TrayEvent> {
    match id {
        TOGGLE => Some(TrayEvent::Toggle),
        PLAY => Some(TrayEvent::TogglePlay),
        NEXT => Some(TrayEvent::Next),
        PREVIOUS => Some(TrayEvent::Previous),
        QUIT => Some(TrayEvent::Quit),
        _ => None,
    }
}

/// What a click on the icon itself means: a left click (once released) shows or hides Aria.
/// On macOS a click opens the menu instead, as menu bar icons do there.
fn icon_event(event: &TrayIconEvent) -> Option<TrayEvent> {
    let left_click = matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    );
    (left_click && !cfg!(target_os = "macos")).then_some(TrayEvent::Toggle)
}

/// Aria's icon at [`ICON_SIDE`] pixels square, as RGBA.
fn tray_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let icon = crate::icon::app_icon()?;
    let image = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba)?;
    let small = image::imageops::resize(
        &image,
        ICON_SIDE,
        ICON_SIDE,
        image::imageops::FilterType::Lanczos3,
    );
    Some((small.into_raw(), ICON_SIDE, ICON_SIDE))
}

/// The system's side of the tray.
pub trait TrayBackend {
    /// Shows `state`. Called only when it changed.
    fn update(&mut self, state: &TrayState);
}

/// Makes the tray on the UI thread: clicks go to the sender, and wake the window through the
/// context. An error says why there is no tray, in words for the log.
pub type TrayFactory = Box<
    dyn Fn(Sender<TrayEvent>, egui::Context, &TrayState) -> Result<Box<dyn TrayBackend>, String>,
>;

/// Makes the real tray, with `tray-icon`.
pub fn system_factory() -> TrayFactory {
    Box::new(|events, ctx, state| {
        SystemTray::new(events, ctx, state).map(|tray| Box::new(tray) as Box<dyn TrayBackend>)
    })
}

/// The tray icon and the menu items whose words change.
struct SystemTray {
    icon: TrayIcon,
    toggle: MenuItem,
    play: MenuItem,
    next: MenuItem,
    previous: MenuItem,
    quit: MenuItem,
    shown: TrayState,
}

impl SystemTray {
    fn new(
        events: Sender<TrayEvent>,
        ctx: egui::Context,
        state: &TrayState,
    ) -> Result<SystemTray, String> {
        let toggle = MenuItem::with_id(TOGGLE, state.toggle_label(), true, None);
        let play = MenuItem::with_id(PLAY, state.play_label(), true, None);
        let next = MenuItem::with_id(NEXT, &state.labels.next, true, None);
        let previous = MenuItem::with_id(PREVIOUS, &state.labels.previous, true, None);
        let quit = MenuItem::with_id(QUIT, &state.labels.quit, true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &toggle,
            &PredefinedMenuItem::separator(),
            &play,
            &next,
            &previous,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|error| error.to_string())?;
        let (rgba, width, height) = tray_rgba().ok_or("Aria's icon could not be read")?;
        let image = Icon::from_rgba(rgba, width, height).map_err(|error| error.to_string())?;

        // Both handlers can be set once per process; Aria makes one tray.
        let menu_events = events.clone();
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(event) = menu_event(&event.id.0) {
                let _ = menu_events.send(event);
                menu_ctx.request_repaint();
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let Some(event) = icon_event(&event) {
                let _ = events.send(event);
                ctx.request_repaint();
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_id(NAME)
            .with_menu(Box::new(menu))
            .with_tooltip(&state.tooltip)
            .with_icon(image)
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(SystemTray {
            icon,
            toggle,
            play,
            next,
            previous,
            quit,
            shown: state.clone(),
        })
    }
}

impl TrayBackend for SystemTray {
    fn update(&mut self, state: &TrayState) {
        if *state == self.shown {
            return;
        }
        self.toggle.set_text(state.toggle_label());
        self.play.set_text(state.play_label());
        self.next.set_text(&state.labels.next);
        self.previous.set_text(&state.labels.previous);
        self.quit.set_text(&state.labels.quit);
        if state.tooltip != self.shown.tooltip
            && let Err(error) = self.icon.set_tooltip(Some(&state.tooltip))
        {
            tracing::debug!(%error, "the tray did not take the tooltip");
        }
        self.shown = state.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_core::settings::Language;

    fn now(title: &str, artist: &str) -> NowPlaying {
        NowPlaying {
            identity: "x".to_owned(),
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: None,
            duration_ms: None,
            cover_url: None,
            position_ms: 0,
            playing: true,
        }
    }

    #[test]
    fn tooltip_is_aria_when_nothing_plays() {
        assert_eq!(tooltip(None), "Aria");
    }

    #[test]
    fn tooltip_names_title_and_artist() {
        assert_eq!(tooltip(Some(&now("Song", "Ann"))), "Song – Ann");
    }

    #[test]
    fn tooltip_without_artist_is_the_title() {
        assert_eq!(tooltip(Some(&now("Song", ""))), "Song");
        assert_eq!(tooltip(Some(&now("", ""))), "Aria");
    }

    #[test]
    fn a_long_tooltip_is_cut_at_a_character() {
        // 125 letters, then an emoji (two UTF-16 units) across the limit.
        let title = format!("{}🎵{}", "a".repeat(125), "b".repeat(80));
        let text = tooltip(Some(&now(&title, "")));
        assert!(text.encode_utf16().count() <= 127, "{}", text.len());
        assert!(text.ends_with('…'));
        assert!(text.starts_with(&"a".repeat(125)));
        assert!(!text.contains('🎵'), "half an emoji never fits");
        // Exactly at the limit stays whole.
        let fits = "c".repeat(127);
        assert_eq!(tooltip(Some(&now(&fits, ""))), fits);
    }

    #[test]
    fn menu_ids_map_to_events() {
        assert_eq!(menu_event("aria-tray-toggle"), Some(TrayEvent::Toggle));
        assert_eq!(menu_event("aria-tray-play"), Some(TrayEvent::TogglePlay));
        assert_eq!(menu_event("aria-tray-next"), Some(TrayEvent::Next));
        assert_eq!(menu_event("aria-tray-previous"), Some(TrayEvent::Previous));
        assert_eq!(menu_event("aria-tray-quit"), Some(TrayEvent::Quit));
        assert_eq!(menu_event("something-else"), None);
    }

    #[test]
    fn labels_come_from_the_language() {
        let en = TrayLabels::new(&Localizer::new(Language::En).unwrap());
        assert_eq!(en.show, "Show Aria");
        assert_eq!(en.hide, "Hide Aria");
        assert_eq!(en.pause, "Pause");
        assert_eq!(en.quit, "Quit Aria");
        let ru = TrayLabels::new(&Localizer::new(Language::Ru).unwrap());
        assert_eq!(ru.show, "Показать Aria");
    }

    #[test]
    fn the_tray_image_is_64_pixels() {
        let (rgba, width, height) = tray_rgba().unwrap();
        assert_eq!((width, height), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_left_click_on_the_icon_toggles() {
        use tray_icon::{
            MouseButton, MouseButtonState, Rect, TrayIconEvent, dpi::PhysicalPosition,
        };
        let click = |button, button_state| TrayIconEvent::Click {
            id: "aria".into(),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
            button_state,
        };
        assert_eq!(
            icon_event(&click(MouseButton::Left, MouseButtonState::Up)),
            Some(TrayEvent::Toggle)
        );
        assert_eq!(
            icon_event(&click(MouseButton::Left, MouseButtonState::Down)),
            None
        );
        assert_eq!(
            icon_event(&click(MouseButton::Right, MouseButtonState::Up)),
            None
        );
    }
}
