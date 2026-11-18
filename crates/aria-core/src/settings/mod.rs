//! Settings model and persistence.

use std::{fmt, str::FromStr};

use aria_audio::DeviceId;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};

use crate::{hotkey::Hotkey, player::RepeatMode, soundboard::SoundTarget};

pub mod store;

/// Current on-disk schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
/// Minimum sidebar width in logical pixels.
pub const SIDEBAR_MIN: f32 = 180.0;
/// Maximum sidebar width in logical pixels.
pub const SIDEBAR_MAX: f32 = 320.0;
const SIDEBAR_DEFAULT: f32 = 220.0;
const VOLUME_DEFAULT: f32 = 1.0;
const GAPLESS_DEFAULT: bool = true;
const HISTORY_ENABLED_DEFAULT: bool = true;
const EXCLUSIVE_OUTPUT_DEFAULT: bool = false;
const GLOBAL_HOTKEYS_DEFAULT: bool = true;
/// The music's level in the chat, as a share of full.
pub const SHARE_MUSIC_VOLUME_DEFAULT: f32 = 0.5;
/// The microphone's level in the chat; above 1.0 amplifies.
pub const MIC_VOLUME_DEFAULT: f32 = 1.0;
/// The most the microphone can be amplified.
pub const MIC_VOLUME_MAX: f32 = 2.0;

/// UI language.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    /// English.
    #[serde(rename = "en")]
    En,
    /// Russian.
    #[serde(rename = "ru")]
    Ru,
    /// Armenian.
    #[serde(rename = "hy")]
    Hy,
}

impl Language {
    /// All supported languages.
    pub const ALL: [Language; 3] = [Language::En, Language::Ru, Language::Hy];

    /// The language's BCP 47 code.
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Ru => "ru",
            Language::Hy => "hy",
        }
    }
}

/// Color theme.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Dark theme.
    Dark,
    /// Light theme.
    Light,
    /// Follow the OS.
    System,
}

/// Log verbosity.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Warnings and above.
    Warn,
    /// Info and above.
    Info,
    /// Debug and above.
    Debug,
    /// Everything.
    Trace,
}

impl LogLevel {
    /// The `tracing` filter directive for this level.
    pub fn as_filter(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

/// An 8-bit-per-channel color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

/// Error parsing an [`Rgb`] from text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseRgbError;

impl fmt::Display for ParseRgbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected a color like #RRGGBB")
    }
}

impl std::error::Error for ParseRgbError {}

impl FromStr for Rgb {
    type Err = ParseRgbError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseRgbError);
        }
        let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ParseRgbError);
        Ok(Rgb {
            r: ch(0)?,
            g: ch(2)?,
            b: ch(4)?,
        })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Accepts any value for an accent color; anything that is not a valid
/// `#RRGGBB` string becomes `None` with a warning instead of failing the file.
fn lenient_rgb<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Rgb>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        Other(serde::de::IgnoredAny),
    }
    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::S(s)) => match s.parse::<Rgb>() {
            Ok(c) => Ok(Some(c)),
            Err(_) => {
                tracing::warn!(value = %s, "ignoring invalid accent color in settings");
                Ok(None)
            }
        },
        Some(Raw::Other(_)) => {
            tracing::warn!("ignoring non-string accent color in settings");
            Ok(None)
        }
    }
}

/// Reads a setting that must not cost the user the rest of the file: a value of the wrong type
/// becomes `fallback`, with a warning. The value itself is not logged.
fn lenient<'de, D, T>(d: D, fallback: T) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw<T> {
        Valid(T),
        Other(serde::de::IgnoredAny),
    }
    match Raw::<T>::deserialize(d)? {
        Raw::Valid(value) => Ok(value),
        Raw::Other(_) => {
            tracing::warn!(
                expected = std::any::type_name::<T>(),
                "ignoring a setting of the wrong type"
            );
            Ok(fallback)
        }
    }
}

/// [`lenient`], for a setting whose default is its type's.
fn lenient_or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    lenient(d, T::default())
}

