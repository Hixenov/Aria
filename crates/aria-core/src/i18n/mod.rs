//! Localization with Fluent.
//!
//! Each supported language has one `locales/<code>/main.ftl`, embedded at
//! build time. [`Localizer`] looks a key up in the active language, then in
//! English, and finally falls back to the key itself, so a gap shows up as
//! its key rather than as blank space.

use std::{
    borrow::Cow,
    collections::{BTreeSet, HashSet},
    fmt,
    sync::Mutex,
};

use fluent_bundle::{FluentError, FluentResource, concurrent::FluentBundle};
use fluent_syntax::{ast, parser::ParserError};
use unic_langid::LanguageIdentifier;

use crate::{error::I18nError, settings::Language};

pub mod locale;

pub use fluent_bundle::FluentArgs;
pub use locale::{language_from_locale, os_language};

const EN: &str = include_str!("../../../../locales/en/main.ftl");
const RU: &str = include_str!("../../../../locales/ru/main.ftl");
const HY: &str = include_str!("../../../../locales/hy/main.ftl");

/// The embedded Fluent source of every language, in [`Language::ALL`] order,
/// so `SOURCES[0]` is English.
pub const SOURCES: [(Language, &str); 3] =
    [(Language::En, EN), (Language::Ru, RU), (Language::Hy, HY)];

/// A thread-safe bundle, so [`Localizer`] is `Send + Sync`.
type Bundle = FluentBundle<FluentResource>;

/// Translates message keys into UI text.
///
/// Lookups try the active language, then English, then return the key
/// itself. A key that is missing everywhere, or whose message formats with
/// errors (such as a missing argument), is logged at `warn` once.
pub struct Localizer {
    language: Language,
    active: Bundle,
    english: Bundle,
    /// Keys already warned about, so per-frame lookups don't flood the log.
    warned: Mutex<HashSet<String>>,
}

impl Localizer {
    /// Builds the bundles for `lang` and for English.
    pub fn new(lang: Language) -> Result<Self, I18nError> {
        Self::from_sources(lang, source(lang), EN)
    }

    fn from_sources(lang: Language, active: &str, english: &str) -> Result<Self, I18nError> {
        Ok(Self {
            language: lang,
            active: build_bundle(lang, active)?,
            english: build_bundle(Language::En, english)?,
            warned: Mutex::new(HashSet::new()),
        })
    }

    /// Switches the active language; the next lookup uses it. On error the
    /// current language stays active.
    pub fn set_language(&mut self, lang: Language) -> Result<(), I18nError> {
        if lang != self.language {
            self.active = build_bundle(lang, source(lang))?;
            self.language = lang;
        }
        Ok(())
    }

    /// The active language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The text for `key`.
    pub fn tr(&self, key: &str) -> Cow<'_, str> {
        self.format(key, None)
    }

    /// The text for `key`, with `args` filling its placeables.
    pub fn tr_args(&self, key: &str, args: &FluentArgs) -> Cow<'_, str> {
        self.format(key, Some(args))
    }

    fn format(&self, key: &str, args: Option<&FluentArgs>) -> Cow<'_, str> {
        for bundle in [&self.active, &self.english] {
            // A message with only attributes has no text: treat it as missing.
            let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, args, &mut errors);
            if !errors.is_empty() && self.first_warning(key) {
                tracing::warn!(
                    key,
                    errors = %join(&errors),
                    "localized message has formatting errors"
                );
            }
            return text;
        }
        if self.first_warning(key) {
            tracing::warn!(
                key,
                "no translation in the active language or English; showing the key"
            );
        }
        Cow::Owned(key.to_owned())
    }

    /// Whether this is the first warning for `key`. A poisoned lock skips the
    /// dedup: repeating a warning beats losing it.
    fn first_warning(&self, key: &str) -> bool {
        let Ok(mut warned) = self.warned.lock() else {
            return true;
        };
        if warned.contains(key) {
            false
        } else {
            warned.insert(key.to_owned())
        }
    }
}

impl fmt::Debug for Localizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Localizer")
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

