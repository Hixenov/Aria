//! Mapping the OS locale to a supported [`Language`].

use crate::settings::Language;

/// Maps a locale tag such as `ru-RU` (BCP 47) or `hy_AM.UTF-8` (POSIX) to a
/// supported language by its primary language subtag, ignoring case: `ru` is
/// Russian, `hy` is Armenian, and anything else, or no tag, is English.
pub fn language_from_locale(tag: Option<&str>) -> Language {
    let primary = tag
        .and_then(|tag| tag.split(['-', '_', '.', '@']).next())
        .unwrap_or_default();
    if primary.eq_ignore_ascii_case("ru") {
        Language::Ru
    } else if primary.eq_ignore_ascii_case("hy") {
        Language::Hy
    } else {
        Language::En
    }
}

/// The OS UI language as a supported language; English if the OS reports
/// none.
pub fn os_language() -> Language {
    let tag = sys_locale::get_locale();
    let language = language_from_locale(tag.as_deref());
    tracing::debug!(locale = ?tag, ?language, "resolved OS language");
    language
}

#[cfg(test)]
mod tests {
    use super::*;

    // From the Task 4 brief, verbatim.
    #[test]
    fn locale_mapping() {
        assert_eq!(language_from_locale(Some("ru-RU")), Language::Ru);
        assert_eq!(language_from_locale(Some("hy-AM")), Language::Hy);
        assert_eq!(language_from_locale(Some("en-US")), Language::En);
        assert_eq!(language_from_locale(Some("fr-FR")), Language::En);
        assert_eq!(language_from_locale(Some("")), Language::En);
        assert_eq!(language_from_locale(None), Language::En);
    }

    #[test]
    fn locale_mapping_uses_the_whole_language_subtag() {
        for tag in ["ru", "RU", "ru_RU.UTF-8", "ru-Cyrl-UA"] {
            assert_eq!(language_from_locale(Some(tag)), Language::Ru, "{tag}");
        }
        for tag in ["hy", "HY-am", "hy_AM.UTF-8", "hy-Latn"] {
            assert_eq!(language_from_locale(Some(tag)), Language::Hy, "{tag}");
        }
        // Kirundi and Aromanian only share a prefix with Russian.
        for tag in ["run-BI", "rup", "C", "-ru"] {
            assert_eq!(language_from_locale(Some(tag)), Language::En, "{tag}");
        }
    }
}
