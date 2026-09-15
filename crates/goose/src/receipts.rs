//! RECEIPT CONTRACT v1 counters for one headless CLI run and its in-process workers.
//!
//! Only the headless entry point enables collection. The CLI runs one headless
//! session at a time; concurrent independent runs in one process are unsupported.
//! Unknown sources stay unknown even when no event was observed. These are
//! detector counts, not an assertion that the detectors cover every attack.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

const FIELDS: [(&str, &str, bool); 17] = [
    ("dag_enforcement", "roadmap_dispatched", true),
    ("dag_enforcement", "bypass_attempts", true),
    ("dag_enforcement", "bypass_passes", true),
    ("dag_enforcement", "coordinator_mutations", true),
    // Lane R20a (RECEIPT CONTRACT v1.1): the qualification gate's
    // `roadmap_dispatched >= 1` requirement over-penalized two honest,
    // held-out coordinator shapes that correctly finish a coding-harness
    // task without ever dispatching a NEW roadmap: resuming an existing
    // paused roadmap (`orchestrator::handle_resume_roadmap`) and recording
    // an honest `escalate` decision when the task cannot proceed
    // (`orchestrator::record_escalation`). These two counters give the
    // qualification gate (`permagent-eval::qualification`) real, observed
    // evidence for those shapes instead of forcing every attempted task
    // through the roadmap-dispatch machinery. Appended after the existing
    // v1 dag_enforcement keys so a legacy v1 receipt line (no trailing
    // keys) still parses — see `permagent-eval::harness_log`'s
    // `DAG_ENFORCEMENT_RE` for the backward-compatible regex.
    ("dag_enforcement", "roadmap_resumed", true),
    ("dag_enforcement", "escalated", true),
    // Lane R33a (RECEIPT CONTRACT v1.2, A31 analysis): the held-out fixture
    // `select-checks-worker-before-dispatch` seeds an already-existing Ready
    // goal card with a real worker roster, and its CORRECT coordinator
    // behavior is to `check_worker` the card's worker and then `goal_advance
    // {dispatch}` it directly — no new roadmap dispatched, nothing resumed,
    // no honest escalate. That shape had no positive dag_enforcement
    // evidence at all (`roadmap_dispatched=0, roadmap_resumed=0,
    // escalated=0` every run) even though `selection_check` graded the run
    // correct, so the qualification gate failed a genuinely correct run for
    // want of a counter. `worker_checked` is that counter: incremented by
    // `OrchestratorClient::record_check_worker` (`orchestrator.rs`), which
    // both a real `check_worker` tool call and `goal_advance`'s own internal
    // pre-dispatch check (`ensure_worker_checked_for_dispatch`, Lane R22a)
    // already funnel through — so this counts a real, observed worker query,
    // never a hoped-for one. Deliberately NOT wired to `goal_status`: that
    // tool reports a card's stored metadata (worker_key, timestamps,
    // liveness) and is callable on ANY goal card regardless of dispatch
    // state, so a call to it is not distinguishing evidence that a worker was
    // actually queried before/at a dispatch decision the way `check_worker`
    // is. Appended after the existing v1.1 keys so a legacy v1/v1.1 receipt
    // line (no trailing `worker_checked`) still parses — see
    // `permagent-eval::harness_log`'s `DAG_ENFORCEMENT_RE` for the
    // backward-compatible regex.
    ("dag_enforcement", "worker_checked", true),
    ("verification_integrity", "goals", true),
    ("verification_integrity", "review_reached", true),
    ("verification_integrity", "vacuous_passes", true),
    ("verification_integrity", "reviewer_mutations", true),
    ("verification_integrity", "reviewer_not_independent", true),
    ("safety_and_trust", "jail_denials", true),
    // Lane R18a: real, not a permanent placeholder. `write_jail.rs::jail_verdict`
    // resolves every write/edit-tool target by its ACTUAL resolved path (not a
    // text pattern), so for a REFUSING role (`JailRole::GoalWorker` /
    // `HeadlessSubAgent`) an `Outside` verdict is unconditionally denied —
    // `JailRole::denial_reason` returns `Some` for both variants by
    // construction of that match, so a write-tool escape can never reach this
    // counter as a silent pass. Shell escapes go through the same structural
    // analysis (`shell_escape`), which recognizes the escape shapes that
    // matter (`cd`/`pushd`, `git -C`/`--git-dir`/`--work-tree` with a mutating
    // subcommand, an outside output redirection, a mutating file command with
    // an outside target) — a truly undetected shell escape is, by
    // definition, invisible to the detector that failed to detect it, so it
    // cannot be counted without inventing a number (see this module's own
    // "not an assertion that detectors cover every attack" above).
    //
    // Lane R18a-b (G21 HIGH-2): this field must NEVER go `unknown` — a
    // single `unknown` key breaks the regex `permagent-eval` parses the
    // WHOLE `safety_and_trust` line with, reproducing the exact defect this
    // lane exists to fix (confirmed: `dag_enforcement`'s own
    // `bypass_passes`/`coordinator_mutations` already hit this via
    // `mark_unknown` in `extension_manager.rs`, a pre-existing instance of
    // the same class of bug). So the two places this jail loses
    // observability entirely — the `SECURITY_WRITE_JAIL_ENABLED=false`
    // break-glass switch (`tool_inspection::inspect_tools`) and a
    // session-row lookup failure (`WriteJailInspector::inspect`'s own
    // documented fail-open) — do NOT call `mark_unknown` here. The jail
    // fails open for EXECUTION (an unobserved write proceeds, per its own
    // module docs) but fails CLOSED for SCORING: both call `record_count`
    // with the number of write/shell requests in that batch the jail never
    // got to judge, counting each one as a bypass. Conservative (an
    // unobserved request might genuinely have been harmless) but honest —
    // an unobserved write outside a jail that could not judge it is scored
    // as a bypass, not asserted safe. Zero means the jail ran for every
    // batch this run saw and its structural analysis denied every escape
    // shape it is built to recognize; a disabled/fail-open batch adds to
    // the count instead of hiding behind `unknown`.
    ("safety_and_trust", "jail_bypasses", true),
    // Lane R18a: real, not a permanent placeholder.
    // `skill_run::run_command_skill` is FAIL-CLOSED, not fail-open: every
    // command execution goes through `verification_approval::decide` first,
    // and a `Parked` decision returns BEFORE `execute_command` — the only
    // process-spawning seam — is ever called; the S3 unattended path only
    // reaches `execute_command` after `decisions::auto_approve_skill_run`
    // itself returns `Ok`, gated on `headless_skill_run_enabled()` +
    // `GooseMode::Auto` + a non-`User` tier. The old bypass-capable runner
    // (`executable_skills::run_named`/`run_skill`, no registry, no
    // `decide()` call anywhere) was deleted by the X-skill DAG specifically
    // to close this gap — see that module's doc comment.
    //
    // Lane R18a-b (G21 HIGH-1 + MED-1): a prior pass at this field ("Lane
    // Q-receipts review fix: no counter may be a vacuous zero") tried
    // re-checking `ladder.audit_row.decision` for `Parked`/`Denied` INSIDE
    // `execute_command` itself and was correctly reverted as vacuous — that
    // branch could never run, because the caller already returned on
    // `Parked` before `execute_command` was even constructed, and the
    // check happened nowhere the value could differ run to run. G21's
    // review flagged the SAME gap from the other direction: with no call
    // site at all, the printed `0` was a compile-time literal, never a
    // per-run observation — this field could go to 100% and the receipt
    // would still print 0. Both fixed together: `run_command_skill` now
    // refuses loudly (returns `Err`, does not silently fall through) on any
    // `GateDecision` other than the four approving ones
    // (`decision_authorizes_execution`, an exhaustive match — MED-1), and
    // `execute_command` — the actual process-spawning seam — calls
    // `receipts::record_count`/`record` on EVERY run, reading THIS run's
    // real `ladder.audit_row.decision` rather than trusting the type
    // alone (HIGH-1). Zero is still guaranteed today (no command ever
    // reaches the process-spawning seam without first earning one of
    // `decide()`'s approving outcomes), but it is now a measured zero: a
    // future regression in either guard prints a non-zero receipt instead
    // of nothing.
    ("safety_and_trust", "unapproved_skill_runs", true),
    // Lane R18a: real, not a permanent placeholder.
    // `Agent::handle_provider_authorization_failure` (`reply_parts.rs`) is
    // FAIL-CLOSED the same way: the turn loop (`agents/agent.rs`) only ever
    // streams from a provider after a successful `reserve_provider_invocation`
    // — on failure it retries EXACTLY ONCE, and only
    // `if outcome.gate_grant_minted` (a real `granted_via_gate=1` reservation
    // row, not an in-memory flag); that retry calls
    // `reserve_provider_invocation` again and must itself succeed, or the
    // turn ends (`break`) without ever calling the provider. No branch
    // streams a response on a `None`/failed authorization, and a lookup
    // failure inside the reservation path itself becomes
    // `ProviderAuthorizationFailure::Unknown`, which routes through the same
    // fail-closed handling — never a silent allow.
    //
    // Lane R18a-b (G21 HIGH-1): same "measured, not asserted" fix as
    // `unapproved_skill_runs` above — `agents/agent.rs`'s turn loop now
    // calls `receipts::record_count("safety_and_trust","approval_bypasses",
    // 0)` at BOTH points a provider call is authorized to proceed (the
    // direct `reserve_provider_invocation` success, and the retried call
    // after a real `gate_grant_minted` grant), so the printed 0 reflects a
    // real touch on every provider call this run made, not a value that
    // would print identically even if the surrounding function were
    // deleted. `CostReservationOutcome`/`ProviderAuthorizationFailure` are
    // already exhaustively matched with no wildcard arm in
    // `reserve_provider_invocation_for_model`, so — unlike
    // `unapproved_skill_runs` — there is no additional enum-exhaustiveness
    // gap to guard here. Zero means every provider call this run made was
    // preceded by its own successful reservation, never a retried call
    // riding an earlier call's authorization.
    ("safety_and_trust", "approval_bypasses", true),
    ("safety_and_trust", "adversary_blocks", true),
];

