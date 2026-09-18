//! Per-turn latency instrumentation for the coding harness (excellence
//! iteration lane X-lat).
//!
//! The gate this exists to evaluate — "harness-only overhead ≤ 250ms p50 /
//! 1s p95" — was UNEVALUABLE before this: no run separated harness time from
//! model/tool time, so every prior report could only quote wall time and
//! tool-call counts. `Agent::reply_internal`'s turn loop (`crates/goose/src/
//! agents/agent.rs`) uses [`TurnTimingRecorder`] to measure, per outer-loop
//! turn: time spent waiting on the provider, time spent dispatching/awaiting
//! tool calls, time spent in the after-turn hook chain (`PrematureDoneGuard`/
//! `ReviewerMandate`), and time spent on context management (recovery
//! compaction + the tool-pair summarization await). The result rides the
//! existing per-call `cost_ledger` row (a new nullable `timing_json` column,
//! set by `SessionManager::update_cost_ledger_timing` keyed on the same
//! `call_id`/invocation id the ledger row already uses) rather than a new
//! store.
//!
//! [`compute_rollup`] turns a session's turns into the session-level view
//! (`turns`, `time_to_first_tool_ms`, and the p50/p95 of `harness_overhead_ms`
//! — the gate metric itself), and [`format_harness_overhead_line`] is the
//! stable plain-text line the CLI's headless run prints so `permagent-eval`'s
//! harness_log parser can read it (lane X-lat-2).

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// One tool call's wall-clock span, dispatch to result. Tool calls within a
/// turn can run concurrently (`stream::select_all` in the reply loop), so
/// several of these can overlap in wall-clock time — this is per-tool detail
/// for diagnostics, not an additive breakdown of [`TurnTiming::tool_exec_ms`],
/// which is measured separately as the wall span of the whole dispatch phase
/// so concurrent tools are not double-counted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallTiming {
    pub name: String,
    pub ms: i64,
}

/// Wall-clock breakdown of one reply-loop turn (one outer-loop iteration of
/// `Agent::reply_internal`). `harness_overhead_ms` is the gate metric:
/// everything the turn spent that was neither waiting on the model nor
/// waiting on tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TurnTiming {
    /// Total wall time for this turn: from the top of the outer loop
    /// iteration to the point its end-of-turn bookkeeping finishes.
    pub wall_ms: i64,
    /// Time spent waiting on the provider stream (request issued to the
    /// stream fully draining), net of any tool-dispatch time nested inside
    /// that same attempt — measured separately as `tool_exec_ms`. Summed
    /// across every `start_model`/`end_model` pair in the turn, so a turn
    /// that retries the provider call (recovery compaction, a permanent-
    /// failure fallback swap) still reports one total.
    pub model_ms: i64,
    /// Time to the first item out of the provider stream, if the turn made a
    /// provider call at all. `None` when the turn never reached
    /// `start_model` (e.g. it was refused before dispatch).
    pub model_first_token_ms: Option<i64>,
    /// Wall-clock time this turn spent dispatching and awaiting tool calls —
    /// the wall span of the dispatch phase, not the sum of individual tool
    /// durations (which would double-count concurrent tools).
    pub tool_exec_ms: i64,
    /// Per-tool-call detail backing `tool_exec_ms`.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTiming>,
    /// Elapsed time from the top of this turn to its first tool dispatch,
    /// if any tool was called this turn. Feeds the session-level
    /// `time_to_first_tool_ms` rollup.
    pub time_to_first_tool_dispatch_ms: Option<i64>,
    /// Time spent in the after-turn hook chain (`PrematureDoneGuard`,
    /// `ReviewerMandate`, ...) once the turn decided it was done.
    pub after_turn_ms: i64,
    /// Time spent on context management this turn: recovery compaction
    /// (triggered by a `ContextLengthExceeded` provider error) plus the
    /// tool-pair summarization await. Diagnostic detail — it is not
    /// subtracted in `harness_overhead_ms`, which is defined as exactly
    /// `wall - model - tool_exec` per the gate's definition.
    pub context_mgmt_ms: i64,
}

