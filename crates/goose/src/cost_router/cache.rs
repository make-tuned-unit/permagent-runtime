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
//! 3. Keep the prefix BYTE-STABLE across turns. Ordering the prefix correctly
//!    buys nothing if its bytes drift anyway, and the system prompt is where
//!    drift is easiest to introduce: a live counter, a relative timestamp, a
//!    map iteration order. [`SystemPromptParts`] is the seam — turn-specific
//!    context moves *behind* the breakpoint instead of being deleted.
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

/// The cacheable-prefix segments the coding harness emits when **no**
/// explicitly-referenced read-only context is pinned this turn, in order. The
/// Anthropic request builder anchors its `cache_control` breakpoints on exactly
/// these — the tool list (marked on the last tool spec, so all tool defs cache
/// as one block) and the system block, which carries the repo-map (#720) as a
/// system extra — ahead of the mutable `messages` tail. It is the runtime
/// realization of [`CANONICAL_PREFIX`] minus the [`PrefixSegment::ReadOnlyFiles`]
/// slot; when read-only context IS pinned the harness fills that slot — folded
/// into the *same* system breakpoint after the repo-map, no fifth breakpoint —
/// and realizes the full canonical prefix (see [`harness_prefix`]). The guard in
/// `crate::providers::formats::anthropic` asserts the emitted payload matches the
/// prefix for the read-only-present state, so a reorder or mid-prefix insertion
/// fails CI rather than silently discarding a warm cache.
pub const HARNESS_PREFIX: [PrefixSegment; 3] = [
    PrefixSegment::Tools,
    PrefixSegment::System,
    PrefixSegment::RepoMap,
];

/// The cacheable-prefix segments the coding harness emits this turn, given
/// whether explicitly-referenced read-only context (pinned files, CLAUDE.md-type
/// project context) is pinned into the cached system block.
///
/// With read-only context present the harness fills the reserved
/// [`PrefixSegment::ReadOnlyFiles`] slot — realizing the full
/// [`CANONICAL_PREFIX`] — by appending it *inside* the existing system breakpoint
/// after the repo-map (never a fifth breakpoint, never the volatile tail). With
/// none present it emits the base [`HARNESS_PREFIX`]. Either state stays in
/// canonical order and is cache-stable.
pub fn harness_prefix(has_read_only_files: bool) -> &'static [PrefixSegment] {
    if has_read_only_files {
        &CANONICAL_PREFIX
    } else {
        &HARNESS_PREFIX
    }
}

/// The lifetime of a provider prompt-cache entry, selected per request.
///
/// A `cache_control` breakpoint writes an entry that later requests can read at
/// ~**0.1×** the fresh-input price. The write itself is billed at a premium over
/// fresh input, and the premium scales with TTL: a 5-minute (`ephemeral`
/// default) write costs **1.25×**, a 1-hour extended write costs **2×**.
///
/// **Honest break-even (why 1 hour is opt-in, not the default).** A 1-hour write
/// costs `2 / 1.25 = 1.6×` a 5-minute write. The longer TTL only pays for itself
/// when the 5-minute window would otherwise go cold between reuses — a >5-minute
/// think-gap in an interactive coding session — and force a fresh re-write. The
/// extra `2 − 1.25 = 0.75×` paid up front is recovered the first time the 1-hour
/// entry serves a read that a cold 5-minute entry would instead have re-written
/// (a saved re-write is worth `1.25 − 0.1 = 1.15×`). So a single >5-min-gap reuse
/// within the hour already pays for the upgrade; below roughly **1.6× prefix
/// reuse within the hour** the 5-minute TTL is strictly cheaper. Default 5 minutes
/// everywhere; opt into 1 hour only for long interactive / recipe-driven coding
/// sessions where think-gaps routinely exceed 5 minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheTtl {
    /// The `ephemeral` default: entries live ~5 minutes; write premium ~1.25×.
    #[default]
    FiveMinute,
    /// The extended `"ttl": "1h"` option: entries live ~1 hour; write premium
    /// ~2×. Break-even ~1.6× prefix reuse within the hour (see type docs).
    OneHour,
}

