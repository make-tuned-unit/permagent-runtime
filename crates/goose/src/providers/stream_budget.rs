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
//! Four independent bounds, because a degenerate model can blow any one of
//! them without the others:
//!
//! 1. **Inactivity** — `PERMAGENT_PROVIDER_STREAM_STALL_SECS` (default 120,
//!    `0` = unlimited). A *fixed* window applied to each `next()`: how long the
//!    provider may go without saying anything at all. This is the bound that
//!    catches a socket that stops delivering, and it is the one a healthy
//!    response never touches however long that response runs.
//!
//!    **Lane R19a (harness loop 2026-09-14).** The window ABOVE — applied
//!    once a stream has already produced at least one item (even a
//!    usage-only chunk) — is not the same problem as the window BEFORE it
//!    has produced anything at all. M19 DeepSeek forensics: three m1-round
//!    tasks each sent an identical
//!    ~39 KB-system-prompt coordinator request that received **0 chars in
//!    120 s**, three times in a row, while a z.ai request on the same binary,
//!    at the same moment, under the same ~20 load average, with an
//!    equivalent-sized payload, answered in seconds — a provider-side
//!    prefill/queueing delay, not a dead socket. `PERMAGENT_PROVIDER_STREAM_-
//!    FIRST_BYTE_SECS` (default 300) now governs `next()` only until the
//!    first item arrives; every `next()` after that keeps using the ordinary
//!    120 s inactivity window above, unchanged. The caller (`agents::agent`)
//!    retries a first-byte stall once, since nothing has streamed yet to
//!    contradict a second attempt; Y8a's "never retry a genuine mid-stream
//!    stall or a repeating model" is untouched.
//! 2. **Total wall clock** — `PERMAGENT_PROVIDER_STREAM_MAX_SECS` (default
//!    1200, `0` = unlimited). Checked per delta, as a backstop on a stream that
//!    is *productive* forever. It is deliberately generous: a slow local model,
//!    an extended-thinking response or a long file write at ~15 tok/s crosses
//!    five minutes legitimately, and this failure is permanent (never retried),
//!    so a tight total ends honest turns. G13's H2 — the first shape of this
//!    module carried the *remaining total* into the per-`next()` timeout, which
//!    made 300 s a whole-response ceiling product-wide.
//! 3. **Cumulative output** — `PERMAGENT_PROVIDER_STREAM_MAX_OUTPUT_CHARS`
//!    (default 400 000, `0` = unlimited). fx04's 3.0 MB would have tripped this
//!    at roughly a tenth of the wall it actually burned.
//! 4. **Consecutive repetition** — the same substantial delta text
//!    ([`REPEAT_MIN_CHARS`] or more) repeated [`REPEAT_LIMIT`] times in a row.
//!    This is the *cheap* detector for the exact observed failure and it fires
//!    in seconds rather than minutes. It is deliberately conservative: only
//!    byte-identical, back-to-back deltas count, so ordinary repeated
//!    punctuation or a genuinely repetitive table never reaches it.
//!
//! **What a delta is.** Visible text, reasoning text, and the arguments of a
//! completed tool call ([`streamed_text`]). Tool-call arguments are accumulated
//! inside `formats::openai::response_to_streaming_message` and only surface
//! when the call is whole, so the *in-flight* accumulation is bounded by the
//! inactivity window alone (a tool call that never finishes never returns from
//! `next()`); the finished call is counted here (G13's L5).
//!
//! All three surface as [`ProviderError::stream_budget_exceeded`], which is
//! permanent and not retried — a retry would re-enter the same loop.

use std::time::{Duration, Instant};

use super::errors::ProviderError;
use crate::conversation::message::{Message, MessageContent};