/// The id of every message in a Fluent source. Terms, comments and entries
/// that fail to parse are skipped.
pub fn message_ids(src: &str) -> BTreeSet<String> {
    let resource = FluentResource::try_new(src.to_owned()).unwrap_or_else(|(partial, _)| partial);
    resource
        .entries()
        .filter_map(|entry| match entry {
            ast::Entry::Message(message) => Some(message.id.name.to_owned()),
            _ => None,
        })
        .collect()
}

fn source(lang: Language) -> &'static str {
    match lang {
        Language::En => EN,
        Language::Ru => RU,
        Language::Hy => HY,
    }
}

/// Parses `src` into a bundle for `lang`. Bidi isolation is off because egui
/// draws the U+2068/U+2069 marks Fluent puts around placeables as boxes.
fn build_bundle(lang: Language, src: &str) -> Result<Bundle, I18nError> {
    let parse_error = |detail: String| I18nError::Parse {
        lang: lang.code(),
        detail,
    };
    let id: LanguageIdentifier = lang
        .code()
        .parse()
        .map_err(|e| parse_error(format!("invalid language identifier: {e}")))?;
    let resource = FluentResource::try_new(src.to_owned())
        .map_err(|(_, errors)| parse_error(describe_syntax_errors(src, &errors)))?;
    let mut bundle = Bundle::new_concurrent(vec![id]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .map_err(|errors| parse_error(join(&errors)))?;
    Ok(bundle)
}

/// Joins syntax errors, each prefixed with its 1-based line in `src`.
fn describe_syntax_errors(src: &str, errors: &[ParserError]) -> String {
    let described: Vec<String> = errors
        .iter()
        .map(|e| match src.get(..e.pos.start) {
            Some(before) => format!("line {}: {e}", before.matches('\n').count() + 1),
            None => e.to_string(),
        })
        .collect();
    described.join("; ")
}

fn join(errors: &[FluentError]) -> String {
    let described: Vec<String> = errors.iter().map(ToString::to_string).collect();
    described.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    /// Every message id English must define: the Task 4 key list, plus the log level
    /// names Task 10 added for the Settings page.
    const KEYS: &[&str] = &[
        "nav-home",
        "nav-search",
        "nav-library",
        "nav-liked",
        "nav-playlists",
        "nav-albums",
        "nav-artists",
        "nav-local",
        "nav-settings",
        "nav-back",
        "nav-forward",
        "empty-phase-pending",
        "empty-nothing-playing",
        "empty-right-panel",
        "player-play",
        "player-previous",
        "player-next",
        "player-volume",
        "player-devices",
        "player-queue",
        "player-lyrics",
        "player-coming-in",
        "settings-general",
        "settings-language",
        "settings-language-system",
        "settings-theme",
        "settings-theme-dark",
        "settings-theme-light",
        "settings-theme-system",
        "settings-accent-dark",
        "settings-accent-light",
        "settings-accent-reset",
        "settings-minimize-to-tray",
        "settings-minimize-to-tray-hint",
        "settings-advanced",
        "settings-log-level",
        "settings-log-level-error",
        "settings-log-level-warn",
        "settings-log-level-info",
        "settings-log-level-debug",
        "settings-log-level-trace",
        "settings-open-config",
        "settings-open-logs",
        "settings-reset",
        "settings-reset-confirm-title",
        "settings-reset-confirm-body",
        "settings-cancel",
        "settings-confirm",
        "settings-about",
        "settings-version",
        "settings-license",
        "settings-third-party",
        "notice-settings-recovered",
        "notice-settings-readonly",
        "notice-settings-save-failed",
        "notice-open-folder-failed",
        "notice-dismiss",
        "library-scan-skipped",
        "library-folder-unavailable",
        "library-folder-nested",
        "library-folder-duplicate",
        "library-recovered",
        "library-newer",
        "library-unavailable",
        "library-action-failed",
        "playback-track-failed",
        "playback-stopped-after-failures",
        "playback-device-fallback",
        "playback-no-device",
        "error-startup-title",
        "error-startup-window",
        "lang-en",
        "lang-ru",
        "lang-hy",
    ];

    // The six tests below are the Task 4 brief's, verbatim except for the
    // `.cloned()` in `key_parity_with_english`: without it the `&String`s
    // borrow the temporary set from `message_ids(src)` (E0716).

    #[test]
    fn all_sources_parse() {
        for l in Language::ALL {
            Localizer::new(l).unwrap();
        }
    }

    #[test]
    fn key_parity_with_english() {
        let en = message_ids(SOURCES[0].1);
        for (lang, src) in &SOURCES[1..] {
            let missing: Vec<_> = en.difference(&message_ids(src)).cloned().collect();
            assert!(missing.is_empty(), "{lang:?} missing {missing:?}");
        }
    }

    #[test]
    fn falls_back_to_english_then_key() {
        let l = Localizer::new(Language::Ru).unwrap();
        assert_eq!(l.tr("definitely-not-a-key"), "definitely-not-a-key");
    }

    #[test]
    fn switches_language_live() {
        let mut l = Localizer::new(Language::En).unwrap();
        assert_eq!(l.tr("nav-settings"), "Settings");
        l.set_language(Language::Ru).unwrap();
        assert_ne!(l.tr("nav-settings"), "Settings");
    }

    #[test]
    fn tr_args_has_no_isolation_marks() {
        let l = Localizer::new(Language::En).unwrap();
        let mut a = FluentArgs::new();
        a.set("phase", 2);
        let s = l.tr_args("empty-phase-pending", &a);
        assert_eq!(s, "Available in Phase 2");
        assert!(!s.contains('\u{2068}') && !s.contains('\u{2069}'));
    }

    // Sources and key sets.

    #[test]
    fn sources_follow_language_all_order() {
        assert_eq!(SOURCES.map(|(lang, _)| lang), Language::ALL);
    }

    #[test]
    fn english_defines_exactly_the_planned_keys() {
        let expected: BTreeSet<String> = KEYS.iter().map(|k| k.to_string()).collect();
        assert_eq!(message_ids(SOURCES[0].1), expected);
    }

    #[test]
    fn no_locale_defines_keys_english_lacks() {
        let en = message_ids(SOURCES[0].1);
        for (lang, src) in &SOURCES[1..] {
            let ids = message_ids(src);
            let extra: Vec<_> = ids.difference(&en).collect();
            assert!(extra.is_empty(), "{lang:?} has extra {extra:?}");
        }
    }

    #[test]
    fn message_ids_lists_only_messages() {
        let src = concat!(
            "### Resource comment\n",
            "## Group comment\n",
            "# Message comment\n",
            "hello = Hi\n",
            "-brand = Aria\n",
            "attrs-only =\n",
            "    .title = Title\n",
        );
        let ids: Vec<String> = message_ids(src).into_iter().collect();
        assert_eq!(ids, ["attrs-only", "hello"]);
    }

    #[test]
    fn endonyms_are_identical_in_every_locale() {
        for lang in Language::ALL {
            let l = Localizer::new(lang).unwrap();
            assert_eq!(l.tr("lang-en"), "English", "{lang:?}");
            assert_eq!(l.tr("lang-ru"), "Русский", "{lang:?}");
            assert_eq!(l.tr("lang-hy"), "Հայերեն", "{lang:?}");
        }
    }

    #[test]
    fn translations_keep_english_placeables() {
        let mut args = FluentArgs::new();
        args.set("phase", 777);
        args.set("count", 777);
        args.set("folder", "777");
        args.set("title", "777");
        let en = Localizer::new(Language::En).unwrap();
        // Guard against a vacuous pass: English must use the placeable.
        assert!(en.tr_args("player-coming-in", &args).contains("777"));
        for lang in Language::ALL {
            let l = Localizer::new(lang).unwrap();
            for key in KEYS {
                let text = l.tr_args(key, &args);
                let wants_phase = en.tr_args(key, &args).contains("777");
                assert_eq!(text.contains("777"), wants_phase, "{lang:?} {key}: {text}");
                // An unresolved reference is rendered in braces, e.g. `{$count}`.
                assert!(!text.contains('{'), "{lang:?} {key}: {text}");
            }
        }
    }

    // Lookup and fallback.

    #[test]
    fn plain_messages_borrow_from_the_bundle() {
        let l = Localizer::new(Language::En).unwrap();
        assert!(matches!(l.tr("nav-home"), Cow::Borrowed("Home")));
    }

    #[test]
    fn key_missing_in_active_language_uses_english() {
        let l = Localizer::from_sources(
            Language::Ru,
            "both = Оба\n",
            "both = Both\nonly-en = English only\n",
        )
        .unwrap();
        assert_eq!(l.tr("both"), "Оба");
        assert_eq!(l.tr("only-en"), "English only");
    }

    #[test]
    fn message_without_value_counts_as_missing() {
        let l = Localizer::from_sources(
            Language::Ru,
            "x =\n    .title = Т\n",
            "x = English\ny =\n    .title = T\n",
        )
        .unwrap();
        assert_eq!(l.tr("x"), "English");
        assert_eq!(l.tr("y"), "y");
    }

    #[test]
    fn set_language_updates_language_and_bundle() {
        let mut l = Localizer::new(Language::Ru).unwrap();
        assert_eq!(l.language(), Language::Ru);
        l.set_language(Language::Hy).unwrap();
        assert_eq!(l.language(), Language::Hy);
        l.set_language(Language::En).unwrap();
        assert_eq!(l.language(), Language::En);
        assert_eq!(l.tr("nav-settings"), "Settings");
    }

    #[test]
    fn bad_sources_are_parse_errors() {
        let dup = Localizer::from_sources(Language::Ru, "dup = 1\ndup = 2\n", SOURCES[0].1);
        assert!(
            matches!(&dup, Err(I18nError::Parse { lang: "ru", detail }) if detail.contains("\"dup\"")),
            "{dup:?}"
        );
        let syntax = Localizer::from_sources(Language::Hy, "ok = Fine\nbad = { $\n", SOURCES[0].1);
        assert!(
            matches!(&syntax, Err(I18nError::Parse { lang: "hy", detail }) if detail.contains("line 2")),
            "{syntax:?}"
        );
    }

    #[test]
    fn localizer_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Localizer>();
    }

    // Warn-once logging.

    /// Counts `WARN` events on the thread it is installed on.
    struct WarnCounter(Arc<AtomicUsize>);

    impl tracing::Subscriber for WarnCounter {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    fn warnings_during(f: impl FnOnce()) -> usize {
        let count = Arc::new(AtomicUsize::new(0));
        tracing::subscriber::with_default(WarnCounter(Arc::clone(&count)), f);
        count.load(Ordering::Relaxed)
    }

    #[test]
    fn missing_key_warns_once() {
        let l = Localizer::new(Language::Ru).unwrap();
        let warnings = warnings_during(|| {
            for _ in 0..3 {
                assert_eq!(l.tr("no-such-key"), "no-such-key");
            }
        });
        assert_eq!(warnings, 1);
    }

    #[test]
    fn formatting_errors_keep_the_text_and_warn_once() {
        let l = Localizer::new(Language::En).unwrap();
        let warnings = warnings_during(|| {
            for _ in 0..3 {
                // No `phase` argument: Fluent leaves the placeable as text.
                assert_eq!(l.tr("empty-phase-pending"), "Available in Phase {$phase}");
            }
        });
        assert_eq!(warnings, 1);
    }

    #[test]
    fn warn_set_dedups_until_poisoned() {
        let l = Localizer::new(Language::En).unwrap();
        assert!(l.first_warning("k"));
        assert!(!l.first_warning("k"));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = l.warned.lock().unwrap();
            panic!("poisoning the warn-once set on purpose");
        }));
        assert!(l.warned.is_poisoned());
        assert!(l.first_warning("k"));
        assert!(l.first_warning("k"));
        assert_eq!(l.tr("still-works"), "still-works");
    }
}
