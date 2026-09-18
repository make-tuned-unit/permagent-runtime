//! Process-wide cost-routing counters, emitted once by the headless exit sink.
//!
//! Lane R22b (M24 `dschat-select-existing-roadmap-resumes-not-redecomposes`
//! forensics): `record`/`set_planner` are called unconditionally from the
//! ONE shared dispatch seam every caller goes through
//! (`orchestrator::dispatch_goal_fn` — reached by the decompose→
//! `create_roadmap` nudge-dispatch hook, `resume_roadmap`'s
//! `dispatch_eligible_goals`, and `goal_advance(dispatch)`'s
//! `dispatch_goal_to` alike), so a dispatch through any of those three paths
//! already increments the SAME counters. Unlike [`crate::receipts::Registry`],
//! these counters had no [`reset`] at all — process-wide, and never zeroed
//! between runs — so a process that ran more than one headless session
//! (unlike today's one-shot-per-invocation `permagent-eval` subprocess model)
//! would leak a prior run's dispatch counts into the next run's printed
//! line. [`reset`] closes that gap; [`crate::receipts::begin_run`] calls it
//! alongside its own `Registry::reset()` so a fresh headless run always
//! starts this receipt at a genuine zero, not an inherited one.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static DISPATCHES: AtomicU64 = AtomicU64::new(0);
static CHEAPEST_CAPABLE: AtomicU64 = AtomicU64::new(0);
static PACK_HONORED: AtomicU64 = AtomicU64::new(0);
static PLANNER: Mutex<String> = Mutex::new(String::new());
// R48b (HARNESS_LOOP_LOG.md "A45 design", Addendum (c) / Q5): appended at
// the END of the existing `cost_routing` line (RECEIPT CONTRACT v1.3) —
// `routed_local`/`routed_cloud` count each dispatch's classified provider
// (`cost_router::is_local_provider`, the SAME classification the
// coordinator-visible routing roster uses), and `fallbacks` counts a goal
// redispatched once from a LOCAL worker tier to the coordinator/cloud tier
// after a context-exceeded, wall/turn-ceiling, or verify-failed-with-no-
// commit attempt. `dispatches`/`cheapest_capable`/`pack_honored`'s existing
// semantics are unchanged — these are pure additions.
static ROUTED_LOCAL: AtomicU64 = AtomicU64::new(0);
static ROUTED_CLOUD: AtomicU64 = AtomicU64::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Lane R22b: zero every counter and clear the planner, for the start of a
/// fresh headless CLI run — called from [`crate::receipts::begin_run`].
/// Mirrors `receipts::Registry::reset()`'s per-run contract; unlike that
/// registry, this module has no `known`/`unknown` distinction to reset,
/// since cost_routing is never `unknown` — see [`snapshot`]'s own doc
/// comment for why an unrouted run still prints an honest zero.
pub fn reset() {
    DISPATCHES.store(0, Ordering::Relaxed);
    CHEAPEST_CAPABLE.store(0, Ordering::Relaxed);
    PACK_HONORED.store(0, Ordering::Relaxed);
    ROUTED_LOCAL.store(0, Ordering::Relaxed);
    ROUTED_CLOUD.store(0, Ordering::Relaxed);
    FALLBACKS.store(0, Ordering::Relaxed);
    *PLANNER.lock().unwrap_or_else(|e| e.into_inner()) = String::new();
}