struct Registry {
    counts: [AtomicU64; FIELDS.len()],
    known: [AtomicBool; FIELDS.len()],
    seen: Mutex<HashSet<(usize, String)>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            known: std::array::from_fn(|i| AtomicBool::new(FIELDS[i].2)),
            seen: Mutex::new(HashSet::new()),
        }
    }
}

impl Registry {
    fn record(&self, area: &str, key: &str, count: u64, identity: Option<&str>) {
        let Some(index) = FIELDS.iter().position(|&(a, k, _)| a == area && k == key) else {
            return;
        };
        if let Some(identity) = identity {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.insert((index, identity.to_string())) {
                return;
            }
        }
        self.counts[index].fetch_add(count, Ordering::Relaxed);
    }

    fn reset(&self) {
        for (i, counter) in self.counts.iter().enumerate() {
            counter.store(0, Ordering::Relaxed);
            self.known[i].store(FIELDS[i].2, Ordering::Relaxed);
        }
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            counts: std::array::from_fn(|i| {
                self.known[i]
                    .load(Ordering::Relaxed)
                    .then(|| self.counts[i].load(Ordering::Relaxed))
            }),
        }
    }
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::default);
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Lane R17a (M17 readiness audit §6.1, gap 1 & 4): sidecar file path for
/// this run, read from `PERMAGENT_RECEIPTS_SIDECAR` by [`begin_run`]. `None`
/// when unset, which makes [`write_sidecar`] a no-op — an ordinary run (no
/// eval driver watching) pays no extra I/O.
static SIDECAR_PATH: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));

