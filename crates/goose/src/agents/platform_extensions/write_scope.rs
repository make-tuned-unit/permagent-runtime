//! Deterministic write-scope collision checks for parallel goal dispatch.
//!
//! A git worktree isolates branches, but two workers editing the same path from
//! the same baseline still produce a merge conflict (and waste a full model
//! run). Goals may declare repository-relative paths in metadata; this module
//! keeps the collision rule pure so the dispatcher can refuse only the
//! overlapping pair and leave disjoint work parallel.

use std::collections::BTreeSet;

use serde_json::Value;

/// Goal metadata key containing an array of repository-relative write paths.
pub const WRITE_SCOPE_KEY: &str = "write_scope";

/// Parse and normalize a declared write scope. Absolute paths, traversal, and
/// non-string entries are rejected before dispatch; `None` means no scope was
/// declared (disjointness is therefore unproven).
pub fn declared_write_scope(meta: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(raw) = meta.get(WRITE_SCOPE_KEY) else {
        return Ok(None);
    };
    let values = raw.as_array().ok_or_else(|| {
        format!("{WRITE_SCOPE_KEY} must be an array of repository-relative paths")
    })?;
    let mut paths = BTreeSet::new();
    for value in values {
        let path = value
            .as_str()
            .ok_or_else(|| format!("{WRITE_SCOPE_KEY} entries must be strings"))?;
        let normalized = normalize_path(path).ok_or_else(|| {
            format!("{WRITE_SCOPE_KEY} contains an unsafe or empty path: {path:?}")
        })?;
        paths.insert(normalized);
    }
    Ok(Some(paths.into_iter().collect()))
}

/// Whether this node explicitly promises not to write repository files.
/// Missing, non-boolean, or false values are write-capable/unknown.
pub fn explicitly_read_only(meta: &Value) -> bool {
    meta.get("read_only").and_then(Value::as_bool) == Some(true)
}

fn normalize_path(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    if path.is_empty() || path.starts_with('/') {
        return None;
    }
    let mut components = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => components.push(part),
        }
    }
    if components.is_empty() || components[0].ends_with(':') {
        return None;
    }
    Some(components.join("/"))
}

/// Whether two declared scopes overlap. A directory claims all descendants,
/// so `src` conflicts with `src/lib.rs`, while `src2` does not conflict with
/// `src`.
pub fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|a| {
        right.iter().any(|b| {
            a == b
                || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
                || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
        })
    })
}

