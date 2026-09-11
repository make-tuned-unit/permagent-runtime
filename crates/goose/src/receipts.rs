//! RECEIPT CONTRACT v1 counters for one headless CLI run and its in-process workers.
//!
//! Only the headless entry point enables collection. The CLI runs one headless
//! session at a time; concurrent independent runs in one process are unsupported.
//! Unknown sources stay unknown even when no event was observed. These are
//! detector counts, not an assertion that the detectors cover every attack.

use std::collections::HashSet;
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

/// Begin a fresh CLI run, before starting the coordinator or any workers.
pub fn begin_run() {
    ACTIVE.store(false, Ordering::Relaxed);
    REGISTRY.reset();
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
        }
    }
}

pub fn record(area: &str, key: &str) {
    record_count(area, key, 1);
}

pub fn record_count(area: &str, key: &str, count: u64) {
    if ACTIVE.load(Ordering::Relaxed) {
        REGISTRY.record(area, key, count, None);
    }
}

/// Count a card/session once, even if it revisits the same detector.
pub fn record_once(area: &str, key: &str, identity: &str) {
    if ACTIVE.load(Ordering::Relaxed) {
        REGISTRY.record(area, key, 1, Some(identity));
    }
}

pub fn snapshot() -> Snapshot {
    REGISTRY.snapshot()
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
                        registry.record("verification_integrity", "review_reached", 1, Some("card"));
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
        for key in ["jail_bypasses", "unapproved_skill_runs", "approval_bypasses"] {
            registry.record("safety_and_trust", key, 1, None);
        }
        assert_eq!(registry.snapshot().lines()[2], expected);
        registry.reset();
        assert_eq!(registry.snapshot().lines()[2], expected);
    }
}
