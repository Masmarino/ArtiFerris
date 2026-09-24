use crate::error::DomainError;

/// Every catalog is named `artiferris-<format>`, so anything starting with this can never belong to a user.
pub const RESERVED_NAME_PREFIX: &str = "artiferris-";

pub fn is_reserved_name(raw: &str) -> bool {
    raw.to_ascii_lowercase().starts_with(RESERVED_NAME_PREFIX)
}

pub fn reject_reserved_name(raw: &str) -> Result<(), DomainError> {
    if is_reserved_name(raw) {
        Err(DomainError::ReservedName(raw.to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_is_reserved_whatever_the_case() {
        assert!(is_reserved_name("artiferris-npm"));
        assert!(is_reserved_name("ArtiFerris-Docker"));
        assert!(is_reserved_name("artiferris-anything-else"));
    }

    #[test]
    fn similar_names_are_not_reserved() {
        assert!(!is_reserved_name("artiferris"));
        assert!(!is_reserved_name("my-artiferris-npm"));
        assert!(!is_reserved_name("artiferrisnpm"));
    }

    #[test]
    fn rejecting_names_the_offender() {
        assert_eq!(reject_reserved_name("artiferris-npm"), Err(DomainError::ReservedName("artiferris-npm".to_string())));
        assert_eq!(reject_reserved_name("acme"), Ok(()));
    }
}
