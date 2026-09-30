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

    /// The first translated language of an `Accept-Language` header, in order of preference (`q` values, then position), by the primary
    /// subtag alone (`fr-CA` is French). Anything that says nothing usable, an absent header included, gives the fallback.
    pub fn from_accept_language(header: &str) -> Language {
        const MAX_RANGES: usize = 32;
        let mut ranges: Vec<(u32, usize, &str)> = header
            .split(',')
            .take(MAX_RANGES)
            .enumerate()
            .filter_map(|(position, range)| {
                let mut parts = range.split(';');
                let tag = parts.next()?.trim();
                let quality = parts.find_map(|param| param.trim().strip_prefix("q=").or_else(|| param.trim().strip_prefix("Q="))).map_or(Some(1000), |q| q.trim().parse::<f32>().ok().map(|q| (q.clamp(0.0, 1.0) * 1000.0) as u32))?;
                (quality > 0).then_some((quality, position, tag))
            })
            .collect();
        ranges.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranges
            .into_iter()
            .find_map(|(_, _, tag)| {
                let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
                Language::parse(&primary).ok()
            })
            .unwrap_or(Language::FALLBACK)
    }

    /// The `og:locale` of a page written in this language.
    pub fn og_locale(&self) -> &'static str {
        match self {
            Language::En => "en_US",
            Language::Fr => "fr_FR",
            Language::Es => "es_ES",
            Language::It => "it_IT",
            Language::De => "de_DE",
        }
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
    fn the_accept_language_header_picks_the_first_translated_language_by_preference() {
        for (header, expected) in [
            ("fr-CA,fr;q=0.9,en;q=0.8", Language::Fr),
            ("de-AT", Language::De),
            ("DE_de", Language::De),
            ("ja,pt-BR;q=0.9,it;q=0.5,fr;q=0.4", Language::It),
            ("en;q=0.3, es;q=0.8", Language::Es),
            ("fr;q=0, de;q=0.1", Language::De),
            ("es, it", Language::Es),
            ("*", Language::En),
            ("ja, zh-CN", Language::En),
            ("", Language::En),
            ("fr;q=nonsense", Language::En),
            ("constructor, toString", Language::En),
        ] {
            assert_eq!(Language::from_accept_language(header), expected, "{header:?}");
        }
    }

    #[test]
    fn every_language_has_an_og_locale_that_starts_with_its_code() {
        for language in SUPPORTED_LANGUAGES {
            assert!(language.og_locale().starts_with(language.as_str()), "{language:?}");
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
