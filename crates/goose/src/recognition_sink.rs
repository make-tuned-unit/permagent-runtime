//! RecognitionSink — Permagent's consumer boundary for Spectral's recognition
//! subsystem (the third operation alongside recall and the graph).
//!
//! ## Query mode is LIVE
//!
//! [`observe_recall_stimulus`] runs Spectral's `Brain::recognize()` alongside
//! every recall, converts its `RecognitionResult` to Permagent's
//! [`RecognitionVerdict`] at this boundary, forwards it to
//! [`RecognitionSink::on_verdict`], and persists verdict + familiarity next to
//! that recall's outcome row (`recognition_events`, schema v22 columns) via
//! [`crate::recognition::VerdictWriteHandle`].
//!
//! **Read this before writing another "the dependency has not landed"
//! comment.** Until 2026-08-19 this docstring asserted that
//! `Brain::recognize()` had not yet reached the Spectral revision this repo
//! pins, so nothing was ever wired. The assertion had been false since before
//! that revision was chosen:
//!
//!   * `recognize()` landed on Spectral's facade on **2026-07-12**
//!     (`f1692f0`, `crates/spectral/src/lib.rs`).
//!   * `StreamEvent` / `StreamTracker` / `StreamTracker::observe()` landed on
//!     **2026-07-03** (`095f234`, `spectral-recognition/src/stream.rs`).
//!   * This repo's pin, `spectral rev c2c8381`, is dated **2026-07-31** —
//!     nineteen and twenty-eight days LATER respectively. Neither is behind a
//!     Spectral feature (its only features are `http-llm`, `neural-bench`,
//!     `spectrogram-legacy`).
//!
//! `crates/goose/Cargo.toml` said so all along ("Enabling requires NO Spectral
//! dep upgrade"); the module docstring is what governed behaviour, and a
//! subsystem measuring 0.9946 AUC with zero false-Novel therefore never
//! answered a question in production. `crate::dependency_claim_guard` now
//! fails the build for a comment of this shape whose named capability is in
//! fact reachable at the pin.
//!
//! ## Stream mode is NOT wired, and the reason is not the dependency
//!
//! [`observe_ambient_cue`] still only logs. `StreamTracker::observe()` is
//! present and callable at the pin; what is missing is entirely Permagent-side
//! and is a design decision, not conversion + forwarding:
//!
//!   * `StreamTracker` is edge-triggered against ENROLLED `Segment`s, and
//!     nothing in Permagent mines, stores, or versions segments;
//!   * the tracker is stateful and mutable, so it needs an owner with a
//!     defined lifetime (per session? per wing? across a daemon restart?);
//!   * `LockAcquired` is specified here as "the chime-in trigger", and what
//!     the agent actually does on a chime-in is an unwritten product contract.
//!
//! Wiring it would mean inventing all three. Left deliberately undone.
//!
//! ## A returned `memory_id` is UNRESOLVED until you prove otherwise
//!
//! Spectral keeps recognition state in a **separate database file** —
//! `~/.permagent/brain/recognition.db`, not `memory.db`. SQLite foreign keys
//! cannot cross files, so `PRAGMA foreign_keys = ON` and every cascade built
//! on it stop at the file boundary: a raw `DELETE FROM memories` removes the
//! memory and leaves its ~24 recognition rows behind. Those orphaned
//! enrolments still score, and can out-rank live memories.
//!
//! This is real, not hypothetical: the live brain carries 2,841 enrolments
//! against 2,839 memories, two orphans left by a Librarian pruning pass. The
//! raw-delete sites (`routes/librarian/pruning.rs`, `activity::cleanup`) are
//! being moved to `SafeBrain::forget()` separately. Spectral has fixed its own
//! half on a branch; rev `c2c8381` — the revision this code actually calls —
//! predates that fix, so the consumer has to be safe on its own.
//!
//! So: this module never forwards a `Recognized { memory_id }` it has not
//! resolved. [`resolved_verdict`] degrades an unresolvable id to `Familiar` —
//! the weaker verdict the evidence still supports — and reports the orphan so
//! the leak is visible instead of silent. A consumer of Spectral recognition
//! must treat every returned id as unresolved until a `get_memory` says
//! otherwise.
//!
//! ## Why the types below mirror rather than re-export
//!
//! [`RecognitionVerdict`] and [`RoutineLockEvent`] mirror
//! `spectral_recognition::Verdict` and `stream::StreamEvent`. This is a
//! boundary, not a shortcut: Permagent persists these as stable strings in its
//! own schema, and a Spectral shape change must break exactly one conversion
//! function ([`RecognitionVerdict::from_spectral`]) rather than every sink
//! implementor. Convert here; do not leak spectral types past this module.
//!
//! ## Consent and cost
//!
//! The ambient/stream path is opt-in per wing and per source via
//! [`crate::recognition_consent`] — cues for non-consented wings/sources never
//! reach the sink. The query-mode path is deliberately NOT consent-gated: like
//! the `recognition_events` substrate it sits beside, it is local-only
//! validation instrumentation for explicit recalls, not ambient observation.
//!
//! Recognition is an OBSERVER. `recognize()` costs median 12.9 ms / p90 73 ms /
//! p99 86 ms through the production SQLite path on the 2,818-memory brain, and
//! it runs on a detached task under a hard timeout: a recall returns whether
//! recognition succeeds, fails, or hangs.
//!
//! The whole module compiles only under the `spectral-recognition` feature;
//! call sites are `#[cfg]`-gated, so a feature-off build has zero behavior
//! change.

