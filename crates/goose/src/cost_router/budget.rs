//! Spend caps — the "no surprises" guarantee.
//!
//! A budget check against the per-call cost ledger (#714): every response's USD
//! cost comes from `providers::canonical::cost::cost_of` and rolls up into
//! `sessions.accumulated_cost_usd`. This module turns that running total into a
//! three-band decision, per scope:
//!
//! - **SOFT** — a non-blocking alert (log/ambient): "you've spent $X".
//! - **GATE** — raise a `choice` decision in the Decision Inbox and pause:
//!   "spent $X, continue? [+increment / to completion / stop & keep changes]".
//!   A bare `choice` is fail-closed Tier 2, so only the user can answer — spend
//!   never silently continues past the gate.
//! - **HARD** — stop, preserving all work (never a mid-edit abort).
//!
//! Two independent scopes — a single **task** and the whole **session** — and
//! the STRICTER verdict wins. Defaults follow the cost-governance spec: task
//! $2/$5/$10, session $10/$25/$50 (soft/gate/hard). All CONFIGURABLE via env /
//! config; the defaults are the owner's spend-comfort call, surfaced for tuning.
//!
//! `budget_verdict` is the pure `TODO(S8)` seam the loop-safety ProgressMonitor
//! (#715, unmerged) calls pre-turn — "read the session's accumulated $-spend
//! and, if it exceeds the configured cap, escalate to the Decision Inbox and
//! pause". `raise_budget_gate` is the thin Decision-Inbox writer it invokes when
//! the verdict is `Gate`. The pure core has no async/DB so it is directly
//! testable.

use sqlx::{Pool, Sqlite};

use crate::decisions::{self, ChoiceOption, ChoicePayload, Decision, NewDecision};

// ── Config keys (all optional; unset ⇒ the spec default) ─────────────────────

pub const KEY_TASK_SOFT: &str = "PERMAGENT_BUDGET_TASK_SOFT_USD";
pub const KEY_TASK_GATE: &str = "PERMAGENT_BUDGET_TASK_GATE_USD";
pub const KEY_TASK_HARD: &str = "PERMAGENT_BUDGET_TASK_HARD_USD";
pub const KEY_SESSION_SOFT: &str = "PERMAGENT_BUDGET_SESSION_SOFT_USD";
pub const KEY_SESSION_GATE: &str = "PERMAGENT_BUDGET_SESSION_GATE_USD";
pub const KEY_SESSION_HARD: &str = "PERMAGENT_BUDGET_SESSION_HARD_USD";

/// Per-scope thresholds in USD, ascending: soft ≤ gate ≤ hard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetCeilings {
    pub soft: f64,
    pub gate: f64,
    pub hard: f64,
}

impl BudgetCeilings {
    /// Clamp to a sane shape: non-negative and monotonically non-decreasing, so
    /// a misconfigured ceiling (e.g. gate below soft) can never invert the
    /// bands. `gate` is lifted to at least `soft`, `hard` to at least `gate`.
    pub fn sanitized(self) -> Self {
        let soft = self.soft.max(0.0);
        let gate = self.gate.max(soft);
        let hard = self.hard.max(gate);
        Self { soft, gate, hard }
    }
}

/// Task + session ceilings. CONFIGURABLE — the defaults are the only free
/// parameters and are the owner's call to tune.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetConfig {
    pub task: BudgetCeilings,
    pub session: BudgetCeilings,
}

impl Default for BudgetConfig {
    /// Spec defaults: task $2/$5/$10, session $10/$25/$50 (soft/gate/hard).
    fn default() -> Self {
        Self {
            task: BudgetCeilings {
                soft: 2.0,
                gate: 5.0,
                hard: 10.0,
            },
            session: BudgetCeilings {
                soft: 10.0,
                gate: 25.0,
                hard: 50.0,
            },
        }
    }
}

/// Pure constructor: apply any provided overrides over the spec defaults, then
/// sanitize each scope. Split from `load_budget_config` so it is unit-testable
/// without touching the process-global config (mirrors the repo's
/// pure-core-plus-thin-env-wrapper convention).
#[allow(clippy::too_many_arguments)]
pub fn budget_config_from(
    task_soft: Option<f64>,
    task_gate: Option<f64>,
    task_hard: Option<f64>,
    session_soft: Option<f64>,
    session_gate: Option<f64>,
    session_hard: Option<f64>,
) -> BudgetConfig {
    let d = BudgetConfig::default();
    BudgetConfig {
        task: BudgetCeilings {
            soft: task_soft.unwrap_or(d.task.soft),
            gate: task_gate.unwrap_or(d.task.gate),
            hard: task_hard.unwrap_or(d.task.hard),
        }
        .sanitized(),
        session: BudgetCeilings {
            soft: session_soft.unwrap_or(d.session.soft),
            gate: session_gate.unwrap_or(d.session.gate),
            hard: session_hard.unwrap_or(d.session.hard),
        }
        .sanitized(),
    }
}