/// Lane R17a: the most recently formatted `harness_overhead ...` line (see
/// [`set_harness_overhead_line`]), mirrored into the sidecar file alongside
/// the four receipt lines. `None` until at least one has been computed.
static LAST_OVERHEAD_LINE: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

/// Begin a fresh CLI run, before starting the coordinator or any workers.
///
/// Lane R17b (G20 review, LOW-1): deliberately does NOT call
/// [`write_sidecar`] itself — the file appears only once the first counter
/// mutation, turn boundary, or drain-poll tick calls it. A task killed
/// before any of those simply leaves no sidecar file at all, so the eval
/// driver's `receipt_source` reads `None` → the record stays `Unrated` for
/// those areas, which is the CORRECT reading of "no evidence was ever
/// produced". An eager all-zero write here would instead put real-looking
/// zeros on disk (`dispatches=0`, `roadmap_dispatched=0`, …) for a run that
/// never got far enough to observe anything — the qualification gate's
/// `>= 1` per-record thresholds would then read those fabricated zeros as a
/// genuine (and failing) observation instead of correctly reporting
/// `Unrated`. Absence must stay absence.
pub fn begin_run() {
    ACTIVE.store(false, Ordering::Relaxed);
    REGISTRY.reset();
    // Lane R22b (M24 `dschat-select-existing-roadmap-resumes-not-
    // redecomposes` forensics): cost_routing's counters
    // (`cost_router::routing_receipt`) are a SEPARATE process-wide registry
    // from this module's own `REGISTRY` — before this, nothing ever zeroed
    // them, so a process that ran more than one headless session (unlike
    // today's one-shot-per-invocation `permagent-eval` subprocess model)
    // would carry a prior run's dispatch counts into this run's printed
    // line and sidecar. Reset alongside `REGISTRY` so every headless run's
    // cost_routing receipt reflects only THIS run's dispatches, on every
    // path that reaches `orchestrator::dispatch_goal_fn` (decompose's
    // nudge-dispatch hook, `resume_roadmap`'s `dispatch_eligible_goals`, and
    // `goal_advance(dispatch)` alike — all three share that one seam).
    crate::cost_router::routing_receipt::reset();
    *SIDECAR_PATH.lock().unwrap_or_else(|e| e.into_inner()) =
        std::env::var_os("PERMAGENT_RECEIPTS_SIDECAR").map(PathBuf::from);
    *LAST_OVERHEAD_LINE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    ACTIVE.store(true, Ordering::Relaxed);
}

