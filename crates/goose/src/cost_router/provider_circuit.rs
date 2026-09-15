//! Session-scoped provider circuit breaker (Lane R36a, harness loop
//! "A27 analysis" Miss 3 / "A32 analysis" credits section / Lane R28b).
//!
//! The planner's `complete_fast` decomposition and the salvage reviewer's
//! own `complete_fast` call both deliberately PREFER a "best model" over the
//! calling session's own provider (the 2026-09-09 ruling: "plan on the
//! strongest model, work on the cheapest graduated one"). Lane R28b already
//! taught the reviewer path to retry once on the session's tier provider
//! when the preferred one comes back billing/auth/outage-unavailable; A27
//! Miss 3 found the SAME shape in the planner's `resolve_decomposition_
//! provider` escalation. Both of those retries are correct and untouched by
//! this module — what was still missing is memory: on a machine whose
//! Anthropic key is out of credits, EVERY later planner/reviewer call in
//! the same session re-discovers that failure from scratch, paying a full
//! request (and its retry/timeout latency) before falling back again. The
//! M33 DeepSeek record traced exactly this cost: three tasks ended
//! `errored: provider unavailable: credits exhausted` after burning wall
//! budget on repeated planner attempts against the same dead account.
//!
//! This registry is keyed by SESSION ID, not a single flat flag — a bare
//! `static AtomicBool`/`static Mutex<bool>` would be a process-global that
//! leaks between unrelated sessions (two concurrent runs against different
//! provider configs would corrupt each other, and parallel tests would
//! interfere). Keying by session id gives each session's breaker its own,
//! independent lifetime: a provider's failure in session A never affects
//! session B, and a session that never sees a failure never touches another
//! session's entry. State is intentionally NOT persisted (no DB row, no
//! `metadata_json`) — it exists only to save the CURRENT process's repeated
//! calls; a fresh daemon restart or a new session starts with every circuit
//! closed, which is correct (a byte-length reload of the account balance is
//! not this module's job).
use std::collections::HashMap;
use std::sync::Mutex;

/// `session_id -> (provider_key -> reason_class)`. `provider_key` is
/// case-folded so `"Anthropic"` and `"anthropic"` open the same circuit.
static CIRCUITS: Mutex<Option<HashMap<String, HashMap<String, String>>>> = Mutex::new(None);

fn normalize(provider: &str) -> String {
    provider.trim().to_ascii_lowercase()
}

/// Does `session_id` already have `provider`'s circuit open? `false` for an
/// empty/unknown session id or provider — never treat "no evidence" as
/// "open".
pub fn is_open(session_id: &str, provider: &str) -> bool {
    reason_if_open(session_id, provider).is_some()
}

/// The reason class this session recorded for `provider`, if its circuit is
/// open — lets a caller build a skip message without a second lookup.
pub fn reason_if_open(session_id: &str, provider: &str) -> Option<String> {
    if session_id.is_empty() || provider.is_empty() {
        return None;
    }
    let guard = CIRCUITS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()?
        .get(session_id)?
        .get(&normalize(provider))
        .cloned()
}