/// Total wall-clock ceiling for a single provider response, in seconds.
/// `0` disables. This is a backstop, not the working bound — see
/// [`STALL_SECS_ENV`].
pub const MAX_SECS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_MAX_SECS";
/// Inactivity window for a single provider response, in seconds: how long one
/// `next()` may block without delivering anything ONCE THE STREAM HAS ALREADY
/// DELIVERED at least one item. `0` disables. See [`FIRST_BYTE_SECS_ENV`] for
/// the window before that.
pub const STALL_SECS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_STALL_SECS";
/// Lane R19a. Inactivity window for the FIRST `next()` of a response, in
/// seconds: how long we wait for the provider to say anything at all before
/// it has said anything. `0` disables (falls back to [`STALL_SECS_ENV`]'s
/// window, i.e. the pre-R19a behavior). Deliberately more generous than the
/// mid-stream window — a large context's prefill and a busy provider's queue
/// both look, from here, identical to a dead socket, and only one of them is
/// actually a problem.
pub const FIRST_BYTE_SECS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_FIRST_BYTE_SECS";
/// Cumulative streamed-output ceiling for a single response, in characters.
/// `0` disables.
pub const MAX_OUTPUT_CHARS_ENV: &str = "PERMAGENT_PROVIDER_STREAM_MAX_OUTPUT_CHARS";

/// 20 minutes. A response that is still *producing* after this is pathological;
/// one that is merely long is not, which is why this is not 300 (G13 H2).
pub const DEFAULT_MAX_SECS: u64 = 1200;
/// Two minutes of complete silence from the provider, ONCE it has already
/// delivered at least one item. Prefill on a large context plus a first-token
/// wait belongs to [`DEFAULT_FIRST_BYTE_SECS`] now (Lane R19a); a dead socket
/// mid-stream does not get that much rope.
pub const DEFAULT_STALL_SECS: u64 = 120;
/// Three minutes of complete silence before the FIRST item arrives (Lane
/// R19a; lowered from 300s to 180s by Lane R19c/G24 HIGH-2). Wider than
/// [`DEFAULT_STALL_SECS`] on purpose: the M19 DeepSeek forensics this
/// constant comes from measured a ~39 KB system prompt stalling at 0 chars
/// for the full prior 120 s bound, three times in a row, while an
/// equivalent-sized request to a different provider on the same binary at
/// the same moment answered in seconds — plausibly provider-side
/// queueing/prefill, not a dead connection.
///
/// **Worst case, Lane R19c.** The caller (`agents::agent`) retries this
/// specific timeout once before giving up, but the retry does NOT get a
/// fresh 180 s — it gets whatever remains of this same 180 s ceiling for the
/// WHOLE turn (`FIRST_BYTE_WINDOW_OVERRIDE` below), computed from how long
/// the turn has already spent waiting for a first byte. So one coordinator
/// turn spends **at most ~180 s of silent pre-first-byte waiting in total**
/// across both attempts, plus whatever the content stream itself legitimately
/// takes once bytes start arriving (bounded separately by
/// [`DEFAULT_STALL_SECS`] per gap and [`DEFAULT_MAX_SECS`] overall) — not the
/// ~600 s two independent fresh windows would have allowed (G24 HIGH-2: that
/// reliably exceeds a 600 s eval timeout and turns an honest give-up into an
/// ambiguous hard-kill with no receipt). With Lane R19b's keep-alive
/// liveness now live, a DeepSeek hold that sends ANY keep-alive resets the
/// raw-line clock and never reaches this timeout at all; 180 s total is
/// sized for the connection that is genuinely, fully silent — no bytes, no
/// keep-alives, nothing.
pub const DEFAULT_FIRST_BYTE_SECS: u64 = 180;
pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 400_000;

/// A delta shorter than this is never counted as a repetition: single tokens,
/// whitespace and punctuation repeat legitimately all the time.
pub const REPEAT_MIN_CHARS: usize = 40;
/// Consecutive identical substantial deltas before we call it a loop. fx04
/// emitted its sentence thousands of times; 50 is far inside that and far
/// outside anything a working model produces.
pub const REPEAT_LIMIT: usize = 50;