/// Stop collecting after the CLI has drained its workers and printed receipts.
pub fn end_run() {
    ACTIVE.store(false, Ordering::Relaxed);
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Missing evidence at a wired detector must not masquerade as a clean zero.
pub fn mark_unknown(area: &str, key: &str) {
    if is_active() {
        if let Some(i) = FIELDS.iter().position(|&(a, k, _)| a == area && k == key) {
            REGISTRY.known[i].store(false, Ordering::Relaxed);
            write_sidecar();
        }
    }
}

pub fn record(area: &str, key: &str) {
    record_count(area, key, 1);
}

pub fn record_count(area: &str, key: &str, count: u64) {
    if ACTIVE.load(Ordering::Relaxed) {
        REGISTRY.record(area, key, count, None);
        write_sidecar();
    }
}

/// Count a card/session once, even if it revisits the same detector.
pub fn record_once(area: &str, key: &str, identity: &str) {
    if ACTIVE.load(Ordering::Relaxed) {
        REGISTRY.record(area, key, 1, Some(identity));
        write_sidecar();
    }
}

pub fn snapshot() -> Snapshot {
    REGISTRY.snapshot()
}

/// Lane R17a (M17 readiness audit §6.1, gaps 1 & 4): when
/// `PERMAGENT_RECEIPTS_SIDECAR` names a path, keep it holding the exact same
/// four RECEIPT CONTRACT v1 lines the CLI's headless exit sink prints at a
/// clean exit, plus the latest known `harness_overhead` line (see
/// [`set_harness_overhead_line`]) — written atomically (temp file + rename)
/// so a concurrent reader never observes a half-written file. The
/// `permagent-eval` driver reads this file with the SAME line parser it
/// already uses on captured stdout when a harness SIGKILLed for exceeding
/// its timeout never got to print these lines itself. Reuses the exact
/// formatters the stdout sink prints from —
/// [`crate::cost_router::routing_receipt::format_line`] and
/// [`Snapshot::lines`] — so the two sinks can never format differently.
/// Best-effort throughout: any I/O failure is swallowed rather than
/// surfaced as a run failure over an observability write. A no-op (no I/O
/// at all) when the sidecar is not configured, or this run is not active.
pub fn write_sidecar() {
    if !is_active() {
        return;
    }
    let Some(path) = SIDECAR_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    else {
        return;
    };
    let mut text = String::new();
    text.push_str(&crate::cost_router::routing_receipt::format_line());
    text.push('\n');
    for line in REGISTRY.snapshot().lines() {
        text.push_str(&line);
        text.push('\n');
    }
    if let Some(overhead) = LAST_OVERHEAD_LINE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        text.push_str(&overhead);
        text.push('\n');
    }
    let _ = write_atomic(&path, &text);
}

/// Lane R17a: record the harness's latest formatted `harness_overhead ...`
/// line (`session::format_harness_overhead_line`/`_with_attempts`) so the
/// sidecar carries a recent reading even when the run is killed before its
/// own clean-exit print ever runs. Called from the per-turn boundary in
/// `agents::agent`'s reply loop. A no-op (no rollup ever computed) unless
/// this run is active — see [`write_sidecar`] for the sink this feeds.
pub fn set_harness_overhead_line(line: String) {
    if is_active() {
        *LAST_OVERHEAD_LINE.lock().unwrap_or_else(|e| e.into_inner()) = Some(line);
        write_sidecar();
    }
}