use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, warn};

/// Hard ceiling on one `recognize()` call, enforced by the caller's detached
/// task. Measured p99 is 86 ms on a 2,818-memory brain; this is ~23x that, so
/// it fires only for a genuinely wedged brain (a poisoned engine mutex, a
/// locked `recognition.db`) and not for a slow one. On expiry the observation
/// is dropped: no verdict is a fine outcome, an accumulating pile of blocked
/// tasks is not.
const RECOGNIZE_BUDGET: Duration = Duration::from_secs(2);

/// Mirror of Spectral's `Verdict` (query mode).
#[derive(Debug, Clone, PartialEq)]
pub enum RecognitionVerdict {
    /// A specific stored trace was recognized.
    Recognized { memory_id: String },
    /// Familiar in aggregate, no single trace dominates.
    Familiar,
    /// Never seen before.
    Novel,
}

impl RecognitionVerdict {
    /// Convert Spectral's verdict into Permagent's. **The** boundary: this is
    /// the only place in the tree that names `spectral::Verdict`, so a shape
    /// change upstream fails here and nowhere else.
    pub fn from_spectral(verdict: &spectral::Verdict) -> Self {
        match verdict {
            spectral::Verdict::Recognized { memory_id } => RecognitionVerdict::Recognized {
                memory_id: memory_id.clone(),
            },
            spectral::Verdict::Familiar => RecognitionVerdict::Familiar,
            spectral::Verdict::Novel => RecognitionVerdict::Novel,
        }
    }

    /// Canonical lowercase label — the value stored in
    /// `recognition_events.recognition_verdict` (schema v22).
    pub fn as_str(&self) -> &'static str {
        match self {
            RecognitionVerdict::Recognized { .. } => "recognized",
            RecognitionVerdict::Familiar => "familiar",
            RecognitionVerdict::Novel => "novel",
        }
    }
}

/// What resolving a `Recognized` verdict's `memory_id` against the memory
/// store produced. See this module's header for why a lookup is mandatory.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryResolution {
    /// The memory row exists — the id may be named.
    Live,
    /// The recognition index knows the id; the memory store does not. An
    /// orphaned enrolment that outlived its memory.
    Orphaned,
    /// The lookup itself failed, so nothing is known. Treated exactly like
    /// `Orphaned` for naming purposes: unproven is unproven.
    Unresolved(String),
}

/// Convert Spectral's verdict at the boundary, **refusing to name a memory
/// that does not resolve**.
///
/// `Recognized { id }` with a non-`Live` resolution degrades to `Familiar` —
/// the evidence for "this stimulus is a re-encounter" is unaffected by the
/// candidate having been deleted; only the claim about *which* trace is lost.
/// Dropping the verdict entirely would throw away a true signal, and
/// forwarding an unfetchable id would hand every downstream consumer a
/// dangling pointer.
///
/// Returns the safe verdict plus, when an id had to be withheld, that id — so
/// the caller can report a data-integrity event and a test can assert one
/// happened. The warning is emitted here too, so it cannot be forgotten.
pub fn resolved_verdict(
    verdict: &spectral::Verdict,
    resolution: MemoryResolution,
) -> (RecognitionVerdict, Option<String>) {
    let spectral::Verdict::Recognized { memory_id } = verdict else {
        return (RecognitionVerdict::from_spectral(verdict), None);
    };
    match resolution {
        MemoryResolution::Live => (
            RecognitionVerdict::Recognized {
                memory_id: memory_id.clone(),
            },
            None,
        ),
        MemoryResolution::Orphaned => {
            warn!(
                target: "permagent::recognition_sink",
                memory_id = %memory_id,
                "recognition candidate outlived its memory: recognition.db still has an \
                 enrolment for a memory row that no longer exists, so the verdict is \
                 degraded Recognized -> Familiar. A raw DELETE bypassed SafeBrain::forget()."
            );
            (RecognitionVerdict::Familiar, Some(memory_id.clone()))
        }
        MemoryResolution::Unresolved(why) => {
            warn!(
                target: "permagent::recognition_sink",
                memory_id = %memory_id,
                error = %why,
                "could not resolve a recognized memory id; degrading Recognized -> Familiar \
                 rather than naming a memory that may not exist"
            );
            (RecognitionVerdict::Familiar, Some(memory_id.clone()))
        }
    }
}