/// Load the budget config from env / config, falling back to the spec defaults
/// for any unset key. Thin wrapper over `budget_config_from`.
pub fn load_budget_config() -> BudgetConfig {
    let cfg = crate::config::Config::global();
    let read = |key: &str| cfg.get_param::<f64>(key).ok();
    budget_config_from(
        read(KEY_TASK_SOFT),
        read(KEY_TASK_GATE),
        read(KEY_TASK_HARD),
        read(KEY_SESSION_SOFT),
        read(KEY_SESSION_GATE),
        read(KEY_SESSION_HARD),
    )
}

// ── The verdict (pure) ───────────────────────────────────────────────────────

/// Which spend band the running total has entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetBand {
    /// Under the soft threshold — proceed silently.
    Ok,
    /// A non-blocking alert.
    Soft,
    /// Raise the Decision-Inbox gate and pause.
    Gate,
    /// Hard-stop, preserving work.
    Hard,
}

impl BudgetBand {
    fn rank(self) -> u8 {
        match self {
            BudgetBand::Ok => 0,
            BudgetBand::Soft => 1,
            BudgetBand::Gate => 2,
            BudgetBand::Hard => 3,
        }
    }
}

/// Which ceiling drove the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetScope {
    Task,
    Session,
}

impl BudgetScope {
    pub fn word(self) -> &'static str {
        match self {
            BudgetScope::Task => "task",
            BudgetScope::Session => "session",
        }
    }
}

/// The budget check result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetVerdict {
    /// The highest band reached across both scopes.
    pub band: BudgetBand,
    /// The scope that reached it (ties break to `Task`, the tighter ceiling).
    pub scope: BudgetScope,
    /// Spend in that scope, USD.
    pub spent: f64,
    /// The threshold that scope crossed (0.0 when `band == Ok`).
    pub crossed: f64,
    /// Chargeable calls in scope whose model had no published price. Their
    /// dollars ARE in `spent`, billed at the provider's worst known rate, so
    /// `spent` is an upper bound — not the pre-2026-08-24 silent floor.
    pub unpriced_calls: u32,
}

impl BudgetVerdict {
    /// Whether the caller must pause for a human decision before continuing.
    pub fn needs_gate(&self) -> bool {
        self.band == BudgetBand::Gate
    }

    /// Whether the caller must stop now (preserving work).
    pub fn must_stop(&self) -> bool {
        self.band == BudgetBand::Hard
    }
}

/// The band a single scope's spend falls in, plus the threshold it crossed.
fn band_for(spent: f64, ceilings: BudgetCeilings) -> (BudgetBand, f64) {
    let c = ceilings.sanitized();
    if spent >= c.hard {
        (BudgetBand::Hard, c.hard)
    } else if spent >= c.gate {
        (BudgetBand::Gate, c.gate)
    } else if spent >= c.soft {
        (BudgetBand::Soft, c.soft)
    } else {
        (BudgetBand::Ok, 0.0)
    }
}

/// THE budget check — the pure S8 seam. Evaluates BOTH scopes and returns the
/// STRICTER (highest-band) verdict; a tie breaks to `Task` (the tighter, more
/// local ceiling). Thresholds are inclusive (`spent >= threshold`), so the gate
/// fires exactly AT the gate ceiling.
///
/// Spend is a plain `f64` (the caller already has it: `task` spend and the
/// session's `accumulated_cost_usd`). Unpriced chargeable calls are billed at
/// the provider's worst known rate before they reach this figure, so `spent` is
/// an upper bound rather than the old silent floor; pass their count via
/// [`budget_verdict_with_unpriced`] so the user is told which part is estimated.
pub fn budget_verdict(task_spent: f64, session_spent: f64, cfg: &BudgetConfig) -> BudgetVerdict {
    budget_verdict_with_unpriced(task_spent, session_spent, 0, cfg)
}

