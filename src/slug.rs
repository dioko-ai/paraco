//! Canonical application slugs shared by manifests, local URLs, and future catalogs.
//!
//! Validate rather than silently normalize: independently published names must
//! never collapse to the same URL. Uniqueness is enforced by the owning catalog
//! or server configuration, not by this syntax validator.
use std::fmt;

pub const MAX_LENGTH: usize = 63;
pub const PATTERN: &str = "^[a-z]([a-z0-9-]{0,61}[a-z0-9])?$";
pub const RULES: &str = "1-63 lowercase ASCII letters, digits, or hyphens, starting with a letter and ending with a letter or digit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSlug;

impl fmt::Display for InvalidSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "slug must be {RULES}")
    }
}
impl std::error::Error for InvalidSlug {}

pub fn validate(slug: &str) -> Result<(), InvalidSlug> {
    let bytes = slug.as_bytes();
    if (1..=MAX_LENGTH).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    {
        Ok(())
    } else {
        Err(InvalidSlug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_dns_slugs() {
        for slug in ["a", "hello", "ai-example", "app2", &"a".repeat(63)] {
            assert!(validate(slug).is_ok(), "{slug}");
        }
        for slug in [
            "",
            "Hello",
            "2app",
            "-app",
            "app-",
            "my app",
            "my_app",
            "a.b",
            "a/b",
            "café",
            "xn--cafe-dma-",
            &"a".repeat(64),
        ] {
            assert!(validate(slug).is_err(), "{slug}");
        }
    }

    #[test]
    fn manifest_schema_shares_slug_contract() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../docs/paraco-manifest.schema.json")).unwrap();
        assert_eq!(schema["properties"]["name"]["pattern"], PATTERN);
    }
}
