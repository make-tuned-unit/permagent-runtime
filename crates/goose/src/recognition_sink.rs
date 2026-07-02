//! RecognitionSink — Permagent's consumer boundary for Spectral's incoming
//! recognition subsystem (the third operation alongside recall and the graph,
//! Spectral branch `tier0-oracle-and-levers`).
//!
//! This module is the SEAM ONLY. Spectral's `Brain::recognize()` (query mode)
//! and the session-level stream tracker (edge-triggered routine locks) are not
//! in the pinned Spectral rev yet, so nothing here computes a verdict — the
//! call sites are wired and a debug-log sink is installed by default, so the
//! day the dep lands the only work is conversion + forwarding:
//!
//!   * query mode  → `brain_ops::inject_recall` calls [`observe_recall_stimulus`]
//!     alongside every recall; when `recognize()` exists its
//!     `RecognitionResult` is forwarded to [`RecognitionSink::on_verdict`] and
//!     persisted next to the recall's outcome row via
//!     [`crate::recognition::record_verdict`] (schema v22 columns).
//!   * stream mode → `activity::ingestion` calls [`observe_ambient_cue`] for
//!     every ambient memory actually ingested; when the tracker exists, cues
//!     feed it and its `StreamEvent`s are forwarded to
//!     [`RecognitionSink::on_routine_lock`] — the chime-in trigger
//!     (~0.1–0.2 events/day measured on the real brain).
//!
//! The types below deliberately MIRROR Spectral's shapes
//! (`spectral-recognition/src/lib.rs::Verdict` and `src/stream.rs::StreamEvent`)
//! rather than import them — the dep is not upgraded until the branch merges,
//! and its API may still shift. On upgrade, convert at this boundary; do not
//! leak spectral types past it.
//!
//! Consent: the ambient/stream path is opt-in per wing and per source via
//! [`crate::recognition_consent`] — cues for non-consented wings/sources never
//! reach the sink. The query-mode path is deliberately NOT consent-gated: like
//! the `recognition_events` substrate it sits beside, it is local-only
//! validation instrumentation for explicit recalls, not ambient observation.
//!
//! The whole module compiles only under the `spectral-recognition` feature;
//! call sites are `#[cfg]`-gated, so a default build has zero behavior change.

use std::sync::{Arc, OnceLock};
use tracing::debug;

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
// These are the two functions production code calls today. Both are cheap,
// synchronous, and infallible; neither blocks the reply or ingest paths.

/// Query-mode seam, called alongside every recall (`brain_ops::inject_recall`).
///
/// Today: a debug log proving the seam fires. When the Spectral dep lands:
/// run `brain.recognize(stimulus)` here (or immediately around this call),
/// forward the converted result to `sink().on_verdict(...)`, and persist it
/// with [`crate::recognition::record_verdict`] keyed on the recall's
/// `retrieval_id`.
pub fn observe_recall_stimulus(stimulus: &str, wing: Option<&str>, session_id: Option<&str>) {
    debug!(
        target: "permagent::recognition_sink",
        wing = wing.unwrap_or(""),
        session_id = session_id.unwrap_or(""),
        stimulus_len = stimulus.len(),
        "recall stimulus observed (verdict pending Spectral recognize())"
    );
}

/// Stream-mode seam, called for every ambient memory actually ingested
/// (`activity::ingestion::ingest_to_brain_blocking`, success branch).
///
/// The parameters mirror what Spectral's fixed-schema `Cue` is built from
/// (wing + content stats; day/hour and topic peaks are derived tracker-side).
/// Consent-gated: cues for wings the user has not opted in, or sources they
/// have excluded, are dropped HERE — they never reach the sink or (later) the
/// tracker. When the dep lands: build the `Cue`, feed the session tracker, and
/// forward emitted `StreamEvent`s to `sink().on_routine_lock(...)`.
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
        "ambient cue observed (tracker pending Spectral stream mode)"
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
    fn seams_are_infallible_and_cheap() {
        // No consent configured → ambient cue is dropped silently; recall
        // observation always fires. Neither panics.
        observe_recall_stimulus("how do I configure voice?", Some("permagent"), Some("s1"));
        observe_ambient_cue(Some("permagent"), "browser", "browser_navigated", 120);
        observe_ambient_cue(None, "terminal", "terminal_command_completed", 40);
    }
}