impl CacheTtl {
    /// The extended-TTL wire string (`"1h"`) for the 1-hour option, or `None`
    /// for the 5-minute default (which is the bare `ephemeral` marker, no `ttl`
    /// field). Kept JSON-free so this module stays pure policy — the Anthropic
    /// request builder turns this into the `cache_control` value.
    pub fn extended_ttl_str(self) -> Option<&'static str> {
        match self {
            CacheTtl::FiveMinute => None,
            CacheTtl::OneHour => Some("1h"),
        }
    }

    /// True for the 1-hour extended TTL (a longer, pricier write); false for the
    /// 5-minute default.
    pub fn is_extended(self) -> bool {
        matches!(self, CacheTtl::OneHour)
    }
}

// ── The system prompt's stable/volatile split ──────────────────────────────

/// A system prompt cut into the byte-stable prefix that carries the provider
/// `cache_control` breakpoint and the turn-volatile suffix that rides *after*
/// it, outside the cache.
///
/// Prompt caches are prefix-exact. A single drifting byte anywhere in the cached
/// block — a live memory count, a briefing that was unread last turn and is
/// acknowledged this turn, a probe result that flapped — invalidates every token
/// after it, which in a single-block system prompt means the entire system
/// prompt AND everything downstream of it. The fix is not to delete the volatile
/// context (it is load-bearing: the agent must know what its workers just
/// reported) but to move it behind the breakpoint, where changing it costs only
/// its own tokens.
///
/// Lives here rather than in `agents::prompt_manager` because both sides need
/// it: the prompt manager produces it, the provider request builders consume it,
/// and neither should own a type the other depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemPromptParts {
    stable_prefix: String,
    volatile_suffix: String,
}

impl SystemPromptParts {
    pub fn new(stable_prefix: String, volatile_suffix: String) -> Self {
        Self {
            stable_prefix,
            volatile_suffix,
        }
    }

    /// A prompt with nothing volatile in it. Used by callers that assemble a
    /// prompt with no turn-specific tail at all (recipe creation, subagents) and
    /// by the flattening fallback in providers that cannot express a breakpoint.
    pub fn all_stable(stable_prefix: String) -> Self {
        Self {
            stable_prefix,
            volatile_suffix: String::new(),
        }
    }

    /// The bytes that must not change from turn to turn within a session.
    pub fn stable_prefix(&self) -> &str {
        &self.stable_prefix
    }

    /// The turn-specific tail. Free to change every turn.
    pub fn volatile_suffix(&self) -> &str {
        &self.volatile_suffix
    }

    /// The whole prompt as one string, for providers with no cache breakpoint to
    /// place. Concatenation only — no separator — so a caller that splits at a
    /// different boundary still sends the same bytes.
    pub fn render(&self) -> String {
        format!("{}{}", self.stable_prefix, self.volatile_suffix)
    }

    pub fn is_empty(&self) -> bool {
        self.stable_prefix.is_empty() && self.volatile_suffix.is_empty()
    }

    /// Rewrite the stable prefix in place, keeping the volatile tail.
    ///
    /// For transforms that append *stable* text (the toolshim's tool-JSON
    /// instructions). Deliberately not a `map_render`: folding the volatile tail
    /// into the prefix would make the "stable" prefix change every turn, which is
    /// the exact defect this type exists to prevent.
    pub fn map_stable(self, f: impl FnOnce(String) -> String) -> Self {
        Self {
            stable_prefix: f(self.stable_prefix),
            volatile_suffix: self.volatile_suffix,
        }
    }

    /// Short SHA-256 prefix of the stable prefix, for the per-turn debug log.
    ///
    /// Truncated to 16 hex chars: this identifies a prefix across turns in a log,
    /// it does not authenticate one, and a full 64-char digest on every turn is
    /// noise. SHA-256 rather than `DefaultHasher` because the latter's output is
    /// explicitly not guaranteed stable across Rust versions, and a hash that
    /// silently changes under a toolchain bump would read as a cache regression.
    pub fn prefix_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.stable_prefix.as_bytes());
        // Truncate the DIGEST BYTES, not the hex string: slicing a `String` by
        // byte index is a panic waiting on a non-ASCII input, and there is no
        // reason to build the full 64-char encoding just to throw half away.
        hex::encode(&h.finalize()[..8])
    }
}