/// The text a delta actually streamed — visible text, reasoning text, and the
/// arguments of a completed tool call.
///
/// `Message::as_concat_text` sees only `Text` content, and the observed
/// runaway was a `reasoning: true` model. Counting only the visible half would
/// have let fx04 through untouched.
///
/// Tool-call arguments are counted because they are generated output like any
/// other (G13's L5): a model that loops inside a `str_replace` argument emits
/// megabytes that never appear as `Text`. They arrive in one piece — the SSE
/// consumer yields a tool call only once it is whole — so this bounds the
/// finished call, not the accumulation.
fn streamed_text(message: &Message) -> String {
    let mut out = String::new();
    for content in &message.content {
        match content {
            MessageContent::Text(text) => out.push_str(&text.text),
            MessageContent::Thinking(thinking) => out.push_str(&thinking.thinking),
            MessageContent::ToolRequest(request) => {
                if let Ok(call) = &request.tool_call {
                    if let Some(arguments) = &call.arguments {
                        if let Ok(rendered) = serde_json::to_string(arguments) {
                            out.push_str(&rendered);
                        }
                    }
                }
            }
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

tokio::task_local! {
    /// Lane R19c (G24 HIGH-2). Scoped override for the first-byte window
    /// `StreamBudget::from_env` would otherwise read from
    /// `PERMAGENT_PROVIDER_STREAM_FIRST_BYTE_SECS`/[`DEFAULT_FIRST_BYTE_SECS`].
    ///
    /// Exists so a first-byte-stall retry (`agents::agent`) can be given the
    /// REMAINING part of the turn's ~180s first-byte ceiling instead of a
    /// fresh window — the worst case this bounds is "one coordinator turn
    /// spends at most ~180s of silent pre-first-byte waiting in total,
    /// across both attempts" rather than ~2x180s. A `tokio::task_local!`
    /// rather than a process env var on purpose: setting an env var would be
    /// racy across the concurrently-running turns/providers this harness
    /// runs (exactly the `AUTHORIZED_ATTEMPTS` pattern in
    /// `cost_router::authorized_attempts` uses for the same reason — scoped
    /// to one async call tree, invisible to every other one).
    ///
    /// `Some(d)` sets the window to exactly `d` (including `Duration::ZERO`,
    /// which is a real "no time left", not "unlimited" — see
    /// [`StreamBudget::from_env`]). `None` explicitly disables the
    /// first-byte window for calls inside the scope (falls back to the
    /// ordinary stall window throughout, like `with_first_byte(None)`).
    /// Outside any scope — the ordinary, non-retry case — `from_env` reads
    /// the env var/default exactly as before this lane.
    pub static FIRST_BYTE_WINDOW_OVERRIDE: Option<Duration>;
}

#[derive(Debug)]
pub struct StreamBudget {
    started: Instant,
    /// Total wall clock. `None` means unlimited.
    max_wall: Option<Duration>,
    /// Inactivity window applied to each `next()` AFTER the first item has
    /// arrived. `None` means unlimited.
    max_stall: Option<Duration>,
    /// Lane R19a. Inactivity window applied to `next()` calls BEFORE the
    /// first item has arrived. `None` falls back to `max_stall` — the
    /// pre-R19a behavior of one uniform window for the whole stream.
    max_first_byte: Option<Duration>,
    /// Lane R19a. Has this stream delivered anything at all yet — a real
    /// delta or a usage-only chunk? Flips permanently on the first
    /// [`record`](Self::record)/[`record_text`](Self::record_text) call,
    /// which only ever happens after a `next()` has actually returned
    /// something (see `stream_openai_compat`'s loop).
    first_byte_received: bool,
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
        let stall = env_u64(STALL_SECS_ENV, DEFAULT_STALL_SECS);
        let chars = env_u64(MAX_OUTPUT_CHARS_ENV, DEFAULT_MAX_OUTPUT_CHARS as u64) as usize;
        // Lane R19c (G24 HIGH-2): a `FIRST_BYTE_WINDOW_OVERRIDE` scope wins
        // over the env var/default — this is how a first-byte-stall retry
        // gets the REMAINING part of the turn's ceiling instead of a fresh
        // window. Passed through exactly as given: `Some(Duration::ZERO)`
        // stays zero (real "no time left"), never reinterpreted as
        // "unlimited" the way a `0` from the env var is. No scope installed
        // (the ordinary, non-retry case) falls through to the env var/default
        // unchanged from before this lane.
        let first_byte =
            match FIRST_BYTE_WINDOW_OVERRIDE.try_with(|override_window| *override_window) {
                Ok(overridden) => overridden,
                Err(_) => {
                    let first_byte_secs = env_u64(FIRST_BYTE_SECS_ENV, DEFAULT_FIRST_BYTE_SECS);
                    (first_byte_secs > 0).then(|| Duration::from_secs(first_byte_secs))
                }
            };
        Self::new(
            (secs > 0).then(|| Duration::from_secs(secs)),
            (chars > 0).then_some(chars),
        )
        .with_stall((stall > 0).then(|| Duration::from_secs(stall)))
        .with_first_byte(first_byte)
    }

    pub fn new(max_wall: Option<Duration>, max_output_chars: Option<usize>) -> Self {
        Self {
            started: Instant::now(),
            max_wall,
            max_stall: None,
            max_first_byte: None,
            first_byte_received: false,
            max_output_chars,
            chars: 0,
            last_delta: String::new(),
            repeats: 0,
        }
    }

    /// Set the inactivity window applied once the stream has already
    /// delivered at least one item. `None` = unlimited, in which case the
    /// remaining total (if any) is what bounds a `next()`.
    pub fn with_stall(mut self, max_stall: Option<Duration>) -> Self {
        self.max_stall = max_stall;
        self
    }

    /// Lane R19a. Set the inactivity window applied before the stream has
    /// delivered anything at all. `None` = fall back to the ordinary stall
    /// window for the whole stream, matching the pre-R19a behavior.
    pub fn with_first_byte(mut self, max_first_byte: Option<Duration>) -> Self {
        self.max_first_byte = max_first_byte;
        self
    }

    /// Lane R19b. The window that should bound the wait for the very FIRST
    /// raw byte/line of the response — for a caller (`with_raw_line_liveness`
    /// in `openai_compatible.rs`) that enforces liveness at the raw-wire
    /// level rather than at this budget's own semantic-item `next_timeout`.
    /// Mirrors `next_timeout`'s own pre-first-byte fallback: `max_first_byte`,
    /// or `max_stall` when no dedicated first-byte window is configured.
    pub fn first_byte_window(&self) -> Option<Duration> {
        self.max_first_byte.or(self.max_stall)
    }

    /// Lane R19b. The window that should bound the wait between raw
    /// bytes/lines once at least one has already arrived. Same caller as
    /// [`first_byte_window`](Self::first_byte_window).
    pub fn stall_window(&self) -> Option<Duration> {
        self.max_stall
    }

    /// How much of the TOTAL wall clock is left. `None` = unlimited;
    /// `Some(ZERO)` = the budget is already spent. Checked per delta, never
    /// used as the per-`next()` timeout — see [`next_timeout`](Self::next_timeout).
    pub fn remaining(&self) -> Option<Duration> {
        self.max_wall
            .map(|max| max.saturating_sub(self.started.elapsed()))
    }

    /// How long the next `next()` may block: the fixed inactivity window for
    /// the stream's current phase (pre- or post-first-byte, Lane R19a), or
    /// the remaining total when that is tighter (or when there is no
    /// inactivity window at all). `None` = no timeout wrapper.
    pub fn next_timeout(&self) -> Option<Duration> {
        let window = if self.first_byte_received {
            self.max_stall
        } else {
            self.max_first_byte.or(self.max_stall)
        };
        match (window, self.remaining()) {
            (Some(window), Some(remaining)) => Some(window.min(remaining)),
            (Some(window), None) => Some(window),
            (None, remaining) => remaining,
        }
    }

    /// The error for a `next()` that timed out — which bound it broke depends
    /// on which one was the tighter.
    pub fn timeout_error(&self) -> ProviderError {
        if self.remaining().is_some_and(|left| left.is_zero()) {
            return self.wall_clock_error();
        }
        if !self.first_byte_received && self.max_first_byte.is_some() {
            return self.first_byte_error();
        }
        if self.max_stall.is_some() {
            self.stall_error()
        } else {
            self.wall_clock_error()
        }
    }

    /// Lane R19b. The cumulative CONTENT char count recorded so far — the
    /// same number `observed_rate` reports, exposed for a caller that needs
    /// to quote it alongside a different count (raw lines/bytes received at
    /// the wire level, which can be nonzero while this stays at 0 during a
    /// keep-alived-but-content-free hold).
    pub fn chars_so_far(&self) -> u64 {
        self.chars as u64
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

    /// The error for a stream that went silent for longer than the inactivity
    /// window allows.
    pub fn stall_error(&self) -> ProviderError {
        ProviderError::stream_budget_exceeded(format!(
            "the provider sent nothing for longer than {} allows ({}s); {} were received before \
             the stream was stopped. Raise or disable {} (0 = unlimited) if this model \
             legitimately pauses for longer.",
            STALL_SECS_ENV,
            self.max_stall.map(|d| d.as_secs()).unwrap_or(0),
            self.observed_rate(),
            STALL_SECS_ENV,
        ))
    }

    /// Lane R19a. The error for a stream that sent nothing at all — not one
    /// item, real or usage-only — within the first-byte window. Distinct
    /// from [`stall_error`](Self::stall_error): the caller (`agents::agent`)
    /// retries this specific failure once, because nothing has streamed yet
    /// to contradict a second attempt.
    pub fn first_byte_error(&self) -> ProviderError {
        ProviderError::first_byte_stall(format!(
            "the provider sent nothing at all for longer than {} allows ({}s); {} were received \
             before the stream was stopped. Raise or disable {} (0 = unlimited) if this model or \
             provider legitimately needs longer to start responding.",
            FIRST_BYTE_SECS_ENV,
            self.max_first_byte.map(|d| d.as_secs()).unwrap_or(0),
            self.observed_rate(),
            FIRST_BYTE_SECS_ENV,
        ))
    }

    /// The error for a stream that ran past its total wall-clock budget.
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
        // Lane R19a: `record`/`record_text` only ever runs after a `next()`
        // has actually returned an item (see `stream_openai_compat`'s loop),
        // so reaching this line at all — even for an empty/usage-only delta —
        // means the provider is no longer silent. Every `next()` after this
        // one uses the ordinary mid-stream stall window, not the first-byte
        // one, regardless of how short or empty this particular delta was.
        self.first_byte_received = true;

        let len = delta.chars().count();
        self.chars = self.chars.saturating_add(len);

        // The run is over CONSECUTIVE deltas, so ANY different delta ends it —
        // long or short. G13's M6: keeping the reset inside the length guard
        // made the bound "50 identical long deltas with no different LONG delta
        // between", which a provider batching line-sized deltas can reach while
        // writing a file with a repeated >=40-char line.
        //
        // An EMPTY delta is not a delta: a usage-only chunk carries no text, so
        // it must neither extend the run nor clear it.
        if !delta.is_empty() {
            if len >= REPEAT_MIN_CHARS && delta == self.last_delta {
                self.repeats = self.repeats.saturating_add(1);
                if self.repeats >= REPEAT_LIMIT {
                    return Err(self.repetition_error());
                }
            } else if len >= REPEAT_MIN_CHARS {
                self.last_delta.clear();
                self.last_delta.push_str(delta);
                self.repeats = 1;
            } else {
                self.last_delta.clear();
                self.repeats = 0;
            }
        }

        if let Some(max) = self.max_output_chars {
            if self.chars > max {
                return Err(self.output_error());
            }
        }

        // The per-`next()` timeout is an INACTIVITY window, so a stream that
        // keeps delivering deltas is never interrupted by it however long it
        // runs. The total is therefore checked here, per delta: this is the
        // only thing that bounds a productive runaway.
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
            budget
                .record(Some(&text_delta(" the")))
                .expect("ordinary tokens");
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

    /// G13 M6. A short delta between two identical long ones ends the run:
    /// the bound is "50 CONSECUTIVE identical substantial deltas", not "50 of
    /// them with no different LONG delta between". Before the fix this
    /// sequence accumulated to the limit and a file of repeated long lines
    /// could trip it.
    #[test]
    fn a_short_delta_between_two_identical_long_ones_breaks_the_run() {
        let long = "let assert_eq = 1; // a line long enough to count as substantial";
        assert!(long.chars().count() >= REPEAT_MIN_CHARS);
        let mut budget = StreamBudget::new(None, None);

        for _ in 0..(REPEAT_LIMIT * 4) {
            budget.record_text(long).expect("long");
            budget.record_text(" ").expect("short");
        }
    }

    /// A usage-only chunk (`record(None)` → an empty delta) is not a delta: it
    /// must not be able to launder a runaway past the repetition detector.
    #[test]
    fn a_usage_only_chunk_neither_counts_nor_clears_the_run() {
        let sentence = "Good! Cart preview is now updated. Need to check admin and API next.\n\n";
        let mut budget = StreamBudget::new(None, None);
        let mut failure = None;
        for _ in 0..(REPEAT_LIMIT * 2) {
            if let Err(e) = budget.record(Some(&text_delta(sentence))) {
                failure = Some(e);
                break;
            }
            if let Err(e) = budget.record(None) {
                failure = Some(e);
                break;
            }
        }
        let err = failure.expect("an interleaved usage chunk must not defeat the detector");
        assert!(err.to_string().contains("looping"), "{err}");
    }

    /// G13 L5: a completed tool call's arguments are generated output and
    /// count. `as_concat_text` — and the pre-fix `streamed_text` — saw none of
    /// it, so a runaway inside a `str_replace` argument was bounded by the
    /// clock alone.
    #[test]
    fn tool_call_arguments_count_towards_the_output_budget() {
        let call = rmcp::model::CallToolRequestParams::new("developer__text_editor")
            .with_arguments(rmcp::object!({ "new_str": "x".repeat(500) }));
        let message = Message::assistant().with_tool_request("call-1", Ok(call));
        assert!(
            message.as_concat_text().is_empty(),
            "precondition: the visible-text view is blind to a tool call"
        );

        let mut budget = StreamBudget::new(None, Some(100));
        let err = budget
            .record(Some(&message))
            .expect_err("tool-call arguments must be counted");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.to_string().contains(MAX_OUTPUT_CHARS_ENV), "{err}");
    }

    /// G13 H2: the per-`next()` timeout is a FIXED inactivity window, not the
    /// remaining total. A healthy response that runs for twenty minutes must
    /// see the same window on its last delta as on its first.
    #[test]
    fn the_next_timeout_is_a_fixed_inactivity_window() {
        let stall = Duration::from_secs(120);
        let budget =
            StreamBudget::new(Some(Duration::from_secs(1200)), None).with_stall(Some(stall));
        assert_eq!(
            budget.next_timeout(),
            Some(stall),
            "the total is 1200s but a single next() may only be silent for the stall window"
        );

        // Only when the remaining TOTAL is the tighter of the two does it win.
        let spent = StreamBudget::new(Some(Duration::from_secs(1)), None).with_stall(Some(stall));
        assert!(
            spent
                .next_timeout()
                .is_some_and(|d| d <= Duration::from_secs(1)),
            "the remaining total caps the window once it is the smaller number"
        );
        assert!(
            StreamBudget::new(None, None)
                .with_stall(Some(stall))
                .next_timeout()
                .is_some(),
            "an unlimited total still gets an inactivity window"
        );
    }

    /// Which bound a timeout names. A stall is not a total, and an error that
    /// names the wrong env var sends the operator to the wrong knob.
    #[test]
    fn a_timeout_names_the_bound_it_actually_broke() {
        let stalled = StreamBudget::new(Some(Duration::from_secs(1200)), None)
            .with_stall(Some(Duration::from_secs(120)));
        let err = stalled.timeout_error();
        assert!(err.to_string().contains(STALL_SECS_ENV), "{err}");
        assert!(err.is_stream_budget_exceeded(), "{err}");

        let spent = StreamBudget::new(Some(Duration::ZERO), None)
            .with_stall(Some(Duration::from_secs(120)));
        let err = spent.timeout_error();
        assert!(
            err.to_string().contains(MAX_SECS_ENV),
            "a spent total must name the total: {err}"
        );
    }

    /// The total is enforced per delta, because a productive runaway is never
    /// interrupted by an inactivity timeout.
    #[test]
    fn a_spent_total_stops_a_stream_that_is_still_producing() {
        let mut budget = StreamBudget::new(Some(Duration::ZERO), None);
        let err = budget
            .record_text("still talking")
            .expect_err("the total wall clock must bite on the next delta");
        assert!(err.to_string().contains(MAX_SECS_ENV), "{err}");
        assert!(err.is_permanent(), "{err}");
    }

    #[test]
    fn a_zero_budget_means_unlimited() {
        let budget = StreamBudget::new(None, None);
        assert!(budget.remaining().is_none(), "no wall clock, no timeout");
        assert!(
            budget.next_timeout().is_none(),
            "no stall window either: nothing wraps next()"
        );
    }

    #[test]
    fn a_spent_wall_clock_budget_is_reported_as_zero_remaining() {
        let budget = StreamBudget::new(Some(Duration::ZERO), None);
        assert_eq!(budget.remaining(), Some(Duration::ZERO));
        let err = budget.wall_clock_error();
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.to_string().contains(MAX_SECS_ENV), "{err}");
    }

    // --- Lane R19a: first-byte phase vs. mid-stream stall ---------------

    /// Before anything has arrived, `next_timeout` uses the first-byte
    /// window — not the (usually much tighter) mid-stream stall window. A
    /// large context's prefill and a busy provider's queue legitimately take
    /// longer than an ordinary pause between deltas.
    #[test]
    fn the_first_byte_window_applies_before_anything_is_received() {
        let budget = StreamBudget::new(None, None)
            .with_stall(Some(Duration::from_secs(120)))
            .with_first_byte(Some(Duration::from_secs(300)));
        assert_eq!(
            budget.next_timeout(),
            Some(Duration::from_secs(300)),
            "nothing received yet: the first-byte window applies, not the 120s stall window"
        );
    }

    /// Once the first item has arrived — even an empty/usage-only chunk with
    /// no text — `next_timeout` switches to the ordinary mid-stream stall
    /// window. Y8a's protection for a genuinely dead mid-stream socket must
    /// fire exactly as before, at 120s, not at the wider first-byte window.
    #[test]
    fn the_stall_window_applies_once_the_first_byte_has_arrived() {
        let mut budget = StreamBudget::new(None, None)
            .with_stall(Some(Duration::from_secs(120)))
            .with_first_byte(Some(Duration::from_secs(300)));
        budget
            .record(None)
            .expect("a usage-only chunk is not itself a failure");
        assert_eq!(
            budget.next_timeout(),
            Some(Duration::from_secs(120)),
            "the first item (even an empty one) arrived: fall back to the mid-stream window"
        );
    }

    /// A timeout before the first byte reports the first-byte bound by name,
    /// and is classified as a first-byte stall distinct from an ordinary
    /// mid-stream one — the caller uses this to decide whether to retry.
    #[test]
    fn a_pre_first_byte_timeout_reports_the_first_byte_bound() {
        let budget = StreamBudget::new(None, None)
            .with_stall(Some(Duration::from_secs(120)))
            .with_first_byte(Some(Duration::from_secs(300)));
        let err = budget.timeout_error();
        assert!(err.to_string().contains(FIRST_BYTE_SECS_ENV), "{err}");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(err.is_first_byte_provider_stall(), "{err}");
    }

    /// A mid-stream timeout — after at least one item has arrived — still
    /// trips the ordinary 120s stall bound, names it, and is NOT classified
    /// as a first-byte stall, so it keeps Y8a's "do not retry" handling
    /// exactly as before this lane.
    #[test]
    fn a_mid_stream_timeout_still_trips_the_ordinary_stall_bound() {
        let mut budget = StreamBudget::new(None, None)
            .with_stall(Some(Duration::from_secs(120)))
            .with_first_byte(Some(Duration::from_secs(300)));
        budget
            .record_text("hello ")
            .expect("the first delta arrives fine");
        let err = budget.timeout_error();
        assert!(err.to_string().contains(STALL_SECS_ENV), "{err}");
        assert!(err.is_stream_budget_exceeded(), "{err}");
        assert!(!err.is_first_byte_provider_stall(), "{err}");
    }

    /// With no first-byte window configured, `next_timeout` falls back to
    /// the ordinary stall window for the whole stream — the pre-R19a
    /// behavior — so a caller that never opts in (or sets
    /// `PERMAGENT_PROVIDER_STREAM_FIRST_BYTE_SECS=0`) is unaffected.
    #[test]
    fn no_first_byte_window_falls_back_to_the_stall_window_throughout() {
        let stall = Duration::from_secs(120);
        let budget = StreamBudget::new(None, None).with_stall(Some(stall));
        assert_eq!(budget.next_timeout(), Some(stall));
        let err = budget.timeout_error();
        assert!(err.to_string().contains(STALL_SECS_ENV), "{err}");
        assert!(!err.is_first_byte_provider_stall(), "{err}");
    }

    /// The spent-total wall-clock bound still wins over the first-byte
    /// window when it is the tighter of the two, exactly as it already did
    /// over the stall window.
    #[test]
    fn a_spent_total_wins_over_the_first_byte_window() {
        let budget = StreamBudget::new(Some(Duration::ZERO), None)
            .with_stall(Some(Duration::from_secs(120)))
            .with_first_byte(Some(Duration::from_secs(300)));
        let err = budget.timeout_error();
        assert!(err.to_string().contains(MAX_SECS_ENV), "{err}");
        assert!(!err.is_first_byte_provider_stall(), "{err}");
    }

    /// Lane R19c (G24 HIGH-2): lowered from 300s to 180s so one coordinator
    /// turn's worst-case silent pre-first-byte wait (original attempt +
    /// retry, `FIRST_BYTE_WINDOW_OVERRIDE` bounding the retry to the
    /// REMAINING part of this same ceiling) stays near 180s total instead of
    /// ~2x300s=600s.
    #[test]
    fn the_first_byte_default_is_180s() {
        assert_eq!(DEFAULT_FIRST_BYTE_SECS, 180);
    }

    // --- Lane R19c: retry gets the REMAINING first-byte budget, not a fresh window (G24 HIGH-2) ---

    /// With no override task-local in scope, `from_env` behaves exactly as
    /// before (falls through to the env var / default) — the override is
    /// opt-in per call tree, never ambient.
    #[tokio::test]
    async fn with_no_override_in_scope_from_env_reads_the_ordinary_default() {
        // Guards (and serializes with) FIRST_BYTE_SECS_ENV so this assertion
        // is not racing some other test's env_lock-guarded override of the
        // same process-wide env var.
        let _guard = env_lock::lock_env([(FIRST_BYTE_SECS_ENV, None::<&str>)]);
        let budget = StreamBudget::from_env();
        assert_eq!(
            budget.first_byte_window(),
            Some(Duration::from_secs(DEFAULT_FIRST_BYTE_SECS)),
            "no override installed: the ordinary default applies"
        );
    }

    /// Inside `FIRST_BYTE_WINDOW_OVERRIDE`'s scope, `from_env` uses the
    /// override instead of the env var/default — this is how the retry gets
    /// the REMAINING budget rather than a fresh 180s window.
    #[tokio::test]
    async fn an_override_in_scope_replaces_the_first_byte_window() {
        let remaining = Duration::from_secs(7);
        let observed = FIRST_BYTE_WINDOW_OVERRIDE
            .scope(Some(remaining), async {
                StreamBudget::from_env().first_byte_window()
            })
            .await;
        assert_eq!(observed, Some(remaining));
    }

    /// `Some(Duration::ZERO)` is a valid, meaningful override — the first
    /// attempt already spent the whole 180s ceiling, so the retry gets
    /// (deliberately) almost no additional silent wait: if the provider
    /// answers essentially immediately this time, the retry still succeeds;
    /// if not, it fails fast instead of burning another 180s.
    #[tokio::test]
    async fn a_zero_remaining_override_is_not_treated_as_unlimited() {
        let observed = FIRST_BYTE_WINDOW_OVERRIDE
            .scope(Some(Duration::ZERO), async {
                StreamBudget::from_env().first_byte_window()
            })
            .await;
        assert_eq!(
            observed,
            Some(Duration::ZERO),
            "zero remaining must stay zero, not silently become unlimited"
        );
    }

    /// `None` inside the scope means "no first-byte window at all" — an
    /// explicit unlimited, distinct from "no override installed".
    #[tokio::test]
    async fn a_none_override_means_explicitly_unlimited() {
        let observed = FIRST_BYTE_WINDOW_OVERRIDE
            .scope(None, async { StreamBudget::from_env().first_byte_window() })
            .await;
        assert_eq!(observed, None);
    }
}
