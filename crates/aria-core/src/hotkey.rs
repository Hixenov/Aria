//! Hotkeys for the soundboard: their text form, and the rule that keeps typing keys free.
//!
//! A hotkey is written as its held modifiers, then its key, joined by `+`: `Ctrl+Alt+F1`,
//! `Ctrl+Shift+A`, `Numpad3`. The modifiers are always written in the order Ctrl, Alt, Shift,
//! Win. Text is read in any case and with the modifiers in any order, so what was written reads
//! back as the same hotkey.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The highest function key a hotkey can use.
pub const F_KEY_MAX: u8 = 24;

/// A key combination that fires a sound, or stops them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// Ctrl is held.
    pub ctrl: bool,
    /// Alt is held.
    pub alt: bool,
    /// Shift is held.
    pub shift: bool,
    /// The Windows key (Super, Command) is held.
    pub win: bool,
    /// The key pressed with them.
    pub key: HotkeyKey,
}

/// The key of a [`Hotkey`], apart from its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyKey {
    /// A letter, `A` to `Z`, held as the upper-case letter.
    Letter(char),
    /// A digit of the main keyboard, 0 to 9.
    Digit(u8),
    /// A function key, F1 to F24.
    F(u8),
    /// A digit of the numpad, 0 to 9.
    Numpad(u8),
    /// The numpad's `+`.
    NumpadAdd,
    /// The numpad's `-`.
    NumpadSubtract,
    /// The numpad's `*`.
    NumpadMultiply,
    /// The numpad's `/`.
    NumpadDivide,
    /// The numpad's decimal point.
    NumpadDecimal,
}

/// The numpad keys other than its digits, with their names.
const NUMPAD_NAMED: [(HotkeyKey, &str); 5] = [
    (HotkeyKey::NumpadAdd, "NumpadAdd"),
    (HotkeyKey::NumpadSubtract, "NumpadSubtract"),
    (HotkeyKey::NumpadMultiply, "NumpadMultiply"),
    (HotkeyKey::NumpadDivide, "NumpadDivide"),
    (HotkeyKey::NumpadDecimal, "NumpadDecimal"),
];

impl HotkeyKey {
    /// Whether this is a key a hotkey can use at all: a letter `A` to `Z`, a digit 0 to 9, F1 to
    /// F24, or a numpad key.
    fn is_valid(self) -> bool {
        match self {
            Self::Letter(c) => c.is_ascii_uppercase(),
            Self::Digit(d) | Self::Numpad(d) => d <= 9,
            Self::F(n) => (1..=F_KEY_MAX).contains(&n),
            Self::NumpadAdd
            | Self::NumpadSubtract
            | Self::NumpadMultiply
            | Self::NumpadDivide
            | Self::NumpadDecimal => true,
        }
    }

    /// Whether the key is free of typing, so that it may be a hotkey on its own: F1 to F24 and
    /// the numpad.
    fn stands_alone(self) -> bool {
        matches!(
            self,
            Self::F(_)
                | Self::Numpad(_)
                | Self::NumpadAdd
                | Self::NumpadSubtract
                | Self::NumpadMultiply
                | Self::NumpadDivide
                | Self::NumpadDecimal
        )
    }
}

impl Hotkey {
    /// Whether this hotkey may be used: its key is one a hotkey can use, and Ctrl, Alt or Win is
    /// held, or the key is F1 to F24 or a numpad key. Shift alone is not enough, so no key that
    /// types text is taken from typing.
    pub fn is_allowed(&self) -> bool {
        self.key.is_valid() && (self.ctrl || self.alt || self.win || self.key.stands_alone())
    }

    /// Whether this is one of Aria's own shortcuts: Ctrl+0 to Ctrl+9 (the pages) or Ctrl+Alt+R
    /// (the right panel), with Command in place of Ctrl on a Mac. Registered with the system, a
    /// hotkey would take it from Aria, so it counts as used inside Aria. Aria's other shortcuts,
    /// Space and Alt+Left/Right, are on keys no hotkey can have.
    pub fn is_aria_shortcut(&self) -> bool {
        // Aria reads its shortcuts with exactly these modifiers held.
        let command = if cfg!(target_os = "macos") {
            self.win && !self.ctrl
        } else {
            self.ctrl && !self.win
        };
        command
            && !self.shift
            && match self.key {
                HotkeyKey::Digit(_) => !self.alt,
                HotkeyKey::Letter('R') => self.alt,
                _ => false,
            }
    }
}

