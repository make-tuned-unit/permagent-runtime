//! Canonical entity ID normalization for Permagent.
//!
//! All canonical entity IDs in Permagent flow through [`canonicalize_entity_id`].
//! Spectral does not validate canonical_id format; consistency is Permagent's
//! responsibility. Every writer of entity references — activity events,
//! annotations, graph triples — must canonicalize through this helper.
//!
//! The migration path for Mesh-verified entities (`person:slug` →
//! `did:chitin:slug`) will use an alias table introduced in Phase 3+,
//! not in-place rewriting.
//!
//! # Format
//!
//! `{prefix}:{normalized_slug}` where normalized_slug is:
//! - lowercased
//! - non-ASCII stripped (v1; unicode slugs deferred)
//! - whitespace and underscores replaced with hyphens
//! - non-alphanumeric characters (except hyphens) removed
//! - repeated hyphens collapsed
//! - leading/trailing hyphens trimmed

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityPrefix {
    Person,
    Project,
    Org,
    Agent,
    Did(String),
}

impl fmt::Display for EntityPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityPrefix::Person => write!(f, "person"),
            EntityPrefix::Project => write!(f, "project"),
            EntityPrefix::Org => write!(f, "org"),
            EntityPrefix::Agent => write!(f, "agent"),
            EntityPrefix::Did(scheme) => write!(f, "did:{}", scheme),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalizeError {
    EmptyAfterNormalization,
}

impl fmt::Display for CanonicalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalizeError::EmptyAfterNormalization => {
                write!(f, "slug is empty after normalization")
            }
        }
    }
}

impl std::error::Error for CanonicalizeError {}

/// Normalize a human-readable name into a canonical entity ID.
///
/// # Examples
/// ```
/// use permagent::identity::canonical::*;
/// assert_eq!(
///     canonicalize_entity_id(EntityPrefix::Person, "Jesse Sharratt"),
///     Ok("person:jesse-sharratt".to_string())
/// );
/// assert_eq!(
///     canonicalize_entity_id(EntityPrefix::Project, "Get Ladle!"),
///     Ok("project:get-ladle".to_string())
/// );
/// assert_eq!(
///     canonicalize_entity_id(EntityPrefix::Did("chitin".into()), "Henry Malcolm"),
///     Ok("did:chitin:henry-malcolm".to_string())
/// );
/// ```
pub fn canonicalize_entity_id(
    prefix: EntityPrefix,
    slug: &str,
) -> Result<String, CanonicalizeError> {
    let normalized: String = slug
        .to_lowercase()
        // Strip non-ASCII (v1 decision: ASCII-only slugs)
        .chars()
        .filter(|c| c.is_ascii())
        // Replace whitespace and underscores with hyphens
        .map(|c| if c.is_ascii_whitespace() || c == '_' { '-' } else { c })
        // Keep only alphanumeric and hyphens
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();

    // Collapse repeated hyphens and trim
    let mut result = String::with_capacity(normalized.len());
    let mut last_was_hyphen = true; // true to trim leading hyphens
    for c in normalized.chars() {
        if c == '-' {
            if !last_was_hyphen {
                result.push('-');
                last_was_hyphen = true;
            }
        } else {
            result.push(c);
            last_was_hyphen = false;
        }
    }
    // Trim trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }

    if result.is_empty() {
        return Err(CanonicalizeError::EmptyAfterNormalization);
    }

    Ok(format!("{}:{}", prefix, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_person() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Person, "Jesse Sharratt"),
            Ok("person:jesse-sharratt".into())
        );
    }

    #[test]
    fn project_with_special_chars() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Project, "Get Ladle!"),
            Ok("project:get-ladle".into())
        );
    }

    #[test]
    fn did_scheme() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Did("chitin".into()), "Henry Malcolm"),
            Ok("did:chitin:henry-malcolm".into())
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Person, "   "),
            Err(CanonicalizeError::EmptyAfterNormalization)
        );
    }

    #[test]
    fn underscores_to_hyphens() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Project, "my_cool_project"),
            Ok("project:my-cool-project".into())
        );
    }

    #[test]
    fn mixed_case() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Org, "Atlas Atlantic"),
            Ok("org:atlas-atlantic".into())
        );
    }

    #[test]
    fn already_canonical_is_idempotent() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Person, "jesse-sharratt"),
            Ok("person:jesse-sharratt".into())
        );
    }

    #[test]
    fn unicode_stripped_in_v1() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Person, "Rene Descartes"),
            Ok("person:rene-descartes".into())
        );
    }

    #[test]
    fn repeated_hyphens_collapsed() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Project, "my---project"),
            Ok("project:my-project".into())
        );
    }

    #[test]
    fn leading_trailing_hyphens_trimmed() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Project, "---leading-trailing---"),
            Ok("project:leading-trailing".into())
        );
    }

    #[test]
    fn all_special_chars_empty() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Person, "!@#$%^&*()"),
            Err(CanonicalizeError::EmptyAfterNormalization)
        );
    }

    #[test]
    fn agent_prefix() {
        assert_eq!(
            canonicalize_entity_id(EntityPrefix::Agent, "Henry"),
            Ok("agent:henry".into())
        );
    }
}
