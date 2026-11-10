//! User-visible notices raised by core logic.

use crate::i18n::FluentArgs;

/// Severity of a [`Notice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    /// Informational.
    Info,
    /// Something went wrong but the app recovered.
    Warning,
    /// A failure the user should know about.
    Error,
}

/// A notice to show the user; `key` is a localization key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Severity.
    pub level: NoticeLevel,
    /// Localization key of the message.
    pub key: &'static str,
    /// Values for the message's placeables, by name.
    pub args: Vec<(&'static str, String)>,
}

impl Notice {
    /// A notice with no arguments.
    #[must_use]
    pub const fn new(level: NoticeLevel, key: &'static str) -> Self {
        Self {
            level,
            key,
            args: Vec::new(),
        }
    }

    /// Adds the value of the placeable `name`.
    #[must_use]
    pub fn with_arg(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.args.push((name, value.into()));
        self
    }

    /// The arguments in the form [`Localizer::tr_args`](crate::i18n::Localizer::tr_args) takes.
    ///
    /// A whole number becomes a Fluent number, because only a number selects a plural variant.
    /// Other text stays text, including text that merely parses as a number but would print
    /// differently, such as a folder named `007`.
    pub fn fluent_args(&self) -> FluentArgs<'_> {
        let mut args = FluentArgs::with_capacity(self.args.len());
        for (name, value) in &self.args {
            match value.parse::<i64>() {
                Ok(number) if number.to_string() == *value => args.set(*name, number),
                _ => args.set(*name, value.as_str()),
            }
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{i18n::Localizer, settings::Language};

    fn skipped(count: u64) -> Notice {
        Notice::new(NoticeLevel::Warning, "library-scan-skipped")
            .with_arg("count", count.to_string())
    }

    fn text(language: Language, notice: &Notice) -> String {
        Localizer::new(language)
            .unwrap()
            .tr_args(notice.key, &notice.fluent_args())
            .into_owned()
    }

    #[test]
    fn with_arg_keeps_the_order() {
        let notice = Notice::new(NoticeLevel::Info, "key")
            .with_arg("a", "1")
            .with_arg("b", String::from("2"));
        assert_eq!(notice.args, [("a", "1".to_owned()), ("b", "2".to_owned())]);
    }

    #[test]
    fn counts_select_plural_forms() {
        assert!(text(Language::En, &skipped(1)).contains("1 file or folder couldn't"));
        assert!(text(Language::En, &skipped(3)).contains("3 files or folders couldn't"));
        assert!(text(Language::Ru, &skipped(1)).contains("1 файл или папку"));
        assert!(text(Language::Ru, &skipped(3)).contains("3 файла или папки"));
        assert!(text(Language::Ru, &skipped(5)).contains("5 файлов или папок"));
        assert!(text(Language::Hy, &skipped(5)).contains('5'));
    }

    #[test]
    fn text_that_only_looks_like_a_number_stays_as_it_is() {
        for name in ["007", "1.50", "+1", "2024", "Música"] {
            let notice = Notice::new(NoticeLevel::Warning, "library-folder-unavailable")
                .with_arg("folder", name);
            assert!(text(Language::En, &notice).contains(name), "{name}");
        }
    }
}