/// A query-mode verdict observation, delivered alongside a recall.
#[derive(Debug, Clone)]
pub struct VerdictObservation {
    pub verdict: RecognitionVerdict,
    /// Corpus-level familiarity in [0, 1] (mirrors `RecognitionResult.familiarity`).
    pub familiarity: f64,
    /// The stimulus text the verdict was computed over.
    pub stimulus: String,
    /// Wing focus at observation time, when known.
    pub wing: Option<String>,
    pub session_id: Option<String>,
    /// Join key into `recognition_events` when the observation rides a recall.
    pub retrieval_id: Option<String>,
}

/// Mirror of Spectral's edge-triggered `StreamEvent` (stream mode). A
/// continuing lock emits nothing; `LockAcquired` means "the user has started a
/// recognized routine" — the chime-in trigger.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutineLockEvent {
    LockAcquired {
        segment_id: String,
        offset: usize,
        score: f64,
    },
    LockLost {
        segment_id: String,
        /// Offset at which the pattern diverged — "your routine broke here".
        at_offset: usize,
    },
    LockTransferred {
        from: String,
        to: String,
        score: f64,
    },
}

/// The consumer trait. All methods default to no-ops so implementors opt into
/// exactly the signals they consume.
pub trait RecognitionSink: Send + Sync {
    fn on_verdict(&self, _observation: &VerdictObservation) {}
    fn on_routine_lock(&self, _event: &RoutineLockEvent) {}
}

/// Default sink: logs every signal at debug level and does nothing else.
pub struct DebugLogSink;

impl RecognitionSink for DebugLogSink {
    fn on_verdict(&self, observation: &VerdictObservation) {
        debug!(
            target: "permagent::recognition_sink",
            verdict = observation.verdict.as_str(),
            familiarity = observation.familiarity,
            wing = observation.wing.as_deref().unwrap_or(""),
            "recognition verdict observed"
        );
    }

    fn on_routine_lock(&self, event: &RoutineLockEvent) {
        debug!(
            target: "permagent::recognition_sink",
            ?event,
            "routine lock event"
        );
    }
}

static SINK: OnceLock<Arc<dyn RecognitionSink>> = OnceLock::new();

/// Install a sink. First caller wins (returns false if one is already
/// installed) — mirrors the decision-sink precedent.
pub fn install_recognition_sink(sink: Arc<dyn RecognitionSink>) -> bool {
    SINK.set(sink).is_ok()
}

/// The active sink; [`DebugLogSink`] until one is installed.
pub fn sink() -> Arc<dyn RecognitionSink> {
    SINK.get_or_init(|| Arc::new(DebugLogSink)).clone()
}

// ── Call-site seams ──────────────────────────────────────────────────────
//
// These are the two functions production code calls today. Both return
// immediately and are infallible; neither blocks the reply or ingest paths.

