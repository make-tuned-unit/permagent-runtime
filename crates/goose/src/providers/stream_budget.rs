//! A bound on how long a single provider response may stream, and how much it
//! may emit, before we stop consuming it.
//!
//! **Why this exists (Y8a, harness loop 2026-09-10, from Lane A14's M14
//! forensics).** Every SSE consumer in this crate was shaped
//! `while let Some(message) = message_stream.next().await { … }` with no
//! wall-clock and no output budget. The only limit on a single response was the
//! model's own `max_tokens` (logged as `131072` for the harness coordinator)
//! and, in practice, the lifetime of the process.
//!
//! On the M14 eval slice that produced a measured runaway. `glm47/fx04`:
//! request id `202609110713339146c69d4ac34b96` streamed from 23:13:39 to
//! 23:21:00 — **441 s, 12 960 delta events, 3.0 MB** — with the tail repeating
//! `"Good! Cart preview is now updated. Need to check admin and API next.\n\n"`
//! verbatim until the eval driver killed the process at its 600 s ceiling.
//! `glm47/fx06` did the same for 231 s / 7 596 events. Both left a `pending`
//! `cost_reservations` row and no ledger row: the tokens were really streamed
//! and will really be billed, so the recorded cost of those fixtures is a
//! floor, not a total.
//!
//! Three independent bounds, because a degenerate model can blow any one of
//! them without the others:
//!
//! 1. **Wall clock** — `PERMAGENT_PROVIDER_STREAM_MAX_SECS` (default 300,
//!    `0` = unlimited). Checked as a `tokio::time::timeout` on each `next()`
//!    with the *remaining* budget, so a stream that is merely slow between
//!    deltas is bounded by the same number as one that never ends.
//! 2. **Cumulative output** — `PERMAGENT_PROVIDER_STREAM_MAX_OUTPUT_CHARS`
//!    (default 400 000, `0` = unlimited). fx04's 3.0 MB would have tripped this
//!    at roughly a tenth of the wall it actually burned.
//! 3. **Consecutive repetition** — the same substantial delta text
//!    ([`REPEAT_MIN_CHARS`] or more) repeated [`REPEAT_LIMIT`] times in a row.
//!    This is the *cheap* detector for the exact observed failure and it fires
//!    in seconds rather than minutes. It is deliberately conservative: only
//!    byte-identical, back-to-back deltas count, so ordinary repeated
//!    punctuation or a genuinely repetitive table never reaches it.
//!
//! All three surface as [`ProviderError::stream_budget_exceeded`], which is
//! permanent and not retried — a retry would re-enter the same loop.

use std::time::{Duration, Instant};

use super::errors::ProviderError;
use crate::conversation::message::{Message, MessageContent};

/// Wall-clock ceiling for a single provider response, in seconds. `0` disables.
pub const MAX_SECS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_MAX_SECS";
/// Cumulative streamed-output ceiling for a single response, in characters.
/// `0` disables.
pub const MAX_OUTPUT_CHARS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_MAX_OUTPUT_CHARS";

pub const DEFAULT_MAX_SECS: u64 = 300;
pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 400_000;

/// A delta shorter than this is never counted as a repetition: single tokens,
/// whitespace and punctuation repeat legitimately all the time.
pub const REPEAT_MIN_CHARS: usize = 40;
/// Consecutive identical substantial deltas before we call it a loop. fx04
/// emitted its sentence thousands of times; 50 is far inside that and far
/// outside anything a working model produces.
pub const REPEAT_LIMIT: usize = 50;

/// The text a delta actually streamed — visible text AND reasoning text.
///
/// `Message::as_concat_text` sees only `Text` content, and the observed
/// runaway was a `reasoning: true` model. Counting only the visible half would
/// have let fx04 through untouched.
fn streamed_text(message: &Message) -> String {
    let mut out = String::new();
    for content in &message.content {
        match content {
            MessageContent::Text(text) => out.push_str(&text.text),
            MessageContent::Thinking(thinking) => out.push_str(&thinking.thinking),
            _ => {}
        }
    }
    out
}

fn env_u64(key: &str, default: u64) -> u64 {
    match std::env::var(key) {
        Ok(raw) => raw.trim().parse::<u64>().unwrap_or(default),
        Err(_) => default,
    }
}

