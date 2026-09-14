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

const FIELDS: [(&str, &str, bool); 14] = [
    ("dag_enforcement", "roadmap_dispatched", true),
    ("dag_enforcement", "bypass_attempts", true),
    ("dag_enforcement", "bypass_passes", true),
    ("dag_enforcement", "coordinator_mutations", true),
    ("verification_integrity", "goals", true),
    ("verification_integrity", "review_reached", true),
    ("verification_integrity", "vacuous_passes", true),
    ("verification_integrity", "reviewer_mutations", true),
    ("verification_integrity", "reviewer_not_independent", true),
    ("safety_and_trust", "jail_denials", true),
    // Landing sees dirty trunk state, but cannot attribute it to a worker.
    ("safety_and_trust", "jail_bypasses", false),
    // No independent source checks skill executions against durable approvals.
    // S3 commits an answered decision before execution; its runs are approved.
    ("safety_and_trust", "unapproved_skill_runs", false),
    // Tool execution does not carry answered-decision provenance.
    ("safety_and_trust", "approval_bypasses", false),
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
            ("dag_enforcement", 0..4),
            ("verification_integrity", 4..9),
            ("safety_and_trust", 9..14),
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
        assert_eq!(first.counts[5], Some(1));
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
        assert_eq!(registry.snapshot().counts[9], Some(400));
        assert_eq!(registry.snapshot().counts[5], Some(1));
        registry.reset();
        assert_eq!(registry.snapshot().counts[9], Some(0));
        registry.record("verification_integrity", "review_reached", 1, Some("card"));
        assert_eq!(registry.snapshot().counts[5], Some(1));
    }

    #[test]
    fn receipts_contract_byte_exact() {
        let snapshot = Snapshot {
            counts: std::array::from_fn(|i| Some(i as u64 + 1)),
        };
        assert_eq!(snapshot.lines(), [
            "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=2 bypass_passes=3 coordinator_mutations=4",
            "receipt verification_integrity goals=5 review_reached=6 vacuous_passes=7 reviewer_mutations=8 reviewer_not_independent=9",
            "receipt safety_and_trust jail_denials=10 jail_bypasses=11 unapproved_skill_runs=12 approval_bypasses=13 adversary_blocks=14",
        ]);
    }

    #[test]
    fn receipts_unwired_counters_are_unknown_even_if_recorded_or_reset() {
        let registry = Registry::default();
        let expected = "receipt safety_and_trust jail_denials=0 jail_bypasses=unknown unapproved_skill_runs=unknown approval_bypasses=unknown adversary_blocks=0";
        assert_eq!(registry.snapshot().lines()[2], expected);
        for key in [
            "jail_bypasses",
            "unapproved_skill_runs",
            "approval_bypasses",
        ] {
            registry.record("safety_and_trust", key, 1, None);
        }
        assert_eq!(registry.snapshot().lines()[2], expected);
        registry.reset();
        assert_eq!(registry.snapshot().lines()[2], expected);
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
    /// Lane R17 gate fix: `begin_run`/`record`/`write_sidecar` and the
    /// `PERMAGENT_RECEIPTS_SIDECAR` env var are all PROCESS-global, and libtest
    /// runs these two tests on different threads of one process. Interleaved,
    /// one test's `begin_run` re-points `SIDECAR_PATH` at the other's tempdir:
    /// the first test's file then never appears (`NotFound`) and the second's
    /// picks up the first's stray `harness_overhead` line. Serialize them —
    /// this is the only pair in the crate that drives the global run state.
    /// `into_inner` so one test's panic does not cascade into the other.
    static SIDECAR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
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
                "receipt dag_enforcement roadmap_dispatched=1 bypass_attempts=0 bypass_passes=0 coordinator_mutations=0",
                "receipt verification_integrity goals=0 review_reached=0 vacuous_passes=0 reviewer_mutations=0 reviewer_not_independent=0",
                "receipt safety_and_trust jail_denials=0 jail_bypasses=unknown unapproved_skill_runs=unknown approval_bypasses=unknown adversary_blocks=0",
            ]
        );
    }

    /// The `harness_overhead` line only appears once the CLI has computed
    /// one via [`set_harness_overhead_line`] — the sidecar must not fabricate
    /// it, but once set it is written on the very next receipt change too
    /// (not only when the overhead line itself changes).
    #[test]
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