/// [`lenient`] for [`Settings::volume`].
fn lenient_volume<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    lenient(d, VOLUME_DEFAULT)
}

/// [`lenient`] for [`Settings::gapless`].
fn lenient_gapless<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient(d, GAPLESS_DEFAULT)
}

/// [`lenient`] for [`Settings::history_enabled`].
fn lenient_history_enabled<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient(d, HISTORY_ENABLED_DEFAULT)
}

/// [`lenient`] for [`Settings::exclusive_output`].
fn lenient_exclusive_output<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient(d, EXCLUSIVE_OUTPUT_DEFAULT)
}

/// [`lenient`] for [`Settings::global_hotkeys`].
fn lenient_global_hotkeys<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    lenient(d, GLOBAL_HOTKEYS_DEFAULT)
}

/// [`lenient`] for [`Settings::share_music_volume`].
fn lenient_share_music_volume<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    lenient(d, SHARE_MUSIC_VOLUME_DEFAULT)
}

/// [`lenient`] for [`Settings::mic_volume`].
fn lenient_mic_volume<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    lenient(d, MIC_VOLUME_DEFAULT)
}

/// Accepts any value for a hotkey; anything that is not a hotkey's text becomes `None` with a
/// warning instead of failing the file. The value itself is not logged.
fn lenient_hotkey<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Hotkey>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        S(String),
        Other(serde::de::IgnoredAny),
    }
    match Option::<Raw>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::S(s)) => match s.parse::<Hotkey>() {
            Ok(hotkey) if hotkey.is_usable() => Ok(Some(hotkey)),
            // A plain typing key or one of Aria's shortcuts, written into the file by hand, is
            // never registered.
            Ok(_) => {
                tracing::warn!("ignoring a hotkey in the settings that is not allowed");
                Ok(None)
            }
            Err(_) => {
                tracing::warn!("ignoring a hotkey in the settings that cannot be read");
                Ok(None)
            }
        },
        Some(Raw::Other(_)) => {
            tracing::warn!("ignoring a hotkey in the settings that is not text");
            Ok(None)
        }
    }
}