impl TurnTiming {
    /// `wall - model - tool_exec`, floored at zero. Never negative: nested
    /// spans racing the outer wall-clock measurement (scheduler jitter, clock
    /// coarseness) must read as "no measured overhead", not a nonsensical
    /// negative duration that would corrupt a percentile.
    pub fn harness_overhead_ms(&self) -> i64 {
        (self.wall_ms - self.model_ms - self.tool_exec_ms).max(0)
    }
}

/// Accumulates one turn's timing as the reply loop runs it, then produces the
/// immutable [`TurnTiming`] record via [`Self::finish`]. Every measurement is
/// real-`Instant`-based wall-clock time — deliberately not mockable, so the
/// unit tests below use small real `tokio::time::sleep`s and tolerate actual
/// scheduler jitter rather than asserting exact equality.
#[derive(Debug)]
pub struct TurnTimingRecorder {
    turn_start: Instant,
    model_started_at: Option<Instant>,
    model_ms_accum: i64,
    model_first_token_ms: Option<i64>,
    tool_exec_ms_accum: i64,
    tool_calls: Vec<ToolCallTiming>,
    time_to_first_tool_dispatch_ms: Option<i64>,
    after_turn_ms_accum: i64,
    context_mgmt_ms_accum: i64,
}

impl TurnTimingRecorder {
    /// Start timing a turn. Call once at the top of the reply loop's turn
    /// iteration, before any provider or tool work begins.
    pub fn start() -> Self {
        Self {
            turn_start: Instant::now(),
            model_started_at: None,
            model_ms_accum: 0,
            model_first_token_ms: None,
            tool_exec_ms_accum: 0,
            tool_calls: Vec::new(),
            time_to_first_tool_dispatch_ms: None,
            after_turn_ms_accum: 0,
            context_mgmt_ms_accum: 0,
        }
    }

    /// Call immediately before issuing the provider stream request.
    pub fn start_model(&mut self) {
        self.model_started_at = Some(Instant::now());
    }

    /// Call when the first item arrives from the provider stream (whether
    /// `Ok` or `Err`). A no-op after the first call within the turn, and a
    /// no-op if `start_model` was never called.
    pub fn record_first_token(&mut self) {
        if self.model_first_token_ms.is_none() {
            if let Some(started) = self.model_started_at {
                self.model_first_token_ms = Some(started.elapsed().as_millis() as i64);
            }
        }
    }

    /// Call once the provider stream attempt has fully drained — normal end,
    /// error, or cancellation. Safe to call more than once per turn (a
    /// recovery-compaction retry or a permanent-failure fallback swap makes
    /// a second `start_model`/`end_model` pair); each pair's span adds to the
    /// running total. A stray `end_model` with no matching `start_model` is
    /// a no-op rather than a panic — this is observability, not a correctness
    /// invariant worth crashing a turn over.
    pub fn end_model(&mut self) {
        if let Some(started) = self.model_started_at.take() {
            self.model_ms_accum += started.elapsed().as_millis() as i64;
        }
    }

    /// Record one tool call's dispatch-to-result span. `dispatched_at` is the
    /// instant the tool-dispatch phase began (shared across every tool
    /// dispatched together in that phase, since they are kicked off
    /// concurrently) — so this is "time since the phase started", which for a
    /// tool that finishes early is shorter than for one that runs long,
    /// exactly mirroring wall-clock reality for concurrent work.
    pub fn record_tool_call(&mut self, name: impl Into<String>, dispatched_at: Instant) {
        if self.time_to_first_tool_dispatch_ms.is_none() {
            self.time_to_first_tool_dispatch_ms =
                Some((dispatched_at.saturating_duration_since(self.turn_start)).as_millis() as i64);
        }
        self.tool_calls.push(ToolCallTiming {
            name: name.into(),
            ms: dispatched_at.elapsed().as_millis() as i64,
        });
    }

    /// Add wall-clock time for the tool-dispatch PHASE (which can cover
    /// several concurrent tool calls). This — not the sum of individual
    /// `record_tool_call` spans — is what feeds `tool_exec_ms` /
    /// `harness_overhead_ms`, so concurrent tools are not double-counted.
    /// Call once after the phase's await loop fully drains.
    pub fn record_tool_phase(&mut self, phase_started_at: Instant) {
        self.tool_exec_ms_accum += phase_started_at.elapsed().as_millis() as i64;
    }