/// Policy: a conversation's main-loop model MUST stay stable for its lifetime.
/// Cheaper-tier work routes through subagents instead of swapping the model.
pub const KEEP_MAIN_LOOP_MODEL_STABLE: bool = true;

/// Policy: cheaper-tier work runs in a SEPARATE subagent context (each with its
/// own cache) rather than by swapping the live loop's model (which busts it).
pub const ROUTE_CHEAPER_TIERS_VIA_SUBAGENT: bool = true;

/// The identity a prompt cache is scoped to. Caches are held per **provider AND
/// model** — the same model id served by two providers is two distinct caches —
/// so both fields participate in every comparison. Carrying them as one value
/// (rather than four loose `&str` params) makes a transposed argument a type
/// error instead of a silent wrong answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelKey<'a> {
    pub provider: &'a str,
    pub model: &'a str,
}

impl<'a> ModelKey<'a> {
    pub fn new(provider: &'a str, model: &'a str) -> Self {
        Self { provider, model }
    }
}

/// True iff changing from `old` to `new` would bust a cache. Any change to
/// either half of the key does — caches never transfer between models, nor
/// between providers serving the same model id. This is the guard behind
/// "don't swap the main-loop model mid-conversation".
pub fn model_change_breaks_cache(old: ModelKey<'_>, new: ModelKey<'_>) -> bool {
    old != new
}