/// Query-mode seam, called alongside every recall (`brain_ops::inject_recall`).
///
/// Runs `SafeBrain::recognize(stimulus)` on a DETACHED task, resolves any
/// named memory id against the store (see the module header — recognition
/// state lives in its own database file, so a recognized id can outlive its
/// memory), converts the result at this boundary, hands it to
/// `sink().on_verdict(...)`, and — when the recall minted a
/// `recognition_events` row — records verdict + familiarity on it through
/// `verdict_write`.
///
/// **Recognition is an observer and must never be able to fail a recall.**
/// This function returns before `recognize()` starts. Every downstream outcome
/// (Spectral error, panicked blocking task, expiry of [`RECOGNIZE_BUDGET`],
/// a `recognition_events` INSERT that never lands) ends in a `warn!` and a
/// dropped observation. The recall it rides along with has already returned.
///
/// `verdict_write` is `None` when the caller had no recognition pool (recall
/// instrumentation disabled): the verdict still reaches the sink, it is simply
/// not persisted, because there is no row to hang it on.
pub fn observe_recall_stimulus(
    brain: &crate::brain_handle::SafeBrain,
    verdict_write: Option<crate::recognition::VerdictWriteHandle>,
    stimulus: &str,
    wing: Option<&str>,
    session_id: Option<&str>,
) {
    if stimulus.trim().is_empty() {
        return;
    }
    let brain = brain.clone();
    let stimulus = stimulus.to_string();
    let wing = wing.map(str::to_string);
    let session_id = session_id.map(str::to_string);
    let retrieval_id = verdict_write
        .as_ref()
        .map(|handle| handle.retrieval_id().to_string());

    tokio::spawn(async move {
        // ONE budget over recognize() AND the memory resolution it forces:
        // both are Brain calls, and it is the total that must not accumulate.
        let recognized = tokio::time::timeout(RECOGNIZE_BUDGET, async {
            let result = brain.recognize(&stimulus).await?;
            let resolution = match &result.verdict {
                spectral::Verdict::Recognized { memory_id } => {
                    match brain.get_memory(memory_id).await {
                        Ok(Some(_)) => MemoryResolution::Live,
                        Ok(None) => MemoryResolution::Orphaned,
                        Err(e) => MemoryResolution::Unresolved(e.to_string()),
                    }
                }
                // Familiar and Novel name no memory, so nothing to resolve.
                _ => MemoryResolution::Live,
            };
            anyhow::Ok((result, resolution))
        })
        .await;

        let (result, resolution) = match recognized {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                warn!(
                    target: "permagent::recognition_sink",
                    error = %e,
                    "recognize() failed — recall unaffected, verdict dropped"
                );
                return;
            }
            Err(_) => {
                warn!(
                    target: "permagent::recognition_sink",
                    budget_ms = RECOGNIZE_BUDGET.as_millis(),
                    "recognize() exceeded its budget — recall unaffected, verdict dropped"
                );
                return;
            }
        };

        let (verdict, withheld) = resolved_verdict(&result.verdict, resolution);
        let observation = VerdictObservation {
            verdict,
            familiarity: result.familiarity,
            stimulus,
            wing,
            session_id,
            retrieval_id,
        };
        debug!(
            target: "permagent::recognition_sink",
            verdict = observation.verdict.as_str(),
            familiarity = observation.familiarity,
            novelty = result.novelty,
            traces = result.traces.len(),
            withheld_memory_id = withheld.as_deref().unwrap_or(""),
            "recall stimulus recognized"
        );
        sink().on_verdict(&observation);

        if let Some(handle) = verdict_write {
            handle
                .record(observation.verdict.as_str(), observation.familiarity)
                .await;
        }
    });
}

