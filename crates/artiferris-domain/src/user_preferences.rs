use async_trait::async_trait;
use uuid::Uuid;

use crate::error::DomainError;

/// The languages the interface is translated into (the frontend's `public/i18n/<lang>.json`).
pub const SUPPORTED_LANGUAGES: [&str; 5] = ["en", "fr", "es", "it", "de"];

/// A language code the interface is translated into, e.g. `fr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language(&'static str);

impl Language {
    /// Only the exact lowercase code: `fr-FR` or `FR` are the client's job to reduce.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        SUPPORTED_LANGUAGES
            .iter()
            .find(|code| **code == raw)
            .map(|code| Self(code))
            .ok_or_else(|| DomainError::UnsupportedLanguage(raw.chars().take(16).collect()))
    }

    pub fn as_str(&self) -> &'static str {
        self.0
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
        for code in SUPPORTED_LANGUAGES {
            assert_eq!(Language::parse(code).unwrap().as_str(), code);
        }
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