/// Whether it is safe to route a `desired`-tier unit of work by swapping the
/// live main-loop model. It is safe ONLY when the cache key does not actually
/// change (same provider and model as the loop already runs). Any real swap must
/// instead go through a subagent — so a caller that would change either half of
/// the key is told `false`.
pub fn may_swap_main_loop_model(current: ModelKey<'_>, desired: ModelKey<'_>) -> bool {
    !model_change_breaks_cache(current, desired)
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
        let anthropic = |m| ModelKey::new("anthropic", m);
        assert!(model_change_breaks_cache(
            anthropic("claude-frontier"),
            anthropic("cheap-cloud")
        ));
        assert!(model_change_breaks_cache(
            ModelKey::new("ollama", "qwen2.5:7b"),
            anthropic("claude-frontier")
        ));
        assert!(!model_change_breaks_cache(
            anthropic("claude-frontier"),
            anthropic("claude-frontier")
        ));
    }

    #[test]
    fn same_model_id_on_a_different_provider_still_breaks_the_cache() {
        // Regression (#941): the guard compared only the model string, so a
        // provider swap that kept the model id read as "safe" and silently
        // discarded the warm prefix. Caches are provider-scoped — this is a bust.
        let model = "claude-sonnet-5";
        assert!(model_change_breaks_cache(
            ModelKey::new("anthropic", model),
            ModelKey::new("bedrock", model)
        ));
        assert!(!may_swap_main_loop_model(
            ModelKey::new("anthropic", model),
            ModelKey::new("bedrock", model)
        ));
    }

    #[test]
    fn swapping_the_main_loop_model_is_refused_when_it_would_change() {
        // Routing a cheaper tier by swapping the loop model is refused — the
        // caller must use a subagent instead.
        assert!(!may_swap_main_loop_model(
            ModelKey::new("anthropic", "claude-frontier"),
            ModelKey::new("ollama", "qwen2.5:7b")
        ));
        // A no-op "swap" to the same key is fine (nothing is invalidated).
        assert!(may_swap_main_loop_model(
            ModelKey::new("anthropic", "claude-frontier"),
            ModelKey::new("anthropic", "claude-frontier")
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

    // ── The harness's real emitted prefix realizes the canonical policy ────

    #[test]
    fn harness_prefix_is_canonical_and_cache_stable() {
        // What the coding harness actually emits is itself cache-stable…
        assert!(prefix_is_cache_stable(&HARNESS_PREFIX));
        // …every emitted segment is a canonical segment, in canonical order…
        assert!(HARNESS_PREFIX.iter().all(|s| CANONICAL_PREFIX.contains(s)));
        assert!(HARNESS_PREFIX.windows(2).all(|w| {
            let pos = |seg| CANONICAL_PREFIX.iter().position(|s| s == seg);
            pos(&w[0]) < pos(&w[1])
        }));
        // …and the repo-map rides *after* the system prompt, inside the cached
        // prefix — the #720 placement invariant, never before it.
        let sys = HARNESS_PREFIX
            .iter()
            .position(|s| *s == PrefixSegment::System)
            .unwrap();
        let map = HARNESS_PREFIX
            .iter()
            .position(|s| *s == PrefixSegment::RepoMap)
            .unwrap();
        assert!(sys < map);
    }

    // ── Pinning read-only context fills the reserved slot ──────────────────

    #[test]
    fn read_only_context_fills_the_reserved_slot() {
        // No pinned read-only context → the base three-segment prefix.
        assert_eq!(harness_prefix(false), &HARNESS_PREFIX[..]);
        // Pinned read-only context → the full canonical prefix, ReadOnlyFiles
        // slot filled. It rides after the repo-map and stays cache-stable, so
        // filling the slot never busts the warm prefix.
        assert_eq!(harness_prefix(true), &CANONICAL_PREFIX[..]);
        assert!(prefix_is_cache_stable(harness_prefix(true)));
        assert!(prefix_is_cache_stable(harness_prefix(false)));
        // The filled slot is a strict superset that only *appends* — the base
        // segments keep their positions, so a warm read of the shorter prefix is
        // still a prefix of the longer one.
        assert!(harness_prefix(true).starts_with(harness_prefix(false)));
        assert_eq!(
            *harness_prefix(true).last().unwrap(),
            PrefixSegment::ReadOnlyFiles
        );
    }

    // ── Cache TTL policy: 5-minute default, 1-hour opt-in ──────────────────

    #[test]
    fn cache_ttl_defaults_to_five_minutes() {
        // The default everywhere is the cheap 5-minute write; 1 hour is opt-in.
        assert_eq!(CacheTtl::default(), CacheTtl::FiveMinute);
        assert!(!CacheTtl::FiveMinute.is_extended());
        assert!(CacheTtl::OneHour.is_extended());
    }

    // ── The stable/volatile split ─────────────────────────────────────────

    #[test]
    fn render_is_prefix_then_suffix_with_nothing_inserted() {
        // The split must be invisible on the wire for providers that flatten it:
        // a separator here would change the prompt every provider sees.
        let parts = SystemPromptParts::new("STABLE".into(), "\n\nVOLATILE".into());
        assert_eq!(parts.render(), "STABLE\n\nVOLATILE");
        assert_eq!(
            SystemPromptParts::all_stable("only".into()).render(),
            "only"
        );
    }

    #[test]
    fn prefix_hash_ignores_the_volatile_tail() {
        // The whole point: a changed tail must NOT read as a changed prefix, or
        // the regression log would cry wolf on every turn.
        let a = SystemPromptParts::new("STABLE".into(), "turn 1".into());
        let b = SystemPromptParts::new("STABLE".into(), "turn 2 — totally different".into());
        assert_eq!(a.prefix_hash(), b.prefix_hash());

        let c = SystemPromptParts::new("STABLE ".into(), "turn 1".into());
        assert_ne!(a.prefix_hash(), c.prefix_hash());
        assert_eq!(a.prefix_hash().len(), 16);
    }

    #[test]
    fn map_stable_leaves_the_volatile_tail_where_it_is() {
        // The toolshim path appends to the prefix. If it folded the tail in, the
        // "stable" prefix would change every turn — the defect, not the fix.
        let parts = SystemPromptParts::new("base".into(), "\n\ntail".into())
            .map_stable(|s| format!("{s}\n\ntools"));
        assert_eq!(parts.stable_prefix(), "base\n\ntools");
        assert_eq!(parts.volatile_suffix(), "\n\ntail");
        assert_eq!(parts.render(), "base\n\ntools\n\ntail");
    }

    #[test]
    fn cache_ttl_extended_string_is_only_set_for_one_hour() {
        // 5-minute is the bare `ephemeral` marker (no ttl field); 1-hour carries
        // the extended "1h" wire value.
        assert_eq!(CacheTtl::FiveMinute.extended_ttl_str(), None);
        assert_eq!(CacheTtl::OneHour.extended_ttl_str(), Some("1h"));
    }
}
