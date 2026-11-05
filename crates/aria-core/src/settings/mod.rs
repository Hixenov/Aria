//! Settings model and persistence.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod store;

/// Current on-disk schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
/// Minimum sidebar width in logical pixels.
pub const SIDEBAR_MIN: f32 = 180.0;
/// Maximum sidebar width in logical pixels.
pub const SIDEBAR_MAX: f32 = 320.0;
const SIDEBAR_DEFAULT: f32 = 220.0;

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
        }
    }
}

impl Settings {
    /// Clamp out-of-range values; NaN widths reset to the default.
    #[must_use]
    pub fn sanitized(mut self) -> Settings {
        self.sidebar_width = if self.sidebar_width.is_nan() {
            SIDEBAR_DEFAULT
        } else {
            self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
