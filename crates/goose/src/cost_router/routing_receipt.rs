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
        _ => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    // Lane R17a: cost_routing is one of the four RECEIPT CONTRACT v1 lines
    // the receipts sidecar mirrors — refresh it on every counter change too,
    // not just the other three areas (`crate::receipts::write_sidecar` is a
    // no-op unless `PERMAGENT_RECEIPTS_SIDECAR` is set for this run).
    crate::receipts::write_sidecar();
}

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
            "unknown/unknown".into()
        } else {
            planner
        },
    }
}

impl Snapshot {
    pub fn format_line(&self) -> String {
        format!(
            "receipt cost_routing dispatches={} cheapest_capable={} pack_honored={} planner={}",
            self.dispatches, self.cheapest_capable, self.pack_honored, self.planner
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
        };
        assert_eq!(
            receipt.format_line(),
            "receipt cost_routing dispatches=3 cheapest_capable=2 pack_honored=3 planner=provider/model"
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
            planner: "unknown/unknown".into(),
        };
        assert_eq!(
            receipt.format_line(),
            "receipt cost_routing dispatches=0 cheapest_capable=0 pack_honored=0 planner=unknown/unknown"
        );
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
                planner: "unknown/unknown".into(),
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

        let re = regex::Regex::new(
            r"^receipt cost_routing dispatches=\d+ cheapest_capable=\d+ pack_honored=\d+ planner=\S+$",
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
                planner: "unknown/unknown".into(),
            },
            "reset() must fully clear the planner along with the counters, leaving the \
             binary's shared statics clean for any later test"
        );
    }
}