/// Persisted user settings.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// On-disk schema version.
    pub schema_version: u32,
    /// UI language; `None` means detect from the OS.
    pub language: Option<Language>,
    /// Color theme.
    pub theme: Theme,
    /// Accent override for the dark theme.
    #[serde(deserialize_with = "lenient_rgb")]
    pub accent_dark: Option<Rgb>,
    /// Accent override for the light theme.
    #[serde(deserialize_with = "lenient_rgb")]
    pub accent_light: Option<Rgb>,
    /// Minimize to the tray instead of the taskbar (enforced in Phase 4).
    pub minimize_to_tray: bool,
    /// Log verbosity.
    pub log_level: LogLevel,
    /// Whether the right panel is open.
    pub right_panel_open: bool,
    /// Sidebar width, clamped to `SIDEBAR_MIN..=SIDEBAR_MAX`.
    pub sidebar_width: f32,
    /// Playback volume, clamped to `0.0..=1.0`.
    #[serde(deserialize_with = "lenient_volume")]
    pub volume: f32,
    /// Whether playback is muted. The volume is kept for unmuting.
    #[serde(deserialize_with = "lenient_or_default")]
    pub muted: bool,
    /// Whether the queue plays shuffled.
    #[serde(deserialize_with = "lenient_or_default")]
    pub shuffle: bool,
    /// What happens when the current track ends.
    #[serde(deserialize_with = "lenient_or_default")]
    pub repeat: RepeatMode,
    /// The output device; `None` means the system default.
    #[serde(deserialize_with = "lenient_or_default")]
    pub output_device: Option<DeviceId>,
    /// Whether tracks join without a gap.
    #[serde(deserialize_with = "lenient_gapless")]
    pub gapless: bool,
    /// Whether plays are recorded. Plays already stored stay when it is switched off.
    #[serde(deserialize_with = "lenient_history_enabled")]
    pub history_enabled: bool,
    /// Whether the device is held alone, at each track's own format. Only Windows has this.
    #[serde(deserialize_with = "lenient_exclusive_output")]
    pub exclusive_output: bool,
    /// The second device for sounds; `None` means there is none.
    #[serde(deserialize_with = "lenient_or_default")]
    pub soundboard_device: Option<DeviceId>,
    /// Where sounds are played.
    #[serde(deserialize_with = "lenient_or_default")]
    pub soundboard_target: SoundTarget,
    /// Whether the hotkeys work while another program has the focus. Off, they stay stored.
    #[serde(deserialize_with = "lenient_global_hotkeys")]
    pub global_hotkeys: bool,
    /// The hotkey of "Stop all", if any.
    #[serde(deserialize_with = "lenient_hotkey")]
    pub stop_sounds_hotkey: Option<Hotkey>,
    /// Whether the music also goes to the second device, for a chat. Without a second device,
    /// nothing is sent.
    #[serde(deserialize_with = "lenient_or_default")]
    pub share_music: bool,
    /// The music's level on the second device, clamped to `0.0..=1.0`. The player's own volume
    /// and mute do not touch it.
    #[serde(deserialize_with = "lenient_share_music_volume")]
    pub share_music_volume: f32,
    /// Whether the microphone goes to the second device. It is open only while this is on and a
    /// second device is set.
    #[serde(deserialize_with = "lenient_or_default")]
    pub mic_enabled: bool,
    /// The microphone; `None` means the system default.
    #[serde(deserialize_with = "lenient_or_default")]
    pub mic_device: Option<DeviceId>,
    /// The microphone's level on the second device, clamped to `0.0..=2.0`.
    #[serde(deserialize_with = "lenient_mic_volume")]
    pub mic_volume: f32,
    /// The hotkey that turns the microphone on and off, if any.
    #[serde(deserialize_with = "lenient_hotkey")]
    pub mic_toggle_hotkey: Option<Hotkey>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            language: None,
            theme: Theme::System,
            accent_dark: None,
            accent_light: None,
            minimize_to_tray: false,
            log_level: LogLevel::Info,
            right_panel_open: false,
            sidebar_width: SIDEBAR_DEFAULT,
            volume: VOLUME_DEFAULT,
            muted: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            output_device: None,
            gapless: GAPLESS_DEFAULT,
            history_enabled: HISTORY_ENABLED_DEFAULT,
            exclusive_output: EXCLUSIVE_OUTPUT_DEFAULT,
            soundboard_device: None,
            soundboard_target: SoundTarget::Mine,
            global_hotkeys: GLOBAL_HOTKEYS_DEFAULT,
            stop_sounds_hotkey: None,
            share_music: false,
            share_music_volume: SHARE_MUSIC_VOLUME_DEFAULT,
            mic_enabled: false,
            mic_device: None,
            mic_volume: MIC_VOLUME_DEFAULT,
            mic_toggle_hotkey: None,
        }
    }
}

impl Settings {
    /// Clamp out-of-range values; a width or volume that is not a number resets to the default.
    #[must_use]
    pub fn sanitized(mut self) -> Settings {
        self.sidebar_width = if self.sidebar_width.is_nan() {
            SIDEBAR_DEFAULT
        } else {
            self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
        };
        self.volume = if self.volume.is_nan() {
            VOLUME_DEFAULT
        } else {
            self.volume.clamp(0.0, 1.0)
        };
        self.share_music_volume = level(self.share_music_volume, 1.0, SHARE_MUSIC_VOLUME_DEFAULT);
        self.mic_volume = level(self.mic_volume, MIC_VOLUME_MAX, MIC_VOLUME_DEFAULT);
        self
    }
}