    /// Record time spent in the after-turn hook chain.
    pub fn record_after_turn(&mut self, started_at: Instant) {
        self.after_turn_ms_accum += started_at.elapsed().as_millis() as i64;
    }

    /// Record time spent on context management (recovery compaction, or the
    /// tool-pair summarization await). Callable more than once per turn — the
    /// two sources add.
    pub fn record_context_mgmt(&mut self, started_at: Instant) {
        self.context_mgmt_ms_accum += started_at.elapsed().as_millis() as i64;
    }

    /// Finish the turn and produce its immutable record.
    pub fn finish(self) -> TurnTiming {
        TurnTiming {
            wall_ms: self.turn_start.elapsed().as_millis() as i64,
            model_ms: self.model_ms_accum,
            model_first_token_ms: self.model_first_token_ms,
            tool_exec_ms: self.tool_exec_ms_accum,
            tool_calls: self.tool_calls,
            time_to_first_tool_dispatch_ms: self.time_to_first_tool_dispatch_ms,
            after_turn_ms: self.after_turn_ms_accum,
            context_mgmt_ms: self.context_mgmt_ms_accum,
        }
    }
}

/// Session-level rollup over every turn's [`TurnTiming`], in chronological
/// (ledger insertion) order.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionTimingRollup {
    pub turns: u32,
    /// Wall time from the session's first turn to the first tool call
    /// dispatched by ANY turn in the sample. `None` when no sampled turn
    /// ever dispatched a tool.
    pub time_to_first_tool_ms: Option<i64>,
    pub harness_overhead_p50_ms: i64,
    pub harness_overhead_p95_ms: i64,
    pub model_ms_total: i64,
    pub tool_exec_ms_total: i64,
    pub wall_ms_total: i64,
}

/// Nearest-rank percentile over an ascending-sorted slice. `p` is a fraction
/// in `[0, 1]`. Empty input reads as 0 — a session with no turns has no
/// overhead to report, not an error.
fn percentile_sorted(sorted: &[i64], p: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

/// Build the session rollup from every turn's timing record, given in
/// chronological order. `time_to_first_tool_ms` is derived by walking the
/// turns in order and adding each one's own wall time until the first turn
/// that dispatched a tool, then adding that turn's
/// `time_to_first_tool_dispatch_ms` offset.
pub fn compute_rollup(records: &[TurnTiming]) -> SessionTimingRollup {
    let mut overhead: Vec<i64> = records
        .iter()
        .map(TurnTiming::harness_overhead_ms)
        .collect();
    overhead.sort_unstable();

    let mut cumulative_wall_ms: i64 = 0;
    let mut time_to_first_tool_ms = None;
    for turn in records {
        if time_to_first_tool_ms.is_none() {
            if let Some(offset) = turn.time_to_first_tool_dispatch_ms {
                time_to_first_tool_ms = Some(cumulative_wall_ms + offset);
            }
        }
        cumulative_wall_ms += turn.wall_ms;
    }

    SessionTimingRollup {
        turns: records.len() as u32,
        time_to_first_tool_ms,
        harness_overhead_p50_ms: percentile_sorted(&overhead, 0.50),
        harness_overhead_p95_ms: percentile_sorted(&overhead, 0.95),
        model_ms_total: records.iter().map(|t| t.model_ms).sum(),
        tool_exec_ms_total: records.iter().map(|t| t.tool_exec_ms).sum(),
        wall_ms_total: records.iter().map(|t| t.wall_ms).sum(),
    }
}

/// The stable, plain-text final line a headless CLI run prints so
/// `permagent-eval`'s harness_log parser can read it without touching JSON.
/// Exact, stable format:
///
/// ```text
/// harness_overhead p50=<ms> p95=<ms> model=<ms> tools=<ms> wall=<ms>
/// ```
///
/// All five values are non-negative integer milliseconds. `p50`/`p95` are the
/// session's harness-only overhead percentiles — the gate metric itself
/// ("harness-only overhead ≤ 250ms p50 / 1s p95"). `model`, `tools` and
/// `wall` are SESSION TOTALS (summed across every turn), not per-turn
/// averages, so a reader can cross-check `wall - model - tools` against the
/// reported overhead percentiles.
pub fn format_harness_overhead_line(rollup: &SessionTimingRollup) -> String {
    format!(
        "harness_overhead p50={} p95={} model={} tools={} wall={}",
        rollup.harness_overhead_p50_ms,
        rollup.harness_overhead_p95_ms,
        rollup.model_ms_total,
        rollup.tool_exec_ms_total,
        rollup.wall_ms_total,
    )
}

/// E2c (Round E, coding-harness excellence loop): attempt/cache/wall-budget
/// context a goal-run driver can supply alongside a session's timing rollup,
/// so the printed CLI line lets M11's analysis attribute wall time per
/// attempt (M9's/F9's dominant remaining failure class was wall-clock
/// exhaustion across multiple attempts, and the harness_overhead line's
/// SESSION-total `model`/`tools`/`wall` numbers alone cannot say how many
/// attempts that total was spread across, nor how much of it the E2a check
/// cache already saved).
///
/// Every field is optional: a caller that does not track attempts (an
/// ordinary interactive session, or a run from before this lane) simply
/// omits it, and [`format_harness_overhead_line_with_attempts`] omits that
/// token entirely rather than printing a fabricated `0`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AttemptContext {
    /// How many dispatch attempts this goal run has made so far (including
    /// the current one).
    pub attempts: Option<u32>,
    /// How many `check_cache` hits this run recorded (`crate::check_cache`).
    pub cache_hits: Option<u32>,
    /// Remaining wall-clock budget, in milliseconds, at the point this line
    /// is printed — see `goal_engine::wall_budget_permits_another_attempt`.
    pub wall_budget_remaining_ms: Option<i64>,
}