/// Like [`budget_verdict`], plus the count of chargeable calls whose model had
/// no published price.
///
/// As of 2026-08-24 those calls are no longer invisible to the ceilings:
/// `agents::reply_parts` bills an unpriced chargeable call at the provider's
/// most expensive known rate (see
/// [`crate::providers::canonical::worst_case_pricing`]), so their dollars are
/// ALREADY inside `task_spent` / `session_spent` and drive the bands like any
/// other spend. That is the fail-closed half: unmeasurable spend can make the
/// cap fire early, never late.
///
/// `unpriced_calls` remains for honesty, not for arithmetic. It still lifts an
/// otherwise-`Ok` band to `Soft` so the user is told that some of the figure is
/// an upper-bound estimate rather than a measured cost — the one thing a dollar
/// amount alone cannot say. When it lifts, `crossed` stays 0.0 and the scope is
/// `Task`.
pub fn budget_verdict_with_unpriced(
    task_spent: f64,
    session_spent: f64,
    unpriced_calls: u32,
    cfg: &BudgetConfig,
) -> BudgetVerdict {
    let (task_band, task_thresh) = band_for(task_spent, cfg.task);
    let (session_band, session_thresh) = band_for(session_spent, cfg.session);

    let mut verdict = if session_band.rank() > task_band.rank() {
        BudgetVerdict {
            band: session_band,
            scope: BudgetScope::Session,
            spent: session_spent,
            crossed: session_thresh,
            unpriced_calls,
        }
    } else {
        BudgetVerdict {
            band: task_band,
            scope: BudgetScope::Task,
            spent: task_spent,
            crossed: task_thresh,
            unpriced_calls,
        }
    };

    if unpriced_calls > 0 && verdict.band == BudgetBand::Ok {
        verdict.band = BudgetBand::Soft;
        verdict.crossed = 0.0;
        verdict.scope = BudgetScope::Task;
    }
    verdict
}

// ── The Decision-Inbox gate (near-pure: builds the request; thin async writer) ─

/// Option ids on the spend-gate `choice` — stable identifiers the answer path
/// matches on.
pub const GATE_OPTION_INCREMENT: &str = "increment";
pub const GATE_OPTION_TO_COMPLETION: &str = "to_completion";
pub const GATE_OPTION_STOP: &str = "stop";

/// Build the three-way `choice` payload offered at a spend gate. Pure so the
/// wording and option set are testable without a DB. `stop` is the fail-safe
/// default. Exactly three options — within the Decision Inbox's 2..=8 rule.
pub fn gate_choice_payload(scope: BudgetScope, spent: f64, increment: f64) -> ChoicePayload {
    ChoicePayload {
        question: format!(
            "This {} has spent ${:.2}. How should I proceed?",
            scope.word(),
            spent
        ),
        options: vec![
            ChoiceOption {
                id: GATE_OPTION_INCREMENT.to_string(),
                label: format!("Add ${increment:.2} to the budget and continue"),
            },
            ChoiceOption {
                id: GATE_OPTION_TO_COMPLETION.to_string(),
                label: "Continue to completion (no further gate this run)".to_string(),
            },
            ChoiceOption {
                id: GATE_OPTION_STOP.to_string(),
                label: "Stop now and keep all changes".to_string(),
            },
        ],
        default: Some(GATE_OPTION_STOP.to_string()),
        proposal: None,
    }
}

/// Build the Decision-Inbox request for a spend gate (pure). Separated from the
/// DB write so the headline/detail/payload can be asserted in tests.
pub fn gate_decision_request(
    verdict: BudgetVerdict,
    increment: f64,
    goal_id: Option<String>,
    project_id: Option<String>,
) -> NewDecision {
    let scope = verdict.scope.word();
    // Plain-language outcome headline, <= 80 chars, no technical identifiers.
    let headline = format!("Spent ${:.2} on this {scope} — keep going?", verdict.spent);
    let mut detail = format!(
        "Spend gate reached: ${:.2} spent against the ${:.2} {scope} gate ceiling. \
         'increment' raises the ceiling by ${:.2}; 'to_completion' lifts the gate for \
         the rest of this run; 'stop' halts now and preserves all changes.",
        verdict.spent, verdict.crossed, increment
    );
    if verdict.unpriced_calls > 0 {
        detail.push_str(&format!(
            " This figure is a floor because {} call(s) ran on a model with no published price.",
            verdict.unpriced_calls
        ));
    }
    NewDecision {
        kind: "choice".to_string(),
        goal_id,
        project_id,
        headline: Some(headline),
        detail: Some(detail),
        payload: serde_json::to_value(gate_choice_payload(verdict.scope, verdict.spent, increment))
            .unwrap_or_default(),
        rank: None,
        action_class: None,
    }
}