/// `value` within `0.0..=max`, or `fallback` if it is not a number.
fn level(value: f32, max: f32, fallback: f32) -> f32 {
    if value.is_nan() {
        fallback
    } else {
        value.clamp(0.0, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_load_with_sharing_and_microphone_off() {
        // A 0.2.2 file: none of the 0.2.3 keys.
        let s: Settings = toml::from_str(
            "soundboard_device = \"cable\"\nsoundboard_target = \"second\"\nvolume = 0.4",
        )
        .unwrap();
        assert!(!s.share_music && !s.mic_enabled);
        assert_eq!(s.share_music_volume, 0.5);
        assert_eq!(s.mic_volume, 1.0);
        assert_eq!(s.mic_device, None);
        assert_eq!(s.mic_toggle_hotkey, None);
        assert_eq!(s.soundboard_device, Some(DeviceId("cable".into())));
        assert_eq!(s.volume, 0.4);
        assert_eq!(s.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn chat_settings_of_the_wrong_type_fall_back() {
        let s: Settings = toml::from_str(
            "share_music = 3\nshare_music_volume = \"loud\"\nmic_enabled = \"yes\"\n\
             mic_volume = true\nmic_toggle_hotkey = \"A\"\ntheme = \"dark\"",
        )
        .unwrap();
        assert!(!s.share_music && !s.mic_enabled);
        assert_eq!(s.share_music_volume, 0.5);
        assert_eq!(s.mic_volume, 1.0);
        // A plain typing key is never a hotkey.
        assert_eq!(s.mic_toggle_hotkey, None);
        assert_eq!(s.theme, Theme::Dark);
    }

    #[test]
    fn a_hotkey_in_the_settings_that_is_one_of_arias_shortcuts_is_dropped() {
        let command = if cfg!(target_os = "macos") {
            "Win"
        } else {
            "Ctrl"
        };
        let text = format!(
            "stop_sounds_hotkey = \"{command}+1\"
mic_toggle_hotkey = \"{command}+Numpad9\""
        );
        let s: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s.stop_sounds_hotkey, None);
        assert_eq!(s.mic_toggle_hotkey, None);
    }

    #[test]
    fn volumes_are_clamped() {
        let with = |share: f32, mic: f32| {
            Settings {
                share_music_volume: share,
                mic_volume: mic,
                ..Settings::default()
            }
            .sanitized()
        };
        let s = with(1.5, 3.0);
        assert_eq!((s.share_music_volume, s.mic_volume), (1.0, 2.0));
        let s = with(-1.0, -1.0);
        assert_eq!((s.share_music_volume, s.mic_volume), (0.0, 0.0));
        let s = with(0.3, 1.7);
        assert_eq!((s.share_music_volume, s.mic_volume), (0.3, 1.7));
        let s = with(f32::NAN, f32::NAN);
        assert_eq!((s.share_music_volume, s.mic_volume), (0.5, 1.0));
    }

    #[test]
    fn chat_settings_round_trip() {
        let s = Settings {
            share_music: true,
            share_music_volume: 0.25,
            mic_enabled: true,
            mic_device: Some(DeviceId("mic".into())),
            mic_volume: 1.5,
            mic_toggle_hotkey: Some("Ctrl+Alt+M".parse().unwrap()),
            ..Settings::default()
        };
        let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn defaults_roundtrip() {
        let s = Settings::default();
        let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
        assert_eq!(s.schema_version, 1);
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.sidebar_width, 220.0);
        assert!(s.language.is_none() && !s.minimize_to_tray && !s.right_panel_open);
    }
    #[test]
    fn partial_file_fills_defaults() {
        let s: Settings = toml::from_str("theme = \"dark\"").unwrap();
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.log_level, LogLevel::Info);
    }
    #[test]
    fn rgb_parse_variants() {
        for ok in ["#2EC4B6", "#2ec4b6", "2ec4b6"] {
            assert_eq!(
                ok.parse::<Rgb>().unwrap(),
                Rgb {
                    r: 0x2e,
                    g: 0xc4,
                    b: 0xb6
                }
            );
        }
        for bad in ["#zzz", "", "#2ec4b", "#2ec4b6ff"] {
            assert!(bad.parse::<Rgb>().is_err());
        }
        assert_eq!(
            Rgb {
                r: 0x2e,
                g: 0xc4,
                b: 0xb6
            }
            .to_string(),
            "#2EC4B6"
        );
    }
    #[test]
    fn invalid_accent_falls_back_to_none() {
        let s: Settings = toml::from_str("accent_dark = \"#zzz\"\ntheme = \"light\"").unwrap();
        assert_eq!(s.accent_dark, None);
        assert_eq!(s.theme, Theme::Light);
    }
    #[test]
    fn non_string_accent_falls_back_to_none() {
        let s: Settings = toml::from_str("accent_dark = 5\ntheme = \"light\"").unwrap();
        assert_eq!(s.accent_dark, None);
        assert_eq!(s.theme, Theme::Light);
    }
    #[test]
    fn unknown_keys_are_ignored() {
        let s: Settings = toml::from_str("future_option = 1\ntheme = \"dark\"").unwrap();
        assert_eq!(s.theme, Theme::Dark);
    }
    #[test]
    fn sidebar_width_clamps() {
        let with = |w: f32| {
            Settings {
                sidebar_width: w,
                ..Settings::default()
            }
            .sanitized()
        };
        assert_eq!(with(9000.0).sidebar_width, 320.0);
        assert_eq!(with(10.0).sidebar_width, 180.0);
        assert_eq!(with(f32::NAN).sidebar_width, 220.0);
    }
    #[test]
    fn new_fields_default() {
        let s = Settings::default();
        assert_eq!(s.volume, 1.0);
        assert!(!s.muted && !s.shuffle);
        assert_eq!(s.repeat, RepeatMode::Off);
        assert_eq!(s.output_device, None);
        assert!(s.gapless);
    }
    #[test]
    fn old_settings_file_loads() {
        // A complete file as Phase 1 wrote it.
        let phase1 = "schema_version = 1\n\
                      language = \"ru\"\n\
                      theme = \"dark\"\n\
                      minimize_to_tray = false\n\
                      log_level = \"info\"\n\
                      right_panel_open = true\n\
                      sidebar_width = 240.0\n";
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        std::fs::write(&path, phase1).unwrap();
        let loaded = store::SettingsStore::new(path).load();
        assert_eq!(loaded.notice, None);
        assert!(loaded.writable);
        let s = loaded.settings;
        assert_eq!(s.language, Some(Language::Ru));
        assert_eq!(s.sidebar_width, 240.0);
        assert_eq!(s.volume, 1.0);
        assert!(s.gapless);
        assert_eq!(s.repeat, RepeatMode::Off);
        assert_eq!(s.output_device, None);
    }
    #[test]
    fn history_enabled_defaults_to_true_and_survives_a_bad_value() {
        assert!(Settings::default().history_enabled);
        let s: Settings = toml::from_str("history_enabled = \"x\"\ntheme = \"dark\"").unwrap();
        assert!(s.history_enabled);
        assert_eq!(s.theme, Theme::Dark, "the rest of the file still counts");
        let s: Settings = toml::from_str("history_enabled = false").unwrap();
        assert!(!s.history_enabled);
    }
    #[test]
    fn an_old_settings_file_loads_with_exclusive_off() {
        let s: Settings = toml::from_str(
            "theme = \"dark\"
volume = 0.5",
        )
        .unwrap();
        assert!(!s.exclusive_output);
        assert!(!Settings::default().exclusive_output);
        let s: Settings = toml::from_str("exclusive_output = true").unwrap();
        assert!(s.exclusive_output);
    }
    #[test]
    fn a_bad_exclusive_value_falls_back_to_off() {
        let s: Settings = toml::from_str(
            "exclusive_output = \"yes\"
theme = \"dark\"",
        )
        .unwrap();
        assert!(!s.exclusive_output);
        assert_eq!(s.theme, Theme::Dark, "the rest of the file still counts");
    }
    #[test]
    fn an_old_settings_file_loads_with_the_soundboard_defaults() {
        let s: Settings = toml::from_str("theme = \"dark\"\nexclusive_output = true").unwrap();
        assert_eq!(s.soundboard_device, None);
        assert_eq!(s.soundboard_target, SoundTarget::Mine);
        assert!(s.global_hotkeys);
        assert_eq!(s.stop_sounds_hotkey, None);
        let d = Settings::default();
        assert_eq!(
            (d.soundboard_device, d.soundboard_target, d.global_hotkeys),
            (None, SoundTarget::Mine, true)
        );
        assert_eq!(d.stop_sounds_hotkey, None);
    }
    #[test]
    fn the_soundboard_settings_roundtrip() {
        let s = Settings {
            soundboard_device: Some(DeviceId("name:Cable".into())),
            soundboard_target: SoundTarget::Both,
            global_hotkeys: false,
            stop_sounds_hotkey: Some("Ctrl+Alt+F12".parse().unwrap()),
            ..Settings::default()
        };
        let text = toml::to_string(&s).unwrap();
        assert!(
            text.contains("stop_sounds_hotkey = \"Ctrl+Alt+F12\""),
            "{text}"
        );
        assert!(text.contains("soundboard_target = \"both\""), "{text}");
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(s, back);
    }
    #[test]
    fn bad_soundboard_values_fall_back_without_costing_the_file() {
        let s: Settings = toml::from_str(
            "soundboard_device = 5
soundboard_target = \"loud\"
global_hotkeys = \"yes\"
stop_sounds_hotkey = \"Hyper+Q\"
theme = \"dark\"",
        )
        .unwrap();
        assert_eq!(s.soundboard_device, None);
        assert_eq!(s.soundboard_target, SoundTarget::Mine);
        assert!(s.global_hotkeys);
        assert_eq!(s.stop_sounds_hotkey, None);
        assert_eq!(s.theme, Theme::Dark, "the rest of the file still counts");
        let s: Settings = toml::from_str("stop_sounds_hotkey = 3").unwrap();
        assert_eq!(s.stop_sounds_hotkey, None);
    }
    #[test]
    fn a_stop_hotkey_that_is_not_allowed_loads_as_none() {
        for text in ["A", "Shift+A", "5"] {
            let s: Settings = toml::from_str(&format!(
                "stop_sounds_hotkey = \"{text}\"\ntheme = \"dark\""
            ))
            .unwrap();
            assert_eq!(s.stop_sounds_hotkey, None, "{text}");
            assert_eq!(s.theme, Theme::Dark);
        }
        let s: Settings = toml::from_str("stop_sounds_hotkey = \"F9\"").unwrap();
        assert_eq!(s.stop_sounds_hotkey, Some("F9".parse().unwrap()));
    }
    #[test]
    fn a_0_2_0_settings_file_loads() {
        // A complete file as 0.2.0 wrote it, before the history setting.
        let v020 = "schema_version = 1\n\
                    language = \"hy\"\n\
                    theme = \"light\"\n\
                    minimize_to_tray = false\n\
                    log_level = \"info\"\n\
                    right_panel_open = false\n\
                    sidebar_width = 220.0\n\
                    volume = 0.5\n\
                    muted = false\n\
                    shuffle = true\n\
                    repeat = \"all\"\n\
                    gapless = false\n";
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.toml");
        std::fs::write(&path, v020).unwrap();
        let loaded = store::SettingsStore::new(path).load();
        assert_eq!(loaded.notice, None);
        assert!(loaded.writable);
        let s = loaded.settings;
        assert_eq!(s.language, Some(Language::Hy));
        assert_eq!(s.volume, 0.5);
        assert!(s.shuffle && !s.gapless);
        assert_eq!(s.repeat, RepeatMode::All);
        assert!(s.history_enabled);
    }
    #[test]
    fn new_fields_roundtrip() {
        let s = Settings {
            volume: 0.25,
            muted: true,
            shuffle: true,
            repeat: RepeatMode::All,
            output_device: Some(DeviceId("name:Speakers".into())),
            gapless: false,
            ..Settings::default()
        };
        let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
    #[test]
    fn volume_is_sanitized() {
        let with = |volume: f32| {
            Settings {
                volume,
                ..Settings::default()
            }
            .sanitized()
            .volume
        };
        assert_eq!(with(5.0), 1.0);
        assert_eq!(with(-1.0), 0.0);
        assert_eq!(with(f32::NAN), 1.0);
        assert_eq!(with(0.5), 0.5);
    }
}