pub fn record(area: &str, key: &str) {
    if area != "cost_routing" {
        return;
    }
    let counter = match key {
        "dispatches" => &DISPATCHES,
        "cheapest_capable" => &CHEAPEST_CAPABLE,
        "pack_honored" => &PACK_HONORED,
        "routed_local" => &ROUTED_LOCAL,
        "routed_cloud" => &ROUTED_CLOUD,
        "fallbacks" => &FALLBACKS,
        _ => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    // Lane R17a: cost_routing is one of the four RECEIPT CONTRACT v1 lines
    // the receipts sidecar mirrors — refresh it on every counter change too,
    // not just the other three areas (`crate::receipts::write_sidecar` is a
    // no-op unless `PERMAGENT_RECEIPTS_SIDECAR` is set for this run).
    crate::receipts::write_sidecar();
}

/// Lane R63c: count ONE delegate child dispatch on the same counters a goal
/// dispatch increments (`orchestrator::dispatch_goal_fn`): `dispatches`, then
/// `cheapest_capable`/`pack_honored` per the audit, then `routed_local`/
/// `routed_cloud` by the same `is_local_provider` classification. The line's
/// keys are unchanged — a delegate child that runs a model is a dispatch, so
/// the existing `cheapest_capable == dispatches && pack_honored ==
/// dispatches` gate now sees it. `fallbacks` is goal-redispatch-only and is
/// not touched here.
pub fn record_delegate_dispatch(audit: super::delegate::DelegateCostAudit, provider: &str) {
    record("cost_routing", "dispatches");
    if audit.cheapest_capable {
        record("cost_routing", "cheapest_capable");
    }
    if audit.pack_honored {
        record("cost_routing", "pack_honored");
    }
    if super::is_local_provider(provider) {
        record("cost_routing", "routed_local");
    } else {
        record("cost_routing", "routed_cloud");
    }
}

/// What the `planner=` field prints when no decompose call answered this run
/// (r64 s5): an explicit value, not an unresolved planner.
pub const PLANNER_NONE: &str = "none";

/// Record the planner that physically ANSWERED this run's decompose call —
/// `provider` and the model its response named — never an intended route or
/// the coordinator. `decompose_roadmap` is the only caller.
pub fn set_planner(provider: &str, model: &str) {
    {
        let mut planner = PLANNER.lock().unwrap_or_else(|e| e.into_inner());
        *planner = format!("{provider}/{model}");
    }
    crate::receipts::write_sidecar();
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub dispatches: u64,
    pub cheapest_capable: u64,
    pub pack_honored: u64,
    pub planner: String,
    /// R48b: dispatches this run routed to a local/on-device worker tier.
    pub routed_local: u64,
    /// R48b: dispatches this run routed to a metered cloud worker tier.
    pub routed_cloud: u64,
    /// R48b: goals redispatched once from a LOCAL worker tier to the
    /// coordinator/cloud tier after a context-exceeded, wall/turn-ceiling,
    /// or verify-failed-with-no-commit attempt.
    pub fallbacks: u64,
}

/// Lane R22b: an unrouted run (no dispatch went through
/// `orchestrator::dispatch_goal_fn` this run) reads every counter as a real,
/// observed zero here — never `unknown` — because `record`/`set_planner`
/// have no gate to fail open on: the shared dispatch seam calls them
/// unconditionally on every successful spawn, so zero genuinely means "this
/// run dispatched nothing," the same honest reading Lane R18c's `NoEvidence`
/// handling and R20a's honest-escalate rule already give a zero-dispatch
/// coding-harness task (a `resume_roadmap` that finds nothing newly eligible,
/// or an `escalate`d task) credit for.
pub fn snapshot() -> Snapshot {
    let planner = PLANNER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Snapshot {
        dispatches: DISPATCHES.load(Ordering::Relaxed),
        cheapest_capable: CHEAPEST_CAPABLE.load(Ordering::Relaxed),
        pack_honored: PACK_HONORED.load(Ordering::Relaxed),
        planner: if planner.is_empty() {
            PLANNER_NONE.into()
        } else {
            planner
        },
        routed_local: ROUTED_LOCAL.load(Ordering::Relaxed),
        routed_cloud: ROUTED_CLOUD.load(Ordering::Relaxed),
        fallbacks: FALLBACKS.load(Ordering::Relaxed),
    }
}

impl Snapshot {
    pub fn format_line(&self) -> String {
        format!(
            "receipt cost_routing dispatches={} cheapest_capable={} pack_honored={} planner={} \
             routed_local={} routed_cloud={} fallbacks={}",
            self.dispatches,
            self.cheapest_capable,
            self.pack_honored,
            self.planner,
            self.routed_local,
            self.routed_cloud,
            self.fallbacks
        )
    }
}

pub fn format_line() -> String {
    snapshot().format_line()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_receipt_format_contract() {
        let receipt = Snapshot {
            dispatches: 3,
            cheapest_capable: 2,
            pack_honored: 3,
            planner: "provider/model".into(),
            routed_local: 2,
            routed_cloud: 1,
            fallbacks: 1,
        };
        assert_eq!(
            receipt.format_line(),
            "receipt cost_routing dispatches=3 cheapest_capable=2 pack_honored=3 \
             planner=provider/model routed_local=2 routed_cloud=1 fallbacks=1"
        );
    }

    /// Lane R22b: `Snapshot::format_line`'s zero-value shape, exactly as
    /// [`snapshot`] returns it for a freshly-[`reset`] (or never-dispatched)
    /// run — a plain literal check, like `routing_receipt_format_contract`
    /// above, so it can assert the exact zero line without touching the
    /// real process-wide statics (see the test below for that). This is the
    /// line a zero-dispatch coding-harness task (a `resume_roadmap` that
    /// found nothing newly eligible, or an honest `escalate`) must still
    /// print — never omitted — matching `permagent-eval`'s `harness_log.rs`
    /// cost_routing regex, which distinguishes "no evidence was ever
    /// produced" (no line at all) from "this run genuinely routed zero
    /// dispatches" (a line with every count `0`).
    #[test]
    fn routing_receipt_zero_dispatch_format_contract() {
        let receipt = Snapshot {
            dispatches: 0,
            cheapest_capable: 0,
            pack_honored: 0,
            planner: PLANNER_NONE.into(),
            routed_local: 0,
            routed_cloud: 0,
            fallbacks: 0,
        };
        assert_eq!(
            receipt.format_line(),
            "receipt cost_routing dispatches=0 cheapest_capable=0 pack_honored=0 \
             planner=none routed_local=0 routed_cloud=0 fallbacks=0"
        );
    }

    /// Lane R63c (M47 r63 s5): a delegate child is a dispatch. The s5 shape —
    /// one goal dispatch that was cheapest/pack-honoured, plus a reviewer
    /// delegate that ran on `claude-fable-5` against DeepSeek-pinned packs —
    /// must read `dispatches=2 cheapest_capable=1 pack_honored=1`, which the
    /// qualification gate (`cheapest_capable == dispatches`) now fails,
    /// instead of the `1/1/1` the record actually printed.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn a_delegate_child_is_counted_as_a_dispatch_with_its_own_audit() {
        reset();
        // The goal dispatch, exactly as `orchestrator::dispatch_goal_fn` records it.
        record("cost_routing", "dispatches");
        record("cost_routing", "cheapest_capable");
        record("cost_routing", "pack_honored");
        record("cost_routing", "routed_cloud");

        record_delegate_dispatch(
            super::super::delegate::DelegateCostAudit {
                cheapest_capable: false,
                pack_honored: false,
            },
            "anthropic",
        );
        let after = snapshot();
        assert!(after.dispatches >= 2, "{after:?}");
        assert!(
            after.cheapest_capable < after.dispatches,
            "an expensive delegate must not read as cheapest-capable: {after:?}"
        );
        assert!(
            after.pack_honored < after.dispatches,
            "a pin-leaving delegate must not read as pack-honoured: {after:?}"
        );
        assert!(after.routed_cloud >= 2, "{after:?}");

        // A cheapest, pack-honoured local delegate increments all three and
        // `routed_local`, never `fallbacks`.
        let before = snapshot();
        record_delegate_dispatch(
            super::super::delegate::DelegateCostAudit {
                cheapest_capable: true,
                pack_honored: true,
            },
            "ollama",
        );
        let local = snapshot();
        assert!(local.dispatches > before.dispatches);
        assert!(local.cheapest_capable > before.cheapest_capable);
        assert!(local.pack_honored > before.pack_honored);
        assert!(local.routed_local > before.routed_local);
        assert_eq!(local.fallbacks, before.fallbacks);
        reset();
    }

    /// Lane R22b (M24 `dschat-select-existing-roadmap-resumes-not-
    /// redecomposes` forensics): exercises the REAL global `reset`/`record`/
    /// `snapshot`/`format_line` API — the same statics
    /// `orchestrator::dispatch_goal_fn` mutates from every dispatch path
    /// (the decompose→`create_roadmap` nudge-dispatch hook, `resume_roadmap`'s
    /// `dispatch_eligible_goals`, and `goal_advance(dispatch)`'s
    /// `dispatch_goal_to` alike — all three share that one seam) and that
    /// [`crate::receipts::begin_run`] now also resets. Tagged with the SAME
    /// named lock `crate::receipts`'s own global-registry tests use
    /// (`receipts_global_registry`) because `begin_run` — exercised by those
    /// tests — now touches these exact statics too; see that module's own
    /// doc comment for why a shared key is required once two test groups
    /// touch the same global state. The post-`record` assertions are
    /// delta/floor checks (`>=`), not exact equality, so a concurrent
    /// production dispatch from an unrelated test elsewhere in this binary
    /// (a different, unnamed-key lock domain this test deliberately does not
    /// also claim) can only ever push the counters higher, never trip a
    /// false failure.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn reset_and_record_operate_on_the_real_process_wide_counters() {
        reset();
        let baseline = snapshot();
        assert_eq!(
            baseline,
            Snapshot {
                dispatches: 0,
                cheapest_capable: 0,
                pack_honored: 0,
                planner: PLANNER_NONE.into(),
                routed_local: 0,
                routed_cloud: 0,
                fallbacks: 0,
            },
            "reset() must zero every counter and clear the planner"
        );

        record("cost_routing", "dispatches");
        record("cost_routing", "cheapest_capable");
        record("cost_routing", "pack_honored");
        let after = snapshot();
        assert!(
            after.dispatches >= baseline.dispatches + 1,
            "record() must be additive: {after:?}"
        );
        assert!(after.cheapest_capable >= baseline.cheapest_capable + 1);
        assert!(after.pack_honored >= baseline.pack_honored + 1);

        // NOTE: a `\`-newline continuation does NOT join lines inside a RAW
        // string — the backslash, newline and indentation all become literal
        // regex characters and nothing matches. Keep this pattern on one line.
        let re = regex::Regex::new(
            r"^receipt cost_routing dispatches=\d+ cheapest_capable=\d+ pack_honored=\d+ planner=\S+ routed_local=\d+ routed_cloud=\d+ fallbacks=\d+$",
        )
        .unwrap();
        let line = format_line();
        assert!(
            re.is_match(&line),
            "must still parse after record(): {line}"
        );

        reset();
        assert_eq!(
            snapshot(),
            Snapshot {
                dispatches: 0,
                cheapest_capable: 0,
                pack_honored: 0,
                planner: PLANNER_NONE.into(),
                routed_local: 0,
                routed_cloud: 0,
                fallbacks: 0,
            },
            "reset() must fully clear the planner along with the counters, leaving the \
             binary's shared statics clean for any later test"
        );
    }

    /// r64 s5: the planner field names exactly what `set_planner` was given
    /// (the decompose call's answering provider and model), and `none` once
    /// reset with no decompose call.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn planner_field_names_the_answering_planner_or_none() {
        reset();
        assert_eq!(snapshot().planner, PLANNER_NONE);
        assert!(
            format_line().contains(" planner=none "),
            "{}",
            format_line()
        );
        set_planner("anthropic", "claude-haiku-4-5-20251001");
        assert_eq!(snapshot().planner, "anthropic/claude-haiku-4-5-20251001");
        reset();
        assert_eq!(snapshot().planner, PLANNER_NONE);
    }

    /// R48b: a plain local dispatch (no fallback) increments `routed_local`
    /// once and leaves `routed_cloud`/`fallbacks` at zero — the shape
    /// `dispatch_goal_fn` produces for an ordinary goal that started, and
    /// stayed, on the LOCAL worker tier. `begin_run()` (not a bare
    /// `reset()`) per this lane's own contract: it is the one entry point a
    /// real headless run calls, and it resets `receipts::Registry`
    /// alongside this module's own statics, exercising the same seam a
    /// live run does.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn a_local_dispatch_with_no_fallback_records_routed_local_only() {
        crate::receipts::begin_run();
        let baseline = snapshot();

        record("cost_routing", "dispatches");
        record("cost_routing", "routed_local");

        let after = snapshot();
        assert_eq!(after.dispatches, baseline.dispatches + 1);
        assert_eq!(after.routed_local, baseline.routed_local + 1);
        assert_eq!(after.routed_cloud, baseline.routed_cloud);
        assert_eq!(after.fallbacks, baseline.fallbacks);

        reset();
    }

    /// R48b: a fallback re-dispatch (local tier → coordinator/cloud tier)
    /// records `fallbacks=1` AND `routed_cloud=1` for the SAME dispatch —
    /// mirroring the two independent `record()` calls `dispatch_goal_fn`
    /// makes at its one recording point: the provider classification
    /// (`routed_cloud`, since the fallback always targets the coordinator/
    /// cloud `Orchestrate` tier) and the fallback flag (`fallbacks`) are
    /// both driven off `worker_fallback_applied`/the resolved provider,
    /// never off each other, so this test drives them independently too.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn a_fallback_dispatch_records_fallbacks_and_routed_cloud() {
        crate::receipts::begin_run();
        let baseline = snapshot();

        record("cost_routing", "dispatches");
        record("cost_routing", "routed_cloud");
        record("cost_routing", "fallbacks");

        let after = snapshot();
        assert_eq!(after.dispatches, baseline.dispatches + 1);
        assert_eq!(after.routed_cloud, baseline.routed_cloud + 1);
        assert_eq!(after.fallbacks, baseline.fallbacks + 1);
        assert_eq!(after.routed_local, baseline.routed_local);

        reset();
    }

    /// An unrecognized counter key (typo, or a key from an unrelated area)
    /// must be a silent no-op, exactly like the four pre-existing keys
    /// already guarantee — R48b's three new keys must not weaken that.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn record_ignores_unknown_keys_and_areas() {
        reset();
        let baseline = snapshot();
        record("cost_routing", "not_a_real_counter");
        record("some_other_area", "routed_local");
        assert_eq!(snapshot(), baseline);
    }
}