/// Record that `provider` failed for `session_id` with `reason_class` (a
/// short tag such as `"billing"`, `"auth"`, `"rate_limit"`, `"server_error"`,
/// `"network"`, `"timeout"` — see [`crate::providers::errors::classify_
/// unavailable`], the ONLY intended source of `reason_class`, so the same
/// classification decides both "is this breaker-worthy" and "what reason
/// gets logged").
///
/// Returns `true` the FIRST time this `(session_id, provider)` pair opens —
/// callers use that to emit exactly one `provider_circuit_open` line per
/// session per provider, never a repeat on every subsequent skipped call.
/// A no-op (returns `false`) for an empty session id or provider — there is
/// nothing to scope the breaker to.
pub fn open(session_id: &str, provider: &str, reason_class: &str) -> bool {
    if session_id.is_empty() || provider.is_empty() {
        return false;
    }
    let mut guard = CIRCUITS.lock().unwrap_or_else(|e| e.into_inner());
    let sessions = guard.get_or_insert_with(HashMap::new);
    let providers = sessions.entry(session_id.to_string()).or_default();
    let key = normalize(provider);
    if providers.contains_key(&key) {
        false
    } else {
        providers.insert(key, reason_class.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per test so parallel tests in this binary never share an
    /// entry — the registry itself has no per-test reset, matching how the
    /// production code always addresses it by a real, unique session id.
    fn sid(tag: &str) -> String {
        format!("r36a-test-{tag}-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn closed_by_default_for_an_unseen_session_and_provider() {
        let session = sid("closed");
        assert!(!is_open(&session, "anthropic"));
        assert!(reason_if_open(&session, "anthropic").is_none());
    }

    /// "breaker opens on a billing error and the next planner selection
    /// skips the provider" (lane brief's test contract).
    #[test]
    fn opens_on_a_billing_class_error_and_is_then_seen_by_a_later_lookup() {
        let session = sid("billing-opens");
        assert!(!is_open(&session, "anthropic"));

        let newly_opened = open(&session, "anthropic", "billing");
        assert!(newly_opened, "first failure for this pair must be new");

        assert!(is_open(&session, "anthropic"));
        assert_eq!(
            reason_if_open(&session, "anthropic").as_deref(),
            Some("billing")
        );

        // A later "planner selection" is just another `is_open` lookup — it
        // must see the SAME open circuit without a second failure.
        assert!(is_open(&session, "anthropic"));
    }

    /// Recorded once; a second failure for the same pair is not "new" (so
    /// callers log the `provider_circuit_open` line exactly once).
    #[test]
    fn a_second_open_for_the_same_pair_is_not_reported_as_new() {
        let session = sid("second-open");
        assert!(open(&session, "anthropic", "billing"));
        assert!(!open(&session, "anthropic", "rate_limit"));
        // The FIRST reason class wins — it is not overwritten by the second.
        assert_eq!(
            reason_if_open(&session, "anthropic").as_deref(),
            Some("billing")
        );
    }

    /// Provider keys are case-insensitive: the planner and reviewer paths
    /// must never disagree because one spelled the provider differently.
    #[test]
    fn provider_lookup_is_case_insensitive() {
        let session = sid("case-fold");
        assert!(open(&session, "Anthropic", "billing"));
        assert!(is_open(&session, "anthropic"));
        assert!(is_open(&session, "ANTHROPIC"));
    }

    /// "a non-billing error does not open it" (lane brief's test contract):
    /// callers only call `open()` after their OWN classification decided the
    /// failure is breaker-worthy — a caller that never calls `open()` for a
    /// request-shaped failure leaves the circuit closed.
    #[test]
    fn a_provider_never_reported_via_open_stays_closed() {
        let session = sid("never-opened");
        // Simulate a caller that classified a 400 invalid_request_error as
        // NOT breaker-worthy and therefore never calls `open()` at all.
        assert!(!is_open(&session, "anthropic"));
    }

    /// "breaker state is per-session (two sessions independent)".
    #[test]
    fn two_sessions_are_independent() {
        let session_a = sid("indep-a");
        let session_b = sid("indep-b");

        assert!(open(&session_a, "anthropic", "billing"));

        assert!(is_open(&session_a, "anthropic"));
        assert!(
            !is_open(&session_b, "anthropic"),
            "session B must not see session A's open circuit"
        );

        // Session B can still use the same provider successfully — opening
        // its own breaker only for the failure it actually saw.
        assert!(!is_open(&session_b, "openrouter"));
    }

    #[test]
    fn empty_session_id_or_provider_never_opens_or_reports_open() {
        assert!(!open("", "anthropic", "billing"));
        let session = sid("empty-provider");
        assert!(!open(&session, "", "billing"));
        assert!(!is_open("", "anthropic"));
        assert!(!is_open(&session, ""));
    }
}