#[derive(Debug)]
pub struct StreamBudget {
    started: Instant,
    /// `None` means unlimited.
    max_wall: Option<Duration>,
    /// `None` means unlimited.
    max_output_chars: Option<usize>,
    chars: usize,
    last_delta: String,
    repeats: usize,
}

impl StreamBudget {
    /// The budget every provider stream gets, from the environment.
    pub fn from_env() -> Self {
        let secs = env_u64(MAX_SECS_ENV, DEFAULT_MAX_SECS);
        let chars = env_u64(MAX_OUTPUT_CHARS_ENV, DEFAULT_MAX_OUTPUT_CHARS as u64) as usize;
        Self::new(
            (secs > 0).then(|| Duration::from_secs(secs)),
            (chars > 0).then_some(chars),
        )
    }

    pub fn new(max_wall: Option<Duration>, max_output_chars: Option<usize>) -> Self {
        Self {
            started: Instant::now(),
            max_wall,
            max_output_chars,
            chars: 0,
            last_delta: String::new(),
            repeats: 0,
        }
    }

    /// How long the next `next()` may block. `None` = unlimited (no timeout
    /// wrapper at all); `Some(ZERO)` = the budget is already spent.
    pub fn remaining(&self) -> Option<Duration> {
        self.max_wall
            .map(|max| max.saturating_sub(self.started.elapsed()))
    }

    fn observed_rate(&self) -> String {
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed <= 0.0 {
            return format!("{} chars", self.chars);
        }
        format!(
            "{} chars in {:.0} s ({:.0} chars/s)",
            self.chars,
            elapsed,
            self.chars as f64 / elapsed
        )
    }

    /// The error for a stream that ran past its wall-clock budget.
    pub fn wall_clock_error(&self) -> ProviderError {
        ProviderError::stream_budget_exceeded(format!(
            "the response streamed for longer than {} allows ({}s); {} were received before it was \
             stopped. Raise or disable {} (0 = unlimited) if this model legitimately needs longer.",
            MAX_SECS_ENV,
            self.max_wall.map(|d| d.as_secs()).unwrap_or(0),
            self.observed_rate(),
            MAX_SECS_ENV,
        ))
    }

    fn output_error(&self) -> ProviderError {
        ProviderError::stream_budget_exceeded(format!(
            "the response emitted more than {} allows ({} chars); {} were received before it was \
             stopped. Raise or disable {} (0 = unlimited) if this model legitimately needs more.",
            MAX_OUTPUT_CHARS_ENV,
            self.max_output_chars.unwrap_or(0),
            self.observed_rate(),
            MAX_OUTPUT_CHARS_ENV,
        ))
    }

    fn repetition_error(&self) -> ProviderError {
        ProviderError::stream_budget_exceeded(format!(
            "the model repeated the same {}-char delta {} times in a row — it is looping, not \
             answering; {} were received before it was stopped",
            self.last_delta.chars().count(),
            self.repeats,
            self.observed_rate(),
        ))
    }

    /// Account for one streamed delta. `Err` means: stop consuming this stream.
    ///
    /// Takes an `Option` because that is exactly what a `MessageStream` item
    /// carries: a usage-only chunk has no message, and it must still advance
    /// the wall clock rather than being skipped.
    pub fn record(&mut self, message: Option<&Message>) -> Result<(), ProviderError> {
        match message {
            Some(message) => self.record_text(&streamed_text(message)),
            None => self.record_text(""),
        }
    }

