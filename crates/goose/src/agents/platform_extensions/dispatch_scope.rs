//! Per-dispatch extension scopes for goal workers.
//!
//! Selection rides `metadata_json.dispatch_extension_scope` on the goal card
//! (set by the `goal_advance` tool's `extension_scope` argument). The key is
//! sticky: a goal keeps its scope across re-dispatches until it is changed.

use crate::config::agent_identity::WorkerEngineKind;

/// Card metadata key naming the extensions allowed for the next dispatch.
pub const DISPATCH_EXTENSION_SCOPE_KEY: &str = "dispatch_extension_scope";

/// Resolve a dispatch extension scope from card metadata.
///
/// Absence inherits the worker's ordinary extension set. A present array is
/// normalized without losing an intentional empty denial. A scope containing
/// only whitespace normalizes to an empty scope (total denial); this is an
/// intentional fail-closed result. Malformed values fail closed by refusing
/// dispatch rather than silently changing authority.
pub fn extension_scope_from_metadata(
    meta: &serde_json::Value,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = meta.get(DISPATCH_EXTENSION_SCOPE_KEY) else {
        return Ok(None);
    };
    let entries = value.as_array().ok_or_else(|| {
        format!("card metadata '{DISPATCH_EXTENSION_SCOPE_KEY}' must be an array of strings")
    })?;

    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::trim)
                .map(crate::config::name_to_key)
                .ok_or_else(|| {
                    format!(
                        "card metadata '{DISPATCH_EXTENSION_SCOPE_KEY}' must contain only strings"
                    )
                })
        })
        .filter(|entry| entry.as_ref().map_or(true, |value| !value.is_empty()))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The label of the engine that would SILENTLY IGNORE a dispatch scope, or
/// `None` when this process composes the worker's tool set and can honestly
/// enforce one. Returning the label rather than a bool keeps the refusal
/// message's engine name on the same code path as the decision, so there is no
/// second lookup — and no unreachable `expect` — between deciding to refuse and
/// naming why.
///
/// An absent roster entry falls through to the default `InternalSubagent` match
/// arm in `dispatch_goal_fn`, whose tool set is composed in-process, so absence
/// is enforceable.
pub(super) fn unenforceable_engine_label(
    engine: Option<&WorkerEngineKind>,
) -> Option<&'static str> {
    engine.filter(|kind| !kind.grants_enforced()).map(|kind| {
        // Reuses the same predicate the agents surface uses to label a grant
        // enforced-or-advisory, so a scope and a grant can never disagree about
        // which engines this process controls.
        kind.label()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::ExtensionConfig;
    use crate::config::narrow_extensions_for_agent;

    fn builtin(name: &str) -> ExtensionConfig {
        ExtensionConfig::Builtin {
            name: name.to_string(),
            description: String::new(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    /// A present scope preserves trimmed, non-empty names in source order.
    #[test]
    fn parses_and_normalizes_scope() {
        let meta = serde_json::json!({
            DISPATCH_EXTENSION_SCOPE_KEY: [" developer ", "", "bravesearch"]
        });
        assert_eq!(
            extension_scope_from_metadata(&meta),
            Ok(Some(vec![
                "developer".to_string(),
                "bravesearch".to_string()
            ]))
        );
    }

    /// Display names normalize to keys and retain the matching base extension.
    #[test]
    fn display_name_scope_matches_extension_key() {
        let meta = serde_json::json!({ DISPATCH_EXTENSION_SCOPE_KEY: ["Brave Search"] });
        let scope = extension_scope_from_metadata(&meta).unwrap().unwrap();

        assert_eq!(scope, vec!["bravesearch"]);
        assert_eq!(
            narrow_extensions_for_agent(vec![builtin("bravesearch")], Some(&scope)),
            vec![builtin("bravesearch")]
        );
    }

    /// Honesty property: a scope may only be accepted for an engine whose tool
    /// set THIS process composes. Every engine kind is named explicitly so that
    /// adding a variant to `WorkerEngineKind` without deciding its scope story
    /// fails here rather than shipping a scope the engine silently ignores.
    #[test]
    fn only_in_process_engines_can_enforce_a_scope() {
        let external = WorkerEngineKind::ExternalCli {
            bin: "claude".to_string(),
            args: Vec::new(),
        };
        let supervised = WorkerEngineKind::SupervisedCli {
            bin: "claude".to_string(),
        };

        // Enforceable: in-process, and an absent roster entry (which dispatches
        // as an in-process subagent).
        assert_eq!(
            unenforceable_engine_label(Some(&WorkerEngineKind::InternalSubagent)),
            None
        );
        assert_eq!(unenforceable_engine_label(None), None);

        // Not enforceable — and the refusal names the engine.
        assert_eq!(
            unenforceable_engine_label(Some(&external)),
            Some("external_cli")
        );
        assert_eq!(
            unenforceable_engine_label(Some(&supervised)),
            Some("supervised_cli")
        );
        assert_eq!(
            unenforceable_engine_label(Some(&WorkerEngineKind::Pending)),
            Some("pending")
        );
    }

    /// An empty array is an explicit denial, distinct from an absent scope.
    #[test]
    fn empty_array_denies_everything() {
        let meta = serde_json::json!({ DISPATCH_EXTENSION_SCOPE_KEY: [] });
        assert_eq!(extension_scope_from_metadata(&meta), Ok(Some(vec![])));
        assert_eq!(
            extension_scope_from_metadata(&serde_json::json!({})),
            Ok(None)
        );
    }

    /// Malformed metadata must refuse dispatch instead of silently widening.
    #[test]
    fn malformed_metadata_errors() {
        assert!(extension_scope_from_metadata(
            &serde_json::json!({ DISPATCH_EXTENSION_SCOPE_KEY: "developer" })
        )
        .is_err());
        assert!(extension_scope_from_metadata(
            &serde_json::json!({ DISPATCH_EXTENSION_SCOPE_KEY: ["developer", 7] })
        )
        .is_err());
    }

    /// Unrelated role metadata must not create an extension scope.
    #[test]
    fn dispatch_role_alone_has_no_scope() {
        assert_eq!(
            extension_scope_from_metadata(&serde_json::json!({ "dispatch_role": "debugger" })),
            Ok(None)
        );
    }
}
