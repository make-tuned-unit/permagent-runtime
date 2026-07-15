//! Prompt-cache discipline: keep model-scoped, prefix-exact caches warm.
//!
//! Provider prompt caches are (a) MODEL-SCOPED and (b) PREFIX-EXACT: a hit needs
//! the same model AND a byte-identical leading prefix. Two rules follow, and
//! this module encodes both as checkable policy so the tiered router does not
//! silently throw away a warm cache while chasing a cheaper tier.
//!
//! 1. NEVER swap a conversation's main-loop model mid-conversation — it discards
//!    the entire cache. Route cheaper tiers via SEPARATE subagent contexts, each
//!    keeping its own model-scoped cache. (This is *why* the tiered router
//!    escalates through subagents rather than mutating the live loop's model.)
//!
//! 2. Keep the cacheable prefix STABLE and correctly ORDERED, most-static first:
//!    tools → system → repo-map → read-only files → (mutable tail). Any
//!    reordering or mid-prefix insertion invalidates every token after the
//!    change, so callers must assemble the prefix in canonical order.
//!
//! Pure policy — no async/IO.

/// A segment of the cacheable prompt prefix, ordered most-static-first. The
/// discriminant order IS the canonical cache order (derived `Ord`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrefixSegment {
    /// Tool definitions — change least often, so they anchor the prefix.
    Tools,
    /// The system prompt.
    System,
    /// The repo map (the ranked-tags codebase map, #712) — stable per session.
    RepoMap,
    /// Read-only context files pinned for the session.
    ReadOnlyFiles,
}

/// The canonical order the cacheable prefix must be assembled in.
pub const CANONICAL_PREFIX: [PrefixSegment; 4] = [
    PrefixSegment::Tools,
    PrefixSegment::System,
    PrefixSegment::RepoMap,
    PrefixSegment::ReadOnlyFiles,
];

/// Policy: a conversation's main-loop model MUST stay stable for its lifetime.
/// Cheaper-tier work routes through subagents instead of swapping the model.
pub const KEEP_MAIN_LOOP_MODEL_STABLE: bool = true;

/// Policy: cheaper-tier work runs in a SEPARATE subagent context (each with its
/// own cache) rather than by swapping the live loop's model (which busts it).
pub const ROUTE_CHEAPER_TIERS_VIA_SUBAGENT: bool = true;

/// True iff changing from `old_model` to `new_model` would bust a model-scoped
/// cache. Any model change does — caches never transfer between models. This is
/// the guard behind "don't swap the main-loop model mid-conversation".
pub fn model_change_breaks_cache(old_model: &str, new_model: &str) -> bool {
    old_model != new_model
}

/// Whether it is safe to route a `desired`-tier unit of work by swapping the
/// live main-loop model. It is safe ONLY when the model does not actually change
/// (same model as the loop already runs). Any real swap must instead go through
/// a subagent — so a caller that would change the model is told `false`.
pub fn may_swap_main_loop_model(current_model: &str, desired_model: &str) -> bool {
    !model_change_breaks_cache(current_model, desired_model)
}

/// True iff `segments` are in canonical cache order — each strictly after the
/// previous in `CANONICAL_PREFIX`. A reorder or a duplicate returns `false`:
/// the caller is about to silently invalidate the cached prefix. An empty or
/// single-segment prefix is trivially stable.
pub fn prefix_is_cache_stable(segments: &[PrefixSegment]) -> bool {
    segments.windows(2).all(|w| w[0] < w[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The cache-stability rule: no mid-conversation model swap ───────────

    #[test]
    fn any_model_change_breaks_the_cache() {
        assert!(model_change_breaks_cache("claude-frontier", "cheap-cloud"));
        assert!(model_change_breaks_cache("qwen2.5:7b", "claude-frontier"));
        assert!(!model_change_breaks_cache(
            "claude-frontier",
            "claude-frontier"
        ));
    }

    #[test]
    fn swapping_the_main_loop_model_is_refused_when_it_would_change() {
        // Routing a cheaper tier by swapping the loop model is refused — the
        // caller must use a subagent instead.
        assert!(!may_swap_main_loop_model("claude-frontier", "qwen2.5:7b"));
        // A no-op "swap" to the same model is fine (nothing is invalidated).
        assert!(may_swap_main_loop_model(
            "claude-frontier",
            "claude-frontier"
        ));
    }

    // ── The cacheable prefix must be ordered most-static-first ─────────────

    #[test]
    fn canonical_prefix_is_stable() {
        assert!(prefix_is_cache_stable(&CANONICAL_PREFIX));
    }

    #[test]
    fn a_correctly_ordered_subset_is_stable() {
        assert!(prefix_is_cache_stable(&[
            PrefixSegment::Tools,
            PrefixSegment::RepoMap,
        ]));
        assert!(prefix_is_cache_stable(&[PrefixSegment::System]));
        assert!(prefix_is_cache_stable(&[]));
    }

    #[test]
    fn a_reordered_prefix_is_flagged_unstable() {
        // system before tools would move the anchor and bust the cache.
        assert!(!prefix_is_cache_stable(&[
            PrefixSegment::System,
            PrefixSegment::Tools,
        ]));
        // read-only files before the repo map, likewise.
        assert!(!prefix_is_cache_stable(&[
            PrefixSegment::ReadOnlyFiles,
            PrefixSegment::RepoMap,
        ]));
    }

    #[test]
    fn a_duplicated_segment_is_flagged_unstable() {
        assert!(!prefix_is_cache_stable(&[
            PrefixSegment::Tools,
            PrefixSegment::Tools,
        ]));
    }
}