/// Raise the spend-gate decision in the Decision Inbox and return the created
/// row. A `choice` with no seeded action-class resolves fail-closed to Tier 2,
/// so only the user (never Henry-policy) can answer it. The caller pauses the
/// run until it is answered — the "no surprises" guarantee in code. Impure half
/// of the S8 seam.
pub async fn raise_budget_gate(
    pool: &Pool<Sqlite>,
    verdict: BudgetVerdict,
    increment: f64,
    goal_id: Option<String>,
    project_id: Option<String>,
) -> Result<Decision, String> {
    let req = gate_decision_request(verdict, increment, goal_id, project_id);
    decisions::create_decision(pool, req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    // ── Defaults are the spec's dollar figures ─────────────────────────────

    #[test]
    fn defaults_match_the_spec() {
        let d = BudgetConfig::default();
        approx(d.task.soft, 2.0);
        approx(d.task.gate, 5.0);
        approx(d.task.hard, 10.0);
        approx(d.session.soft, 10.0);
        approx(d.session.gate, 25.0);
        approx(d.session.hard, 50.0);
    }

    // ── Soft / gate / hard fire at the thresholds ──────────────────────────

    #[test]
    fn task_bands_at_the_thresholds() {
        let c = BudgetConfig::default();
        assert_eq!(budget_verdict(0.0, 0.0, &c).band, BudgetBand::Ok);
        assert_eq!(budget_verdict(1.99, 0.0, &c).band, BudgetBand::Ok);
        assert_eq!(budget_verdict(2.0, 0.0, &c).band, BudgetBand::Soft);
        assert_eq!(budget_verdict(4.99, 0.0, &c).band, BudgetBand::Soft);
        // The gate fires exactly AT the gate ceiling.
        assert_eq!(budget_verdict(5.0, 0.0, &c).band, BudgetBand::Gate);
        assert_eq!(budget_verdict(9.99, 0.0, &c).band, BudgetBand::Gate);
        assert_eq!(budget_verdict(10.0, 0.0, &c).band, BudgetBand::Hard);
        assert_eq!(budget_verdict(999.0, 0.0, &c).band, BudgetBand::Hard);
    }

    #[test]
    fn gate_verdict_reports_scope_spend_and_threshold() {
        let c = BudgetConfig::default();
        let v = budget_verdict(5.0, 0.0, &c);
        assert!(v.needs_gate());
        assert!(!v.must_stop());
        assert_eq!(v.scope, BudgetScope::Task);
        approx(v.spent, 5.0);
        approx(v.crossed, 5.0);
    }

    #[test]
    fn hard_verdict_says_stop() {
        let c = BudgetConfig::default();
        let v = budget_verdict(0.0, 50.0, &c);
        assert!(v.must_stop());
        assert_eq!(v.band, BudgetBand::Hard);
        assert_eq!(v.scope, BudgetScope::Session);
    }

    // ── Two scopes: the stricter wins ──────────────────────────────────────

    #[test]
    fn session_can_gate_while_task_is_fine() {
        let c = BudgetConfig::default();
        // Task ok ($1), session at its gate ($25) → Gate on the session.
        let v = budget_verdict(1.0, 25.0, &c);
        assert_eq!(v.band, BudgetBand::Gate);
        assert_eq!(v.scope, BudgetScope::Session);
    }

    #[test]
    fn stricter_scope_wins_and_ties_break_to_task() {
        let c = BudgetConfig::default();
        // Task hard ($10) beats session soft ($10) → Hard/Task.
        let v = budget_verdict(10.0, 10.0, &c);
        assert_eq!(v.band, BudgetBand::Hard);
        assert_eq!(v.scope, BudgetScope::Task);
        // Both at Soft → tie breaks to Task.
        let v2 = budget_verdict(2.0, 10.0, &c);
        assert_eq!(v2.band, BudgetBand::Soft);
        assert_eq!(v2.scope, BudgetScope::Task);
    }

    // ── Config overrides + sanitation ──────────────────────────────────────

    #[test]
    fn overrides_apply_over_defaults() {
        let c = budget_config_from(Some(1.0), Some(3.0), Some(7.0), None, None, None);
        approx(c.task.soft, 1.0);
        approx(c.task.gate, 3.0);
        approx(c.task.hard, 7.0);
        // Session untouched → spec defaults.
        approx(c.session.gate, 25.0);
    }

    #[test]
    fn inverted_or_negative_ceilings_are_sanitized() {
        // gate below soft, hard below gate, a negative soft → clamped monotonic.
        let c = budget_config_from(Some(-1.0), Some(1.0), Some(0.5), None, None, None);
        approx(c.task.soft, 0.0);
        approx(c.task.gate, 1.0);
        approx(c.task.hard, 1.0); // lifted up to gate
                                  // A sanitized config never inverts a verdict.
        let v = budget_verdict(1.0, 0.0, &c);
        assert!(matches!(v.band, BudgetBand::Gate | BudgetBand::Hard));
    }

    // ── The Decision-Inbox gate request ────────────────────────────────────

    #[test]
    fn gate_choice_has_the_three_spec_options() {
        let p = gate_choice_payload(BudgetScope::Session, 25.0, 10.0);
        assert_eq!(p.options.len(), 3);
        let ids: Vec<&str> = p.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                GATE_OPTION_INCREMENT,
                GATE_OPTION_TO_COMPLETION,
                GATE_OPTION_STOP
            ]
        );
        assert_eq!(p.default.as_deref(), Some(GATE_OPTION_STOP));
        // Within the Decision Inbox's 2..=8 rule.
        assert!((2..=8).contains(&p.options.len()));
    }

    #[test]
    fn gate_request_is_a_valid_choice_with_a_short_headline() {
        let c = BudgetConfig::default();
        let v = budget_verdict(0.0, 25.0, &c); // session gate
        let req = gate_decision_request(v, 10.0, Some("goal-1".into()), None);
        assert_eq!(req.kind, "choice");
        assert_eq!(req.goal_id.as_deref(), Some("goal-1"));
        let headline = req.headline.as_deref().unwrap();
        // The Decision Inbox caps headlines at 80 chars.
        assert!(
            headline.chars().count() <= 80,
            "headline too long: {headline:?}"
        );
        assert!(headline.contains("session"));
        // Payload round-trips as a ChoicePayload with 3 options.
        let payload: ChoicePayload = serde_json::from_value(req.payload).unwrap();
        assert_eq!(payload.options.len(), 3);
    }

    // ── Unpriced calls: floor semantics, never fabricate Gate/Hard ─────────

    #[test]
    fn unpriced_calls_lift_ok_to_soft_and_never_past_it() {
        let c = BudgetConfig::default();
        // Zero spend + unpriced → Soft alert (visible floor), not Ok.
        let v = budget_verdict_with_unpriced(0.0, 0.0, 3, &c);
        assert_eq!(v.band, BudgetBand::Soft);
        assert_eq!(v.scope, BudgetScope::Task);
        approx(v.crossed, 0.0);
        assert_eq!(v.unpriced_calls, 3);

        // Soft / Gate / Hard bands are unchanged by unpriced calls.
        let soft = budget_verdict_with_unpriced(2.0, 0.0, 5, &c);
        assert_eq!(soft.band, BudgetBand::Soft);
        let gate = budget_verdict_with_unpriced(5.0, 0.0, 5, &c);
        assert_eq!(gate.band, BudgetBand::Gate);
        let hard = budget_verdict_with_unpriced(10.0, 0.0, 5, &c);
        assert_eq!(hard.band, BudgetBand::Hard);
    }

    #[test]
    fn gate_detail_names_unpriced_calls_only_when_present() {
        let c = BudgetConfig::default();
        let priced = budget_verdict(0.0, 25.0, &c);
        let req = gate_decision_request(priced, 10.0, None, None);
        let detail = req.detail.as_deref().unwrap();
        assert!(
            !detail.contains("floor") && !detail.contains("no published price"),
            "priced gate detail must not mention unpriced calls, got: {detail}"
        );

        let unpriced = budget_verdict_with_unpriced(0.0, 25.0, 2, &c);
        let req = gate_decision_request(unpriced, 10.0, None, None);
        let detail = req.detail.as_deref().unwrap();
        assert!(
            detail.contains("floor") && detail.contains("2 call(s)"),
            "unpriced gate detail must name the floor and call count, got: {detail}"
        );
        // Headline stays short (Decision Inbox 80-char cap).
        assert!(req.headline.as_deref().unwrap().chars().count() <= 80);
    }
}