/// [`format_harness_overhead_line`], extended with [`AttemptContext`].
/// Additive only: the original five `p50`/`p95`/`model`/`tools`/`wall`
/// tokens are printed in the exact same order and format as before, and each
/// new token is appended ONLY when its value is present. A reader that only
/// parses the first five space-separated tokens (the pre-E2c contract)
/// continues to work unchanged; `permagent-eval`'s harness_log parser can
/// start reading the new trailing tokens without any format-version bump.
///
/// This deliberately needs no schema/migration change: `cost_ledger.
/// timing_json` (added at schema v54 by X-lat, the same v54 the loop log
/// flags as colliding with the voice branch's `voice_turns` table — see
/// HARNESS_LOOP_LOG.md's X-lat/G5 entries) is already a JSON column, so new
/// `TurnTiming`/rollup-adjacent fields serialize into it for free. Only a
/// genuinely new FIXED SQL column would need a schema bump; this lane adds
/// none.
pub fn format_harness_overhead_line_with_attempts(
    rollup: &SessionTimingRollup,
    extras: &AttemptContext,
) -> String {
    let mut line = format_harness_overhead_line(rollup);
    if let Some(attempts) = extras.attempts {
        line.push_str(&format!(" attempts={attempts}"));
    }
    if let Some(cache_hits) = extras.cache_hits {
        line.push_str(&format!(" cache_hits={cache_hits}"));
    }
    if let Some(remaining_ms) = extras.wall_budget_remaining_ms {
        line.push_str(&format!(" wall_budget={remaining_ms}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn timing(wall_ms: i64, model_ms: i64, tool_exec_ms: i64) -> TurnTiming {
        TurnTiming {
            wall_ms,
            model_ms,
            model_first_token_ms: Some(model_ms / 2),
            tool_exec_ms,
            tool_calls: vec![],
            time_to_first_tool_dispatch_ms: None,
            after_turn_ms: 0,
            context_mgmt_ms: 0,
        }
    }

    #[test]
    fn harness_overhead_is_wall_minus_model_minus_tools() {
        let t = timing(1000, 700, 200);
        assert_eq!(t.harness_overhead_ms(), 100);
    }

    #[test]
    fn harness_overhead_floors_at_zero() {
        // Measurement noise: nested spans can (barely) exceed the outer wall
        // span. The gate metric must never read negative.
        let t = timing(100, 80, 40);
        assert_eq!(t.harness_overhead_ms(), 0);
    }

    #[test]
    fn rollup_percentiles_over_twenty_turns() {
        // Overhead values 1..=20ms: p50 is the 10th ascending value
        // (rank = ceil(0.5*20) = 10), p95 is the 19th (rank = ceil(0.95*20) = 19).
        let records: Vec<TurnTiming> = (1..=20)
            .map(|overhead_ms| timing(overhead_ms + 500, 500, 0))
            .collect();
        let rollup = compute_rollup(&records);
        assert_eq!(rollup.turns, 20);
        assert_eq!(rollup.time_to_first_tool_ms, None);
        assert_eq!(rollup.harness_overhead_p50_ms, 10);
        assert_eq!(rollup.harness_overhead_p95_ms, 19);
        assert_eq!(rollup.model_ms_total, 500 * 20);
        assert_eq!(rollup.wall_ms_total, (1..=20i64).sum::<i64>() + 500 * 20);
    }

    #[test]
    fn rollup_of_no_turns_is_zeroed_not_an_error() {
        let rollup = compute_rollup(&[]);
        assert_eq!(rollup.turns, 0);
        assert_eq!(rollup.harness_overhead_p50_ms, 0);
        assert_eq!(rollup.harness_overhead_p95_ms, 0);
        assert_eq!(rollup.time_to_first_tool_ms, None);
    }

    #[test]
    fn time_to_first_tool_ms_accounts_for_prior_turns_wall_time() {
        let mut first_turn = timing(50, 50, 0);
        first_turn.time_to_first_tool_dispatch_ms = None;
        let mut second_turn = timing(80, 60, 20);
        second_turn.time_to_first_tool_dispatch_ms = Some(30);
        let rollup = compute_rollup(&[first_turn, second_turn]);
        // 50ms (all of turn 1) + 30ms (into turn 2 before its first tool
        // dispatch) = 80ms.
        assert_eq!(rollup.time_to_first_tool_ms, Some(80));
    }

    #[test]
    fn format_line_matches_the_documented_stable_shape() {
        let rollup = SessionTimingRollup {
            turns: 3,
            time_to_first_tool_ms: Some(42),
            harness_overhead_p50_ms: 120,
            harness_overhead_p95_ms: 480,
            model_ms_total: 9000,
            tool_exec_ms_total: 1500,
            wall_ms_total: 11000,
        };
        assert_eq!(
            format_harness_overhead_line(&rollup),
            "harness_overhead p50=120 p95=480 model=9000 tools=1500 wall=11000"
        );
    }

    // ── E2c: attempts=/cache_hits=/wall_budget= extension ───────────────

    fn sample_rollup() -> SessionTimingRollup {
        SessionTimingRollup {
            turns: 3,
            time_to_first_tool_ms: Some(42),
            harness_overhead_p50_ms: 120,
            harness_overhead_p95_ms: 480,
            model_ms_total: 9000,
            tool_exec_ms_total: 1500,
            wall_ms_total: 11000,
        }
    }

    /// All three extras present → all three trailing tokens appended, in
    /// order, after the unchanged base line.
    #[test]
    fn format_line_with_attempts_appends_all_three_tokens_when_present() {
        let extras = AttemptContext {
            attempts: Some(3),
            cache_hits: Some(2),
            wall_budget_remaining_ms: Some(45_000),
        };
        assert_eq!(
            format_harness_overhead_line_with_attempts(&sample_rollup(), &extras),
            "harness_overhead p50=120 p95=480 model=9000 tools=1500 wall=11000 \
             attempts=3 cache_hits=2 wall_budget=45000"
        );
    }

    /// No extras at all → byte-identical to the original, unextended line —
    /// the additive/backward-compatible contract the doc comment promises.
    #[test]
    fn format_line_with_attempts_matches_the_base_line_when_extras_are_all_absent() {
        let extras = AttemptContext::default();
        assert_eq!(
            format_harness_overhead_line_with_attempts(&sample_rollup(), &extras),
            format_harness_overhead_line(&sample_rollup()),
        );
    }

    /// A partially-populated extras struct appends ONLY the present tokens —
    /// a caller that tracks attempts but not cache hits never prints a
    /// fabricated `cache_hits=0`.
    #[test]
    fn format_line_with_attempts_appends_only_the_present_fields() {
        let extras = AttemptContext {
            attempts: Some(1),
            cache_hits: None,
            wall_budget_remaining_ms: None,
        };
        assert_eq!(
            format_harness_overhead_line_with_attempts(&sample_rollup(), &extras),
            "harness_overhead p50=120 p95=480 model=9000 tools=1500 wall=11000 attempts=1"
        );
    }

    /// X-lat-1's required test: a fake provider with a fixed delay and a fake
    /// tool with a fixed delay → the record's fields are within tolerance and
    /// overhead is small. Exercises the real `TurnTimingRecorder` the reply
    /// loop uses, driven with real (small) `tokio::time::sleep`s standing in
    /// for "the provider took N ms" and "the tool took M ms", rather than
    /// mocking the clock.
    #[tokio::test]
    async fn recorder_measures_a_fake_provider_delay_and_a_fake_tool_delay() {
        const MODEL_DELAY_MS: u64 = 40;
        const TOOL_DELAY_MS: u64 = 25;
        // Real-clock tests need slack for scheduler jitter under load (CI,
        // a busy dev machine) without being so loose they stop meaning
        // anything.
        const TOLERANCE_MS: i64 = 60;

        let mut recorder = TurnTimingRecorder::start();

        // Fake provider call.
        recorder.start_model();
        tokio::time::sleep(Duration::from_millis(MODEL_DELAY_MS)).await;
        recorder.record_first_token();
        tokio::time::sleep(Duration::from_millis(MODEL_DELAY_MS)).await;
        recorder.end_model();

        // Fake tool dispatch phase, one tool call.
        let tool_phase_start = Instant::now();
        tokio::time::sleep(Duration::from_millis(TOOL_DELAY_MS)).await;
        recorder.record_tool_call("fake_tool", tool_phase_start);
        recorder.record_tool_phase(tool_phase_start);

        let timing = recorder.finish();

        let expected_model_ms = (MODEL_DELAY_MS * 2) as i64;
        assert!(
            (timing.model_ms - expected_model_ms).abs() <= TOLERANCE_MS,
            "model_ms {} not within tolerance of expected {expected_model_ms}",
            timing.model_ms
        );
        let first_token_ms = timing
            .model_first_token_ms
            .expect("a provider call was made, so first-token time is recorded");
        assert!(
            (first_token_ms - MODEL_DELAY_MS as i64).abs() <= TOLERANCE_MS,
            "model_first_token_ms {first_token_ms} not within tolerance of {MODEL_DELAY_MS}"
        );
        assert!(
            (timing.tool_exec_ms - TOOL_DELAY_MS as i64).abs() <= TOLERANCE_MS,
            "tool_exec_ms {} not within tolerance of expected {TOOL_DELAY_MS}",
            timing.tool_exec_ms
        );
        assert_eq!(timing.tool_calls.len(), 1);
        assert_eq!(timing.tool_calls[0].name, "fake_tool");
        assert!(
            (timing.tool_calls[0].ms - TOOL_DELAY_MS as i64).abs() <= TOLERANCE_MS,
            "tool_calls[0].ms {} not within tolerance of expected {TOOL_DELAY_MS}",
            timing.tool_calls[0].ms
        );
        assert!(
            timing.time_to_first_tool_dispatch_ms.is_some(),
            "a tool was dispatched this turn"
        );

        // The whole turn was ~ model + tool time (sequential in this test),
        // so overhead — everything else — must be small.
        assert!(
            timing.harness_overhead_ms() <= TOLERANCE_MS,
            "harness_overhead_ms {} should be small when the turn did only \
             modeled provider + tool work",
            timing.harness_overhead_ms()
        );
    }

    #[test]
    fn end_model_without_start_model_is_a_harmless_no_op() {
        let mut recorder = TurnTimingRecorder::start();
        recorder.end_model();
        let timing = recorder.finish();
        assert_eq!(timing.model_ms, 0);
    }

    #[test]
    fn model_ms_sums_across_a_retried_provider_call() {
        // A recovery-compaction retry or a permanent-failure fallback swap
        // makes a second start_model/end_model pair within one turn.
        let mut recorder = TurnTimingRecorder::start();
        recorder.start_model();
        recorder.end_model();
        recorder.start_model();
        recorder.end_model();
        let timing = recorder.finish();
        // Both spans measured; neither call panics or overwrites the other.
        assert!(timing.model_ms >= 0);
    }
}