/// Stream-mode seam, called for every ambient memory actually ingested
/// (`activity::ingestion::ingest_to_brain_blocking`, success branch).
///
/// The parameters mirror what Spectral's fixed-schema `Cue` is built from
/// (wing + content stats; day/hour and topic peaks are derived tracker-side).
/// Consent-gated: cues for wings the user has not opted in, or sources they
/// have excluded, are dropped HERE — they never reach the sink or (later) the
/// tracker.
///
/// Still a log, and NOT because the dependency is missing: Spectral's
/// `StreamTracker::observe()` is callable at the pin. Feeding it needs three
/// Permagent-side decisions that do not exist yet — where enrolled `Segment`s
/// come from, who owns the mutable tracker and for how long, and what a
/// `LockAcquired` chime-in actually does. See this module's header.
pub fn observe_ambient_cue(
    wing: Option<&str>,
    source_surface: &str,
    event_type: &str,
    content_len: usize,
) {
    if !crate::recognition_consent::ambient_cue_allowed(wing, source_surface) {
        return;
    }
    debug!(
        target: "permagent::recognition_sink",
        wing = wing.unwrap_or(""),
        source = source_surface,
        event_type,
        content_len,
        "ambient cue observed (tracker not owned by Permagent; see module docs)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSink(AtomicUsize, AtomicUsize);
    impl RecognitionSink for CountingSink {
        fn on_verdict(&self, _o: &VerdictObservation) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn on_routine_lock(&self, _e: &RoutineLockEvent) {
            self.1.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn verdict_labels_are_canonical() {
        assert_eq!(
            RecognitionVerdict::Recognized {
                memory_id: "m".into()
            }
            .as_str(),
            "recognized"
        );
        assert_eq!(RecognitionVerdict::Familiar.as_str(), "familiar");
        assert_eq!(RecognitionVerdict::Novel.as_str(), "novel");
    }

    #[test]
    fn default_sink_is_debug_log_and_install_is_first_wins() {
        // sink() must never panic and must hand back something usable.
        let s = sink();
        s.on_verdict(&VerdictObservation {
            verdict: RecognitionVerdict::Novel,
            familiarity: 0.0,
            stimulus: "q".into(),
            wing: None,
            session_id: None,
            retrieval_id: None,
        });
        s.on_routine_lock(&RoutineLockEvent::LockLost {
            segment_id: "seg".into(),
            at_offset: 3,
        });
        // Once the default is materialized, install loses (first wins).
        assert!(!install_recognition_sink(Arc::new(CountingSink(
            AtomicUsize::new(0),
            AtomicUsize::new(0)
        ))));
    }

    #[test]
    fn ambient_cue_seam_is_infallible_and_cheap() {
        // No consent configured → ambient cue is dropped silently, and neither
        // shape panics. (The query-mode seam needs a real Brain and a runtime,
        // so it is exercised by the daemon suite, not here.)
        observe_ambient_cue(Some("permagent"), "browser", "browser_navigated", 120);
        observe_ambient_cue(None, "terminal", "terminal_command_completed", 40);
    }

    /// **Regression, orphaned enrolment.** recognition.db is a separate file
    /// from memory.db, so no foreign key reaches it and a raw
    /// `DELETE FROM memories` leaves the enrolment behind. Spectral fixed its
    /// own half on a branch that rev c2c8381 predates, so the consumer has to
    /// be safe on its own. The orphan condition is constructed directly here
    /// rather than by replaying a pruning pass: what matters is the contract,
    /// not how the id came to dangle.
    #[test]
    fn an_unresolvable_memory_id_is_never_named_in_a_verdict() {
        let dangling = spectral::Verdict::Recognized {
            memory_id: "mem-pruned-last-night".into(),
        };

        // Orphaned: degrade to Familiar, never carry the id, and report it.
        let (verdict, withheld) = resolved_verdict(&dangling, MemoryResolution::Orphaned);
        assert_eq!(verdict, RecognitionVerdict::Familiar);
        assert_eq!(withheld.as_deref(), Some("mem-pruned-last-night"));
        assert_eq!(verdict.as_str(), "familiar");

        // A failed lookup proves nothing, so it is treated the same way.
        let (verdict, withheld) = resolved_verdict(
            &dangling,
            MemoryResolution::Unresolved("database is locked".into()),
        );
        assert_eq!(verdict, RecognitionVerdict::Familiar);
        assert_eq!(withheld.as_deref(), Some("mem-pruned-last-night"));

        // And the happy path still names the memory.
        let (verdict, withheld) = resolved_verdict(&dangling, MemoryResolution::Live);
        assert_eq!(
            verdict,
            RecognitionVerdict::Recognized {
                memory_id: "mem-pruned-last-night".into()
            }
        );
        assert_eq!(withheld, None);
    }

    /// Familiar and Novel name no memory, so resolution is not consulted and
    /// nothing is ever withheld.
    #[test]
    fn verdicts_that_name_no_memory_pass_through_unchanged() {
        for verdict in [spectral::Verdict::Familiar, spectral::Verdict::Novel] {
            let (converted, withheld) = resolved_verdict(&verdict, MemoryResolution::Live);
            assert_eq!(converted, RecognitionVerdict::from_spectral(&verdict));
            assert_eq!(withheld, None);
        }
    }

    /// The whole point of mirroring rather than re-exporting: exactly one
    /// conversion, and it must cover every Spectral variant. If Spectral grows
    /// a fourth verdict this stops compiling here — which is the design.
    #[test]
    fn spectral_verdicts_convert_at_the_boundary() {
        assert_eq!(
            RecognitionVerdict::from_spectral(&spectral::Verdict::Recognized {
                memory_id: "mem-1".into()
            }),
            RecognitionVerdict::Recognized {
                memory_id: "mem-1".into()
            }
        );
        assert_eq!(
            RecognitionVerdict::from_spectral(&spectral::Verdict::Familiar),
            RecognitionVerdict::Familiar
        );
        assert_eq!(
            RecognitionVerdict::from_spectral(&spectral::Verdict::Novel),
            RecognitionVerdict::Novel
        );
    }
}
