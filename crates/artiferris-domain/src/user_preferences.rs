use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// A language the interface (and the e-mails) are available in: `en`, `fr`, `es`, `it` or `de`, the frontend's `public/i18n/<lang>.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Fr,
    Es,
    It,
    De,
}

/// Every language, in the order the interface lists them.
pub const SUPPORTED_LANGUAGES: [Language; 5] = [Language::En, Language::Fr, Language::Es, Language::It, Language::De];

impl Language {
    /// Used when nothing is known of what the reader wants (an account that never chose, a browser whose languages are not translated).
    pub const FALLBACK: Language = Language::En;

    /// Only the exact lowercase code: `fr-FR` or `FR` are the client's job to reduce.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        SUPPORTED_LANGUAGES
            .iter()
            .copied()
            .find(|language| language.as_str() == raw)
            .ok_or_else(|| DomainError::UnsupportedLanguage(raw.chars().take(16).collect()))
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Fr => "fr",
            Language::Es => "es",
            Language::It => "it",
            Language::De => "de",
        }
    }
}

/// Settings a user picks for themselves, kept apart from the account itself so that reading a
/// session (which happens on every request) does not depend on them.
#[async_trait]
pub trait UserPreferencesPort: Send + Sync {
    /// `None` until the user has chosen (or the app has recorded) a language.
    async fn language(&self, user_id: Uuid) -> Result<Option<Language>, DomainError>;

    async fn set_language(&self, user_id: Uuid, language: Language) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_code_parses_to_itself() {
        for language in SUPPORTED_LANGUAGES {
            assert_eq!(Language::parse(language.as_str()).unwrap(), language);
        }
    }

    #[test]
    fn the_fallback_is_english_and_supported() {
        assert_eq!(Language::FALLBACK.as_str(), "en");
        assert!(SUPPORTED_LANGUAGES.contains(&Language::FALLBACK));
    }

    #[test]
    fn anything_else_is_refused() {
        for raw in ["", "xx", "FR", "fr-FR", " fr", "fr ", "français", "constructor"] {
            assert!(matches!(Language::parse(raw), Err(DomainError::UnsupportedLanguage(_))), "{raw:?}");
        }
    }

    #[test]
    fn the_refusal_does_not_echo_an_unbounded_input() {
        let Err(DomainError::UnsupportedLanguage(shown)) = Language::parse(&"x".repeat(10_000)) else { panic!("should be refused") };

        assert_eq!(shown.len(), 16);
    }
}