/// Text that is not a hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a hotkey")]
pub struct ParseHotkeyError;

impl FromStr for HotkeyKey {
    type Err = ParseHotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return match c {
                'a'..='z' | 'A'..='Z' => Ok(Self::Letter(c.to_ascii_uppercase())),
                '0'..='9' => Ok(Self::Digit(digit(c))),
                _ => Err(ParseHotkeyError),
            };
        }
        if let Some((key, _)) = NUMPAD_NAMED
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(s))
        {
            return Ok(*key);
        }
        let lower = s.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("numpad") {
            let mut chars = rest.chars();
            return match (chars.next(), chars.next()) {
                (Some(c @ '0'..='9'), None) => Ok(Self::Numpad(digit(c))),
                _ => Err(ParseHotkeyError),
            };
        }
        if let Some(rest) = lower.strip_prefix('f') {
            // Digits only, without a leading zero, so that each key has one spelling.
            let digits = !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
            if digits && !rest.starts_with('0') {
                return match rest.parse::<u8>() {
                    Ok(n) if n <= F_KEY_MAX => Ok(Self::F(n)),
                    _ => Err(ParseHotkeyError),
                };
            }
        }
        Err(ParseHotkeyError)
    }
}

/// The value of an ASCII digit.
fn digit(c: char) -> u8 {
    // Only called with '0'..='9', whose values all fit.
    c.to_digit(10)
        .and_then(|d| u8::try_from(d).ok())
        .unwrap_or(0)
}

impl fmt::Display for HotkeyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Self::Digit(d) => write!(f, "{d}"),
            Self::F(n) => write!(f, "F{n}"),
            Self::Numpad(d) => write!(f, "Numpad{d}"),
            named => {
                let name = NUMPAD_NAMED
                    .iter()
                    .find(|(key, _)| key == named)
                    .map_or("", |(_, name)| name);
                f.write_str(name)
            }
        }
    }
}