/// Return the first deterministic conflicting path pair, if any.
pub fn first_conflict(left: &[String], right: &[String]) -> Option<(String, String)> {
    let mut pairs: Vec<(String, String)> = left
        .iter()
        .flat_map(|a| {
            right
                .iter()
                .filter(move |b| {
                    a == *b
                        || a.strip_prefix(b.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                        || b.strip_prefix(a.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                })
                .map(move |b| (a.clone(), b.clone()))
        })
        .collect();
    pairs.sort();
    pairs.into_iter().next()
}

/// Whether `path` is inside `scope`, with the same directory semantics
/// [`scopes_overlap`] uses: a declared directory claims every descendant, and
/// `src2/x` is NOT inside `src`.
fn path_within(scope: &[String], path: &str) -> bool {
    scope.iter().any(|allowed| {
        allowed == path
            || path
                .strip_prefix(allowed.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Lane R71a (M55 r70 s2): the paths a goal actually changed that its declared
/// write scope does not cover, normalized and deduplicated, in deterministic
/// order.
///
/// The declared scope is what the dispatcher trusts when it runs siblings in
/// PARALLEL ([`metadata_conflict`]), but nothing used to compare it against the
/// diff the worker produced. A worker that renames a symbol in its own file and
/// then "helpfully" fixes the call site in a sibling's file breaks that promise
/// after the fact: the first sibling to finish lands, and the second can no
/// longer rebase (its own diff now overlaps the moved trunk), so correct work
/// is stranded and trunk keeps half the change. r70 s2
/// `select-mechanical-rename-skips-roadmap` failed exactly this way — scope
/// `["a.py"]`, diff `a.py` + `b.py`, landing refused `rebase_trunk_overlap`.
///
/// An empty or absent scope returns no violations: an undeclared scope was
/// never a promise, and those goals are serialized rather than parallelized.
pub fn out_of_scope_paths(scope: &[String], changed: &[String]) -> Vec<String> {
    if scope.is_empty() {
        return Vec::new();
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    for path in changed {
        let Some(normalized) = normalize_path(path) else {
            // A path git reports that does not normalize (absolute, traversal)
            // is reported verbatim: it is certainly not inside a declared
            // repo-relative scope, and swallowing it would hide a real escape.
            out.insert(path.clone());
            continue;
        };
        if !path_within(scope, &normalized) {
            out.insert(normalized);
        }
    }
    out.into_iter().collect()
}

/// Explain why two in-flight nodes cannot safely run together. `None` means
/// they are proven independent: either side is explicitly read-only, or both
/// sides declared non-overlapping non-empty scopes. Unknown/empty scopes are
/// serialized rather than optimistic, because a worktree does not prevent a
/// merge collision.
pub fn metadata_conflict(candidate: &Value, active: &Value) -> Result<Option<String>, String> {
    if explicitly_read_only(candidate) || explicitly_read_only(active) {
        return Ok(None);
    }
    let candidate_scope = declared_write_scope(candidate)?;
    let active_scope = declared_write_scope(active)?;
    match (candidate_scope, active_scope) {
        (Some(left), Some(right)) if !left.is_empty() && !right.is_empty() => {
            Ok(first_conflict(&left, &right)
                .map(|(a, b)| format!("declared paths '{a}' and '{b}' overlap")))
        }
        _ => Ok(Some(
            "write scope is missing or empty for a non-read-only node".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_normalized_sorted_and_deduplicated() {
        let scope = declared_write_scope(&serde_json::json!({
            WRITE_SCOPE_KEY: ["./src/./lib.rs", "src//", "src/lib.rs"]
        }))
        .unwrap();
        assert_eq!(
            scope,
            Some(vec!["src".to_string(), "src/lib.rs".to_string()])
        );
    }

    #[test]
    fn unsafe_scope_claims_are_rejected() {
        for value in [
            serde_json::json!({ WRITE_SCOPE_KEY: ["/tmp/out"] }),
            serde_json::json!({ WRITE_SCOPE_KEY: ["src/../secrets"] }),
            serde_json::json!({ WRITE_SCOPE_KEY: [""] }),
            serde_json::json!({ WRITE_SCOPE_KEY: "src/lib.rs" }),
        ] {
            assert!(declared_write_scope(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn directory_and_descendant_collide_but_prefix_names_do_not() {
        let src = vec!["src".into()];
        let child = vec!["src/lib.rs".into()];
        let sibling = vec!["src2/lib.rs".into()];
        assert!(scopes_overlap(&src, &child));
        assert!(!scopes_overlap(&src, &sibling));
        assert_eq!(
            first_conflict(&src, &child),
            Some(("src".into(), "src/lib.rs".into()))
        );
    }

    #[test]
    fn unknown_scope_serializes_behind_an_active_writer() {
        let candidate = serde_json::json!({});
        let active = serde_json::json!({ WRITE_SCOPE_KEY: ["src/lib.rs"] });
        let conflict = metadata_conflict(&candidate, &active).unwrap();
        assert!(conflict.unwrap().contains("missing or empty"));
    }

    #[test]
    fn explicit_read_only_nodes_can_parallelize_without_write_scopes() {
        let candidate = serde_json::json!({ "read_only": true });
        let active = serde_json::json!({});
        assert!(metadata_conflict(&candidate, &active).unwrap().is_none());
        assert!(metadata_conflict(&active, &candidate).unwrap().is_none());
    }

    #[test]
    fn only_explicit_disjoint_scopes_parallelize() {
        let left = serde_json::json!({ WRITE_SCOPE_KEY: ["src/a.rs"] });
        let right = serde_json::json!({ WRITE_SCOPE_KEY: ["src/b.rs"] });
        assert!(metadata_conflict(&left, &right).unwrap().is_none());

        let overlapping = serde_json::json!({ WRITE_SCOPE_KEY: ["src"] });
        assert!(metadata_conflict(&left, &overlapping).unwrap().is_some());
    }

    /// Lane R71a: the r70 s2 shape — scope `a.py`, diff touches `b.py` too.
    #[test]
    fn a_diff_outside_the_declared_scope_is_reported() {
        let scope = vec!["a.py".to_string()];
        let changed = vec!["a.py".to_string(), "b.py".to_string()];
        assert_eq!(out_of_scope_paths(&scope, &changed), vec!["b.py"]);
    }

    #[test]
    fn a_diff_inside_the_declared_scope_is_clean() {
        let scope = vec!["src".to_string(), "docs/readme.md".to_string()];
        let changed = vec![
            "src/lib.rs".to_string(),
            "src/nested/deep.rs".to_string(),
            "docs/readme.md".to_string(),
            "./src/normalized.rs".to_string(),
        ];
        assert!(out_of_scope_paths(&scope, &changed).is_empty());
    }

    /// A declared directory claims its descendants, but a sibling whose name
    /// merely starts with it does not — the same rule [`scopes_overlap`] uses.
    #[test]
    fn a_prefix_sibling_directory_is_out_of_scope() {
        let scope = vec!["src".to_string()];
        let changed = vec!["src2/lib.rs".to_string(), "srcfile.rs".to_string()];
        assert_eq!(
            out_of_scope_paths(&scope, &changed),
            vec!["src2/lib.rs".to_string(), "srcfile.rs".to_string()]
        );
    }

    /// An undeclared scope was never a promise: those goals are serialized,
    /// not parallelized, so there is nothing to enforce.
    #[test]
    fn an_empty_scope_enforces_nothing() {
        assert!(out_of_scope_paths(&[], &["anything.rs".to_string()]).is_empty());
    }

    /// Lane R71a: the containment direction matters. A path is inside a
    /// declared directory, but a declared FILE does not claim a directory that
    /// merely shares its name — this is what `sibling_owned_paths` relies on
    /// when it asks "does this sibling own that path?".
    #[test]
    fn containment_is_directional() {
        assert!(out_of_scope_paths(&["src/a.rs".to_string()], &["src".to_string()]).len() == 1);
        assert!(out_of_scope_paths(&["src".to_string()], &["src/a.rs".to_string()]).is_empty());
    }

    /// Deterministic, deduplicated output — the finding text must not vary
    /// between runs over the same diff.
    #[test]
    fn violations_are_sorted_and_deduplicated() {
        let scope = vec!["a".to_string()];
        let changed = vec![
            "z.rs".to_string(),
            "b.rs".to_string(),
            "z.rs".to_string(),
            "./b.rs".to_string(),
        ];
        assert_eq!(
            out_of_scope_paths(&scope, &changed),
            vec!["b.rs".to_string(), "z.rs".to_string()]
        );
    }
}