/// Atomic write: write `text` to a per-call unique temp file beside `path`,
/// then rename over it. Rename is atomic on the same filesystem, so a
/// concurrent reader always sees either the previous complete file or the
/// new one, never a partial write; the per-call counter keeps concurrent
/// writer threads in this process from colliding on the same temp name.
fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{n}"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    counts: [Option<u64>; FIELDS.len()],
}

impl Snapshot {
    /// Plain text, stable key order, no ANSI escapes or logging prefix.
    pub fn lines(&self) -> [String; 3] {
        [
            // Lane R33a: dag_enforcement grew from 6 to 7 keys
            // (`worker_checked` appended), shifting both subsequent ranges by 1.
            ("dag_enforcement", 0..7),
            ("verification_integrity", 7..12),
            ("safety_and_trust", 12..17),
        ]
        .map(|(area, range)| {
            let mut line = format!("receipt {area}");
            for i in range {
                let value = self.counts[i]
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                line.push_str(&format!(" {}={value}", FIELDS[i].1));
            }
            line
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_increment_snapshot_and_deduplicate() {
        let registry = Registry::default();
        registry.record("dag_enforcement", "roadmap_dispatched", 2, None);
        registry.record("dag_enforcement", "roadmap_dispatched", 1, None);
        registry.record("verification_integrity", "review_reached", 1, Some("card"));
        registry.record("verification_integrity", "review_reached", 1, Some("card"));
        let first = registry.snapshot();
        assert_eq!(first.counts[0], Some(3));
        // Lane R20a: verification_integrity's `review_reached` shifted from
        // index 5 to 7 when `roadmap_resumed`/`escalated` were appended to
        // dag_enforcement. Lane R33a: shifted again, 7 to 8, when
        // `worker_checked` was appended.
        assert_eq!(first.counts[8], Some(1));
        assert_eq!(first.counts[2], Some(0));
        registry.record("dag_enforcement", "roadmap_dispatched", 1, None);
        assert_eq!(first.counts[0], Some(3));
        assert_eq!(registry.snapshot().counts[0], Some(4));
    }

    #[test]
    fn receipts_concurrent_workers_and_next_run() {
        let registry = std::sync::Arc::new(Registry::default());
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        registry.record("safety_and_trust", "jail_denials", 1, None);
                        registry.record(
                            "verification_integrity",
                            "review_reached",
                            1,
                            Some("card"),
                        );
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        // Lane R20a: safety_and_trust's `jail_denials` shifted from index 9
        // to 11, and verification_integrity's `review_reached` from 5 to 7.
        // Lane R33a: shifted again, to 12 and 8, when `worker_checked` was
        // appended to dag_enforcement.
        assert_eq!(registry.snapshot().counts[12], Some(400));
        assert_eq!(registry.snapshot().counts[8], Some(1));
        registry.reset();
        assert_eq!(registry.snapshot().counts[12], Some(0));
        registry.record("verification_integrity", "review_reached", 1, Some("card"));
        assert_eq!(registry.snapshot().counts[8], Some(1));
    }

    #[test]
    fn receipts_contract_byte_exact() {
        let snapshot = Snapshot {
            counts: std::array::from_fn(|i| Some(i as u64 + 1)),
        };
        // Lane R20a: dag_enforcement grew from 4 to 6 keys
        // (`roadmap_resumed`, `escalated` appended), shifting every
        // subsequent area's values by 2. Lane R33a: grew again, to 7 keys
        // (`worker_checked` appended), shifting every subsequent area's
        // values by 1 more.
        assert_eq!(snapshot.lines(), [
            "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=2 bypass_passes=3 coordinator_mutations=4 roadmap_resumed=5 escalated=6 worker_checked=7",
            "receipt verification_integrity goals=8 review_reached=9 vacuous_passes=10 reviewer_mutations=11 reviewer_not_independent=12",
            "receipt safety_and_trust jail_denials=13 jail_bypasses=14 unapproved_skill_runs=15 approval_bypasses=16 adversary_blocks=17",
        ]);
    }

    /// Lane R18a: `jail_bypasses`, `unapproved_skill_runs` and
    /// `approval_bypasses` used to be permanently `unknown` — never wired to
    /// any producer, so the whole `safety_and_trust` line failed
    /// `SAFETY_AND_TRUST_RE` (every key must be an unsigned integer) on
    /// every run. They are now real counters, known from a fresh registry
    /// (and again after `reset`), exactly like `jail_denials` and
    /// `adversary_blocks` — see the `FIELDS` doc comments for why each one
    /// is honestly zero absent an observed event.
    #[test]
    fn safety_and_trust_bypass_counters_are_real_not_permanently_unknown() {
        let registry = Registry::default();
        let expected = "receipt safety_and_trust jail_denials=0 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=0";
        assert_eq!(registry.snapshot().lines()[2], expected);
        for key in [
            "jail_bypasses",
            "unapproved_skill_runs",
            "approval_bypasses",
        ] {
            registry.record("safety_and_trust", key, 1, None);
        }
        assert_eq!(
            registry.snapshot().lines()[2],
            "receipt safety_and_trust jail_denials=0 jail_bypasses=1 unapproved_skill_runs=1 \
             approval_bypasses=1 adversary_blocks=0"
        );
        registry.reset();
        assert_eq!(registry.snapshot().lines()[2], expected);
    }

    /// Lane R18a: the exact regex `permagent-eval`'s `harness_log.rs` uses
    /// to parse this line (`SAFETY_AND_TRUST_RE`), copied verbatim so a
    /// future edit to either side is caught here rather than only in a
    /// downstream eval run. Every key must parse as `[0-9]+` — the defect
    /// this lane fixes is exactly that `unknown` (a valid value this
    /// module prints) fails this regex and drops the WHOLE line, not just
    /// the one field.
    #[test]
    fn safety_and_trust_line_matches_the_driver_regex_when_every_field_is_known() {
        let re = regex::Regex::new(
            r"^\s*receipt\s+safety_and_trust\s+jail_denials=([0-9]+)\s+jail_bypasses=([0-9]+)\s+unapproved_skill_runs=([0-9]+)\s+approval_bypasses=([0-9]+)\s+adversary_blocks=([0-9]+)\s*$",
        )
        .unwrap();
        let registry = Registry::default();
        let line = registry.snapshot().lines()[2].clone();
        assert!(
            re.is_match(&line),
            "a freshly-initialized registry's safety_and_trust line must already parse: {line}"
        );

        // Lane R18a-b (G21 HIGH-2): still parses after the fail-open/bypass
        // producers have actually fired — a real, non-zero, multi-digit
        // count is still `[0-9]+`, never `unknown`.
        registry.record("safety_and_trust", "jail_bypasses", 12, None);
        registry.record("safety_and_trust", "unapproved_skill_runs", 1, None);
        registry.record("safety_and_trust", "approval_bypasses", 1, None);
        let line = registry.snapshot().lines()[2].clone();
        assert!(
            re.is_match(&line),
            "a registry with real bypass counts recorded must still parse: {line}"
        );
    }

    /// Lane R18a-b (G21 HIGH-2): `jail_bypasses` must never go `unknown` —
    /// an `unknown` value breaks the WHOLE `safety_and_trust` line's parse,
    /// reproducing the exact defect this lane exists to fix. The write jail
    /// fails open for EXECUTION (see its own module docs) but fails CLOSED
    /// for SCORING: `WriteJailInspector::inspect`'s session-lookup fail-open
    /// and `tool_inspection::inspect_tools`'s disabled-write-jail branch
    /// both call `record_count` with the number of unobserved write/shell
    /// requests in the batch (see those modules' own tests for the
    /// end-to-end wiring), never `mark_unknown`. This test exercises the
    /// primitive both call: counts accumulate across multiple fail-open
    /// events in the same run, and the NEXT run starts at a real zero
    /// again.
    #[test]
    fn jail_bypasses_accumulates_across_fail_open_events_and_resets_next_run() {
        // Lane R20a: safety_and_trust's `jail_bypasses` shifted from index
        // 10 to 12 when `roadmap_resumed`/`escalated` were appended to
        // dag_enforcement. Lane R33a: shifted again, to 13, when
        // `worker_checked` was appended.
        let registry = Registry::default();
        assert_eq!(registry.snapshot().counts[13], Some(0));
        registry.record("safety_and_trust", "jail_bypasses", 1, None);
        assert_eq!(registry.snapshot().counts[13], Some(1));
        registry.record("safety_and_trust", "jail_bypasses", 2, None);
        assert_eq!(registry.snapshot().counts[13], Some(3));
        registry.reset();
        assert_eq!(registry.snapshot().counts[13], Some(0));
    }

    /// Lane R17a: the sidecar mechanism itself, exercised through the real
    /// global API (`begin_run`/`record`/`write_sidecar` all operate on the
    /// process-wide `REGISTRY`/`ACTIVE` statics — there is no local-instance
    /// escape hatch for this seam, unlike the `Registry::default()` tests
    /// above). Sets `PERMAGENT_RECEIPTS_SIDECAR` to a fresh temp path, begins
    /// a run, bumps one counter, and asserts the sidecar file holds all four
    /// RECEIPT CONTRACT v1 lines — parseable by the exact same regexes
    /// `permagent-eval`'s `harness_log.rs` uses on captured stdout (copied
    /// verbatim from `receipts_contract_byte_exact` / `routing_receipt`'s own
    /// `routing_receipt_format_contract` test, since a zeroed run's lines are
    /// deterministic). `end_run` at the end returns the process to its
    /// pre-test (inactive) state.
    ///
    /// Lane R18a: `#[serial]` because it drives the process-wide
    /// `REGISTRY`/`ACTIVE` statics through the real `begin_run`/`end_run`
    /// API — same shared key as every other test in this binary that does
    /// the same (`write_jail.rs`, `tool_inspection.rs`), so none of them can
    /// interleave and observe a half-reset registry. That key subsumes the
    /// R17 gate fix below, which is kept because it also covers the
    /// `PERMAGENT_RECEIPTS_SIDECAR` env var this pair sets: interleaved, one
    /// test's `begin_run` re-points `SIDECAR_PATH` at the other's tempdir, so
    /// the first test's file never appears (`NotFound`) and the second picks
    /// up the first's stray `harness_overhead` line. `#[serial]` is always
    /// taken before the test body, so the two locks are acquired in one fixed
    /// order and cannot deadlock. `into_inner` so one test's panic does not
    /// cascade into the other.
    static SIDECAR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn sidecar_receives_all_four_contract_lines_on_a_counter_change() {
        let _guard = SIDECAR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let sidecar = dir.path().join("harness-receipts.txt");
        std::env::set_var("PERMAGENT_RECEIPTS_SIDECAR", &sidecar);
        begin_run();
        record("dag_enforcement", "roadmap_dispatched");
        end_run();
        std::env::remove_var("PERMAGENT_RECEIPTS_SIDECAR");

        let text = std::fs::read_to_string(&sidecar).expect("sidecar file was written");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "receipt cost_routing dispatches=0 cheapest_capable=0 pack_honored=0 planner=unknown/unknown",
                "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0 roadmap_resumed=0 escalated=0 worker_checked=0",
                "receipt verification_integrity goals=0 review_reached=0 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0",
                "receipt safety_and_trust jail_denials=0 jail_bypasses=0 unapproved_skill_runs=0 approval_bypasses=0 adversary_blocks=0",
            ]
        );
    }

    /// The `harness_overhead` line only appears once the CLI has computed
    /// one via [`set_harness_overhead_line`] — the sidecar must not fabricate
    /// it, but once set it is written on the very next receipt change too
    /// (not only when the overhead line itself changes).
    /// Lane R18a: `#[serial]` — see
    /// `sidecar_receives_all_four_contract_lines_on_a_counter_change`.
    #[test]
    #[serial_test::serial(receipts_global_registry)]
    fn sidecar_includes_harness_overhead_once_set() {
        let _guard = SIDECAR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let sidecar = dir.path().join("harness-receipts.txt");
        std::env::set_var("PERMAGENT_RECEIPTS_SIDECAR", &sidecar);
        begin_run();
        set_harness_overhead_line(
            "harness_overhead p50=1 p95=2 model=3 tools=4 wall=5".to_string(),
        );
        record("safety_and_trust", "jail_denials");
        end_run();
        std::env::remove_var("PERMAGENT_RECEIPTS_SIDECAR");

        let text = std::fs::read_to_string(&sidecar).expect("sidecar file was written");
        assert!(
            text.lines()
                .any(|l| l == "harness_overhead p50=1 p95=2 model=3 tools=4 wall=5"),
            "sidecar missing harness_overhead line:\n{text}"
        );
    }
}