impl FromStr for Hotkey {
    type Err = ParseHotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, modifiers) = parts.split_last().ok_or(ParseHotkeyError)?;
        let mut hotkey = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: key.parse()?,
        };
        for modifier in modifiers {
            let held = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" => &mut hotkey.ctrl,
                "alt" => &mut hotkey.alt,
                "shift" => &mut hotkey.shift,
                "win" => &mut hotkey.win,
                _ => return Err(ParseHotkeyError),
            };
            // A modifier named twice is a typing slip, not a hotkey.
            if std::mem::replace(held, true) {
                return Err(ParseHotkeyError);
            }
        }
        Ok(hotkey)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let modifiers = [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.win, "Win+"),
        ];
        for (held, text) in modifiers {
            if held {
                f.write_str(text)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl Serialize for Hotkey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Hotkey {
        text.parse()
            .unwrap_or_else(|_| panic!("{text:?} should be a hotkey"))
    }

    /// `rest` with Aria's command key: Ctrl, or Command (Win) on a Mac.
    fn command(rest: &str) -> Hotkey {
        let modifier = if cfg!(target_os = "macos") {
            "Win"
        } else {
            "Ctrl"
        };
        key(&format!("{modifier}+{rest}"))
    }

    #[test]
    fn arias_own_shortcuts_are_known() {
        for digit in 0..=9 {
            assert!(command(&digit.to_string()).is_aria_shortcut(), "{digit}");
        }
        assert!(command("Alt+R").is_aria_shortcut());
        // Other modifiers, or the numpad's digits, are not Aria's.
        for other in [
            "Shift+1",
            "Alt+1",
            "R",
            "Shift+Alt+R",
            "Numpad1",
            "F1",
            "Alt+5",
        ] {
            assert!(!command(other).is_aria_shortcut(), "{other}");
        }
        for other in ["Alt+1", "Alt+R", "F5", "Numpad1"] {
            assert!(!key(other).is_aria_shortcut(), "{other}");
        }
    }

    #[test]
    fn arias_shortcuts_on_other_keys_cannot_be_hotkeys() {
        // Space and Alt+Left/Right are Aria's too; no hotkey has those keys.
        for text in ["Space", "Alt+Left", "Alt+Right", "Left"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text}");
        }
    }

    #[test]
    fn text_round_trips() {
        let table = [
            "Ctrl+Alt+F1",
            "Ctrl+Shift+A",
            "Numpad3",
            "F24",
            "Alt+5",
            "Shift+Win+S",
            "Ctrl+Alt+Shift+Win+Z",
            "Ctrl+NumpadAdd",
            "NumpadSubtract",
            "Shift+NumpadMultiply",
            "Alt+NumpadDivide",
            "Ctrl+NumpadDecimal",
        ];
        for text in table {
            let hotkey = key(text);
            assert_eq!(hotkey.to_string(), text);
            assert_eq!(key(&hotkey.to_string()), hotkey, "{text}");
        }
        assert_eq!(
            key("Ctrl+Alt+F1"),
            Hotkey {
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
                key: HotkeyKey::F(1),
            }
        );
        assert_eq!(key("Numpad3").key, HotkeyKey::Numpad(3));
        assert_eq!(key("Ctrl+Shift+A").key, HotkeyKey::Letter('A'));
    }

    #[test]
    fn text_is_read_in_any_case_and_order_and_written_in_one() {
        assert_eq!(key("shift+ctrl+a").to_string(), "Ctrl+Shift+A");
        assert_eq!(key("win+alt+numpad0").to_string(), "Alt+Win+Numpad0");
        assert_eq!(key(" Ctrl + F5 ").to_string(), "Ctrl+F5");
        assert_eq!(key("Win+Shift+S").to_string(), "Shift+Win+S");
        // A lone "F" is the letter.
        assert_eq!(key("Ctrl+F").key, HotkeyKey::Letter('F'));
    }

    #[test]
    fn a_plain_typing_key_is_refused() {
        for text in ["A", "Shift+A", "5", "Shift+5"] {
            assert!(!key(text).is_allowed(), "{text} must not be allowed");
        }
        for text in ["F5", "Numpad0", "Ctrl+A", "Alt+5", "Win+Shift+S"] {
            assert!(key(text).is_allowed(), "{text} must be allowed");
        }
        for text in ["NumpadAdd", "Shift+NumpadDecimal", "F24", "Shift+F1"] {
            assert!(key(text).is_allowed(), "{text} must be allowed");
        }
    }

    #[test]
    fn a_key_out_of_range_is_never_allowed() {
        let with = |key| Hotkey {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            key,
        };
        for bad in [
            HotkeyKey::Letter('a'),
            HotkeyKey::Letter('!'),
            HotkeyKey::Digit(10),
            HotkeyKey::F(0),
            HotkeyKey::F(25),
            HotkeyKey::Numpad(10),
        ] {
            assert!(!with(bad).is_allowed(), "{bad:?}");
        }
    }

    #[test]
    fn unknown_text_is_an_error() {
        for text in [
            "",
            "+",
            "Ctrl",
            "Ctrl+",
            "Ctrl+Alt",
            "Hyper+A",
            "Ctrl+Ctrl+A",
            "Ctrl+AB",
            "Ctrl+Space",
            "Ctrl+A+B",
            "Ctrl+Numpad",
            "Ctrl+Numpad10",
            "Ctrl+NumpadPlus",
            "Ctrl+F01",
            "Ctrl+F1x",
            "Ctrl+Ä",
            "Ctrl+-",
        ] {
            assert!(text.parse::<Hotkey>().is_err(), "{text:?} parsed");
        }
    }

    #[test]
    fn f_keys_stop_at_twenty_four() {
        assert_eq!(key("F1").key, HotkeyKey::F(1));
        assert_eq!(key("F24").key, HotkeyKey::F(24));
        for text in ["F0", "F25", "F99", "F100", "F255", "F256"] {
            assert!(text.parse::<Hotkey>().is_err(), "{text} parsed");
        }
    }

    #[test]
    fn serde_uses_the_text() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Holder {
            hotkey: Hotkey,
        }
        let holder = Holder {
            hotkey: key("Ctrl+Alt+F1"),
        };
        let text = toml::to_string(&holder).unwrap();
        assert_eq!(text.trim(), "hotkey = \"Ctrl+Alt+F1\"");
        assert_eq!(toml::from_str::<Holder>(&text).unwrap(), holder);
        assert!(toml::from_str::<Holder>("hotkey = \"Ctrl\"").is_err());
    }
}