    /// The half of [`record`](Self::record) that does not need a `Message`.
    pub fn record_text(&mut self, delta: &str) -> Result<(), ProviderError> {
        self.chars = self.chars.saturating_add(delta.chars().count());

        if delta.chars().count() >= REPEAT_MIN_CHARS {
            if delta == self.last_delta {
                self.repeats = self.repeats.saturating_add(1);
                if self.repeats >= REPEAT_LIMIT {
                    return Err(self.repetition_error());
                }
            } else {
                self.last_delta.clear();
                self.last_delta.push_str(delta);
                self.repeats = 1;
            }
        }

        if let Some(max) = self.max_output_chars {
            if self.chars > max {
                return Err(self.output_error());
            }
        }

        // The wall clock is enforced by the `timeout` around `next()`, but a
        // stream that delivers deltas faster than the budget can never be
        // *interrupted* by that timeout — check it here too so a fast runaway
        // is bounded by the same number as a stalled one.
        if let Some(remaining) = self.remaining() {
            if remaining.is_zero() {
                return Err(self.wall_clock_error());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_delta(text: &str) -> Message {
        Message::assistant().with_text(text)
    }

    /// The fx04 shape, in miniature: the same substantial sentence over and
    /// over. Before Y8a this loop had no exit but the process dying.
    #[test]
    fn a_repeating_delta_trips_the_repetition_detector() {
        let sentence = "Good! Cart preview is now updated. Need to check admin and API next.\n\n";
        assert!(sentence.chars().count() >= REPEAT_MIN_CHARS);
        let mut budget = StreamBudget::new(None, None);

        for i in 1..REPEAT_LIMIT {
            budget
                .record(Some(&text_delta(sentence)))
                .unwrap_or_else(|e| panic!("delta {i} must still be accepted: {e}"));
        }
        let err = budget
            .record(Some(&text_delta(sentence)))
            .expect_err("the 50th consecutive identical delta is a loop");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.is_permanent(), "a looping stream must not be retried");
        assert!(err.to_string().contains("looping"), "{err}");
    }

    /// A short delta repeating is ordinary streaming, not a loop.
    #[test]
    fn a_short_delta_repeating_is_never_a_loop() {
        let mut budget = StreamBudget::new(None, None);
        for _ in 0..(REPEAT_LIMIT * 10) {
            budget.record(Some(&text_delta(" the"))).expect("ordinary tokens");
        }
    }

    /// Distinct deltas reset the run, so a model that alternates is not
    /// mistaken for one that is stuck.
    #[test]
    fn alternating_deltas_reset_the_repetition_run() {
        let a = "A".repeat(REPEAT_MIN_CHARS + 1);
        let b = "B".repeat(REPEAT_MIN_CHARS + 1);
        let mut budget = StreamBudget::new(None, None);
        for _ in 0..(REPEAT_LIMIT * 4) {
            budget.record(Some(&text_delta(&a))).expect("alternating a");
            budget.record(Some(&text_delta(&b))).expect("alternating b");
        }
    }

    #[test]
    fn the_output_budget_stops_a_stream_that_never_ends() {
        let mut budget = StreamBudget::new(None, Some(100));
        // Deliberately varied text so the repetition detector is NOT what fires:
        // this pins the cumulative bound on its own.
        let mut failure = None;
        for i in 0..40 {
            if let Err(e) = budget.record(Some(&text_delta(&format!("chunk {i} ")))) {
                failure = Some(e);
                break;
            }
        }
        let err = failure.expect("the cumulative output budget must bite");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.to_string().contains(MAX_OUTPUT_CHARS_ENV), "{err}");
        assert!(!err.to_string().contains("looping"), "{err}");
    }

    /// Reasoning text counts. The observed runaway was a `reasoning: true`
    /// model, and `Message::as_concat_text` does not see thinking content.
    #[test]
    fn reasoning_text_counts_towards_the_output_budget() {
        let thinking = Message::assistant().with_thinking("z".repeat(200), "sig");
        assert!(
            thinking.as_concat_text().is_empty(),
            "precondition: the visible-text view is blind to this delta"
        );
        let mut budget = StreamBudget::new(None, Some(100));
        let err = budget
            .record(Some(&thinking))
            .expect_err("thinking content must be counted");
        assert!(err.is_stream_budget_exceeded(), "{err}");
    }

    #[test]
    fn a_zero_budget_means_unlimited() {
        let budget = StreamBudget::new(None, None);
        assert!(budget.remaining().is_none(), "no wall clock, no timeout");
    }

    #[test]
    fn a_spent_wall_clock_budget_is_reported_as_zero_remaining() {
        let budget = StreamBudget::new(Some(Duration::ZERO), None);
        assert_eq!(budget.remaining(), Some(Duration::ZERO));
        let err = budget.wall_clock_error();
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.to_string().contains(MAX_SECS_ENV), "{err}");
    }
}
