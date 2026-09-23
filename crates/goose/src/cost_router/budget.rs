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
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
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
        // Preserve invalid numbers so authorization callers can fail closed.
        // `f64::max` would otherwise turn some NaN configurations into a
        // plausible zero/finite ceiling and accidentally authorize work.
        if !self.soft.is_finite() || !self.gate.is_finite() || !self.hard.is_finite() {
            return self;
        }
        let soft = self.soft.max(0.0);
        let gate = self.gate.max(soft);
        let hard = self.hard.max(gate);
        Self { soft, gate, hard }
    }
}

/// Task + session ceilings. CONFIGURABLE — the defaults are the only free
/// parameters and are the owner's call to tune.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BudgetConfig {
    pub task: BudgetCeilings,
    pub session: BudgetCeilings,
}

impl Default for BudgetConfig {
    /// Task $1/$3/$6, session $5/$12/$25 (soft/gate/hard).
    ///
    /// These were task $2/$5/$10 and session $10/$25/$50. The user, on seeing a
    /// status line offering to spend $47.91 more on one session: "It would be
    /// completely unacceptable to spend $50 on a task. that is absurd."
    ///
    /// He is right, and the old numbers were indefensible for this product
    /// specifically. The harness exists to be cost-optimized — it routes to
    /// local models, prices every call, and reports spend per turn. A default
    /// that silently permits fifty dollars before it hard-stops is not a cap,
    /// it is a formality: the gate at $25 arrives long after the money is
    /// gone. A default should be the number that would not upset someone who
    /// never opened the setting.
    ///
    /// The ladder keeps its shape (soft alerts, gate pauses for a decision,
    /// hard stops) so nothing downstream changes; only the ceilings move. A
    /// long deliberate run raises them explicitly, which is the point — paying
    /// more should be a thing you chose, through
    /// `PERMAGENT_BUDGET_TASK_*_USD` / `PERMAGENT_BUDGET_SESSION_*_USD`.
    ///
    /// A first cut at $0.50/$1/$2 and $2/$5/$10 deadlocked the harness
    /// immediately: "task budget gate required before provider call (spent
    /// $0.12, authorization holds $0.00, requested bound $1.56, ceiling
    /// $1.00)". The gate is compared against a RESERVATION BOUND, and
    /// `calculate_bound` prices the model's whole context window
    /// (`input_tokens = context - output`) no matter how small the actual
    /// prompt is — that call sent 30k tokens, 7% of context, and cost $0.12,
    /// while reserving $1.56. So any ceiling under a single call's worst case
    /// blocks every call, and the tighter the cap the more certainly nothing
    /// runs at all.
    ///
    /// These numbers clear one conservative bound with room for a second, and
    /// remain far below where this started (session hard $50 → $25, task hard
    /// $10 → $6). The right long-term fix is to reserve against the actual
    /// prompt size rather than the full window, which would let the ceilings
    /// come down again without deadlocking.
    fn default() -> Self {
        Self {
            task: BudgetCeilings {
                soft: 1.0,
                gate: 3.0,
                hard: 6.0,
            },
            session: BudgetCeilings {
                soft: 5.0,
                gate: 12.0,
                hard: 25.0,
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
        check_approval: None,
        roadmap_approval: None,
        // This is the historical SETTLED-spend gate, not the reservation
        // gate `reservation_gate_decision_request` builds below — leaving
        // this `None` is what keeps `auto_approve_reservation_gate` from
        // ever matching it, even though the option ids happen to be
        // identical.
        reservation_gate: None,
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

/// Build a spend gate for a reservation crossing. Unlike the historical
/// settled-spend gate, this wording keeps settled dollars, active holds, and
/// the requested worst-case bound separate; an authorization hold is never
/// misreported as money already spent.
// Every argument is a distinct piece of settlement evidence; collapsing them
// into a struct would only move the same fields behind one more name.
#[allow(clippy::too_many_arguments)]
pub fn reservation_gate_decision_request(
    scope: BudgetScope,
    settled_usd: f64,
    held_usd: f64,
    requested_usd: f64,
    ceiling_usd: f64,
    increment: f64,
    goal_id: Option<String>,
    project_id: Option<String>,
) -> NewDecision {
    let headline = format!("{} budget authorization needs approval", scope.word());
    let detail = format!(
        "Provider authorization is paused: settled spend is ${settled_usd:.2}, active authorization holds are ${held_usd:.2}, and this request's worst-case bound is ${requested_usd:.2} against the ${ceiling_usd:.2} {} ceiling. The bound is not recorded as spent. 'increment' raises the ceiling by ${increment:.2}; 'to_completion' lifts the gate for this run; 'stop' halts now.",
        scope.word(),
    );
    NewDecision {
        kind: "choice".to_string(),
        goal_id,
        project_id,
        headline: Some(headline),
        detail: Some(detail),
        payload: serde_json::to_value(ChoicePayload {
            question: format!(
                "Approve this {} provider authorization? Settled ${settled_usd:.2}; holds ${held_usd:.2}; bound ${requested_usd:.2}; ceiling ${ceiling_usd:.2}.",
                scope.word()
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
            check_approval: None,
            roadmap_approval: None,
            // R2 (harness loop, Lane D1): the typed settlement evidence the
            // headless auto-answer path (`decisions::auto_approve_reservation_gate`)
            // reads instead of re-parsing `detail`'s prose, and the marker
            // that shape-scopes that auto-answer to EXACTLY this kind of
            // gate — never an arbitrary `choice` decision.
            reservation_gate: Some(decisions::ReservationGatePayload {
                scope: scope.word().to_string(),
                settled_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                requesting_session_id: None,
            }),
        })
        .unwrap_or_default(),
        ..Default::default()
    }
}

/// I1 (harness loop, 2026-09-10, from M10's forensics): stamp
/// [`decisions::ReservationGatePayload::requesting_session_id`] onto a
/// request built by [`reservation_gate_decision_request`] — used only for a
/// session-scoped gate (a coordinator's own provider call, which carries no
/// `goal_id`), so [`decisions::find_open_reservation_gate_decision_for_session`]
/// has something to dedupe on. Silently returns `request` unchanged if its
/// payload isn't shaped like a reservation-gate `choice` decision —
/// defensive, never a panic on a caller's own construction.
pub fn with_requesting_session_id(mut request: NewDecision, session_id: &str) -> NewDecision {
    let Ok(mut payload) = serde_json::from_value::<ChoicePayload>(request.payload.clone()) else {
        return request;
    };
    let Some(gate) = payload.reservation_gate.as_mut() else {
        return request;
    };
    gate.requesting_session_id = Some(session_id.to_string());
    if let Ok(v) = serde_json::to_value(payload) {
        request.payload = v;
    }
    request
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

// ── Unknown-hold sweep ───────────────────────────────────────────────────────

/// Historical grace-window constant kept for compatibility with existing
/// metrics/tests. Time no longer authorizes releasing unknown holds.
pub const UNKNOWN_HOLD_GRACE_HOURS: i64 = 6;

/// How often the daemon re-runs [`reconcile_concluded_unknown_holds`] after
/// its boot pass.
pub const UNKNOWN_HOLD_SWEEP_INTERVAL_SECS: u64 = 15 * 60;

/// What one sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UnknownHoldSweep {
    /// Expired `pending` holds moved to `unknown`.
    pub expired_to_unknown: u64,
    /// `unknown` holds settled to the spend their `cost_ledger` row records.
    pub settled: u64,
    /// `unknown` holds released because no ledger row shows they were billed.
    pub released: u64,
    /// Unknown holds still retained (no durable settle evidence yet).
    pub still_held: u64,
}

/// Resolve every `unknown` provider-spend hold that has durable usage
/// evidence, across ALL sessions, without waiting for a new call.
///
/// Why this exists (E2E QA, 2026-09-18, row
/// `invocation:e1d47602-5717-47b9-9603-0b86911b4d6f`, session 20260918_7): an
/// openai stream dropped before any reply started, the call marked its own
/// $1.56 hold `unknown` (`unknown_marked_at` set, no ledger row), and after
/// the daemon was redeployed with the session-tree reconcile fix the row was
/// STILL `unknown` with $1.56 held. The only code that ever resolved an
/// `unknown` hold ran inside `reserve_provider_invocation`, i.e. on the next
/// paid call in that chat, so until the user happened to send another turn
/// the hold kept counting against the session and task budgets (and the
/// Budget view kept showing money nobody spent). A hold is a claim on
/// headroom for a call in flight; once the call is known to be over, keeping
/// it depends on nothing but whether someone speaks again.
///
/// The rules are exactly the admission path's, so this never releases
/// anything that path would keep:
/// - an expired `pending` hold becomes `unknown` (the provider may still have
///   accepted it), with `unknown_marked_at` left NULL;
/// - an `unknown` hold is settled only when a `cost_ledger` row for its
///   `invocation_id` exists;
/// - absent that durable usage record, it remains `unknown` (time alone is
///   not evidence of no provider charge).
///
/// Each resolution is guarded by `state = 'unknown'`, so it races safely with
/// the admission path doing the same work for the same row.
pub async fn reconcile_concluded_unknown_holds(
    pool: &Pool<Sqlite>,
) -> Result<UnknownHoldSweep, sqlx::Error> {
    reconcile_concluded_unknown_holds_at(pool, chrono::Utc::now()).await
}

/// [`reconcile_concluded_unknown_holds`] at a fixed instant, for tests.
pub async fn reconcile_concluded_unknown_holds_at(
    pool: &Pool<Sqlite>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<UnknownHoldSweep, sqlx::Error> {
    let now_str = now.to_rfc3339();
    // Same statement the admission path runs first (the lease strings are
    // written by `to_rfc3339`, so they compare in time order as text).
    let expired_to_unknown = sqlx::query(
        "UPDATE cost_reservations SET state = 'unknown', updated_at = ?
         WHERE state = 'pending' AND lease_until <= ?",
    )
    .bind(&now_str)
    .bind(&now_str)
    .execute(pool)
    .await?
    .rows_affected();
    let mut sweep = UnknownHoldSweep {
        expired_to_unknown,
        ..UnknownHoldSweep::default()
    };

    let stale: Vec<(String, String)> = sqlx::query_as(
        "SELECT reservation_id, invocation_id
         FROM cost_reservations WHERE state = 'unknown'",
    )
    .fetch_all(pool)
    .await?;

    for (reservation_id, invocation_id) in stale {
        let mut tx = pool.begin().await?;
        let ledger_cost: Option<f64> =
            sqlx::query_scalar("SELECT cost_usd FROM cost_ledger WHERE call_id = ?")
                .bind(&invocation_id)
                .fetch_optional(&mut *tx)
                .await?;
        let resolved = match ledger_cost {
            Some(cost_usd) => {
                let n = sqlx::query(
                    "UPDATE cost_reservations
                     SET state = 'settled', settled_cost_usd = ?, updated_at = ?
                     WHERE reservation_id = ? AND state = 'unknown'",
                )
                .bind(cost_usd)
                .bind(&now_str)
                .bind(&reservation_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                sweep.settled += n;
                n
            }
            None => {
                sweep.still_held += 1;
                0
            }
        };
        tx.commit().await?;
        if resolved > 0 {
            tracing::info!(
                target: "permagent::cost_router",
                reservation_id = %reservation_id,
                invocation_id = %invocation_id,
                settled_from_ledger = ledger_cost.is_some(),
                "sweep resolved an unknown provider-spend hold without waiting for a new call"
            );
        }
    }
    Ok(sweep)
}

/// Run [`reconcile_concluded_unknown_holds`] once now and then every
/// [`UNKNOWN_HOLD_SWEEP_INTERVAL_SECS`], for the life of the daemon. The boot
/// pass is what clears a hold a redeploy left behind (the E2E row above); the
/// interval settles holds as ledger evidence arrives. Failures are logged and
/// retried on the next tick.
pub fn spawn_unknown_hold_sweeper(pool: Pool<Sqlite>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            UNKNOWN_HOLD_SWEEP_INTERVAL_SECS,
        ));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // The first tick completes immediately: that is the boot pass.
            tick.tick().await;
            match reconcile_concluded_unknown_holds(&pool).await {
                Ok(sweep) if sweep.settled + sweep.released + sweep.expired_to_unknown > 0 => {
                    tracing::info!(
                        target: "permagent::cost_router",
                        expired_to_unknown = sweep.expired_to_unknown,
                        settled = sweep.settled,
                        released = sweep.released,
                        still_held = sweep.still_held,
                        "unknown provider-spend hold sweep"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    target: "permagent::cost_router",
                    error = %e,
                    "unknown provider-spend hold sweep failed; will retry"
                ),
            }
        }
    })
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
        approx(d.task.soft, 1.0);
        approx(d.task.gate, 3.0);
        approx(d.task.hard, 6.0);
        approx(d.session.soft, 5.0);
        approx(d.session.gate, 12.0);
        approx(d.session.hard, 25.0);
    }

    #[test]
    fn reservation_gate_distinguishes_holds_from_settled_spend() {
        let d = reservation_gate_decision_request(
            BudgetScope::Task,
            1.25,
            2.50,
            3.75,
            5.00,
            5.00,
            Some("goal-1".to_string()),
            None,
        );
        assert!(d
            .detail
            .as_deref()
            .unwrap()
            .contains("settled spend is $1.25"));
        assert!(d
            .detail
            .as_deref()
            .unwrap()
            .contains("authorization holds are $2.50"));
        assert!(d
            .detail
            .as_deref()
            .unwrap()
            .contains("worst-case bound is $3.75"));
        assert!(!d.detail.as_deref().unwrap().contains("$3.75 spent"));
    }

    /// An explicit ladder for the band tests.
    ///
    /// These tests are about where the bands fire, not about what the product
    /// ships as a default — they only ever used `default()` because the two
    /// happened to coincide. When the defaults were lowered (task $2/$5/$10 →
    /// $0.50/$1/$2) six of them failed despite the band logic being untouched,
    /// which is a test telling you about the wrong thing. `defaults_match_the_spec`
    /// is the one place that pins the shipped numbers.
    fn ladder() -> BudgetConfig {
        BudgetConfig {
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

    // ── Soft / gate / hard fire at the thresholds ──────────────────────────

    #[test]
    fn task_bands_at_the_thresholds() {
        let c = ladder();
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
        let c = ladder();
        let v = budget_verdict(5.0, 0.0, &c);
        assert!(v.needs_gate());
        assert!(!v.must_stop());
        assert_eq!(v.scope, BudgetScope::Task);
        approx(v.spent, 5.0);
        approx(v.crossed, 5.0);
    }

    #[test]
    fn hard_verdict_says_stop() {
        let c = ladder();
        let v = budget_verdict(0.0, 50.0, &c);
        assert!(v.must_stop());
        assert_eq!(v.band, BudgetBand::Hard);
        assert_eq!(v.scope, BudgetScope::Session);
    }

    // ── Two scopes: the stricter wins ──────────────────────────────────────

    #[test]
    fn session_can_gate_while_task_is_fine() {
        let c = ladder();
        // Task ok ($1), session at its gate ($25) → Gate on the session.
        let v = budget_verdict(1.0, 25.0, &c);
        assert_eq!(v.band, BudgetBand::Gate);
        assert_eq!(v.scope, BudgetScope::Session);
    }

    #[test]
    fn stricter_scope_wins_and_ties_break_to_task() {
        let c = ladder();
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
        approx(c.session.gate, BudgetConfig::default().session.gate);
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

    #[test]
    fn non_finite_ceilings_remain_invalid_for_fail_closed_callers() {
        let c = budget_config_from(Some(f64::NAN), None, None, None, None, None);
        assert!(c.task.soft.is_nan());

        let c = budget_config_from(None, Some(f64::INFINITY), None, None, None, None);
        assert!(c.task.gate.is_infinite());
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
        let c = ladder();
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
        let c = ladder();
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
        let c = ladder();
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

#[cfg(test)]
mod unknown_hold_sweep_tests {
    use super::*;
    use chrono::{DateTime, Duration, TimeZone, Utc};

    // One connection: every `sqlite::memory:` connection is its own database.
    async fn pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // The ledger schema adds its rollup columns to `sessions`, so the
        // base schema has to exist first, exactly as it does in the daemon.
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_cost_ledger_schema(&pool)
            .await
            .unwrap();
        crate::session::spectral_schema::apply_cost_reservations_schema(&pool)
            .await
            .unwrap();
        pool
    }

    /// 2026-09-18T22:30Z: after the E2E row's 20:07Z lease, inside the 6h
    /// grace a lease-only hold would still get.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 18, 22, 30, 0).unwrap()
    }

    async fn hold(
        pool: &Pool<Sqlite>,
        invocation: &str,
        state: &str,
        amount: f64,
        lease_until: DateTime<Utc>,
        marked: bool,
    ) {
        sqlx::query(
            "INSERT INTO cost_reservations
                 (reservation_id, invocation_id, session_id, task_id, amount_usd,
                  state, lease_until, created_at, updated_at, unknown_marked_at)
             VALUES (?, ?, '20260918_7', 'task-1', ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("invocation:{invocation}"))
        .bind(invocation)
        .bind(amount)
        .bind(state)
        .bind(lease_until.to_rfc3339())
        .bind((lease_until - Duration::hours(2)).to_rfc3339())
        .bind((lease_until - Duration::hours(2)).to_rfc3339())
        .bind(marked.then(|| "2026-09-18 18:07:00".to_string()))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn state_of(pool: &Pool<Sqlite>, invocation: &str) -> (String, Option<f64>) {
        sqlx::query_as(
            "SELECT state, settled_cost_usd FROM cost_reservations WHERE invocation_id = ?",
        )
        .bind(invocation)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// What the Budget view and the admission check both count as held.
    async fn held_usd(pool: &Pool<Sqlite>) -> f64 {
        sqlx::query_scalar(
            "SELECT CAST(COALESCE(SUM(amount_usd), 0.0) AS REAL) FROM cost_reservations
             WHERE session_id = '20260918_7' AND state IN ('pending', 'unknown')",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The E2E row: marked `unknown` by the call that dropped (no ledger row),
    /// lease expired 20:07Z, $1.56 held. The sweep keeps holding it because
    /// there is no durable evidence either way yet.
    #[tokio::test]
    async fn the_reported_concluded_hold_is_retained_without_a_new_turn() {
        let pool = pool().await;
        let lease = Utc.with_ymd_and_hms(2026, 9, 18, 20, 7, 0).unwrap();
        let id = "e1d47602-5717-47b9-9603-0b86911b4d6f";
        hold(&pool, id, "unknown", 1.56, lease, true).await;
        assert!((held_usd(&pool).await - 1.56).abs() < 1e-9);

        let sweep = reconcile_concluded_unknown_holds_at(&pool, now())
            .await
            .unwrap();

        assert_eq!(sweep.released, 0);
        assert_eq!(sweep.settled, 0);
        assert_eq!(sweep.still_held, 1);
        assert_eq!(state_of(&pool, id).await, ("unknown".to_string(), None));
        assert!((held_usd(&pool).await - 1.56).abs() < 1e-9);

        // Idempotent: a second pass finds nothing to do.
        let again = reconcile_concluded_unknown_holds_at(&pool, now())
            .await
            .unwrap();
        assert_eq!(again.expired_to_unknown, 0);
        assert_eq!(again.settled, 0);
        assert_eq!(again.released, 0);
        assert_eq!(again.still_held, 1);
    }

    /// A concluded hold whose call DID bill is settled to the ledger's real
    /// spend, never dropped: that spend keeps counting toward the ceilings.
    #[tokio::test]
    async fn a_concluded_hold_with_a_ledger_row_settles_to_the_recorded_spend() {
        let pool = pool().await;
        hold(
            &pool,
            "billed",
            "unknown",
            1.56,
            now() - Duration::hours(1),
            true,
        )
        .await;
        sqlx::query(
            "INSERT INTO cost_ledger (call_id, ts, session_id, task_id, cost_usd)
             VALUES ('billed', ?, '20260918_7', 'task-1', 0.12)",
        )
        .bind(now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let sweep = reconcile_concluded_unknown_holds_at(&pool, now())
            .await
            .unwrap();

        assert_eq!(sweep.settled, 1);
        assert_eq!(sweep.released, 0);
        assert_eq!(
            state_of(&pool, "billed").await,
            ("settled".to_string(), Some(0.12))
        );
    }

    /// A hold that is `unknown` only because its lease ran out still may have
    /// billable usage; elapsed time alone never converts it to released.
    #[tokio::test]
    async fn a_lease_only_hold_is_retained_even_after_grace_window() {
        let pool = pool().await;
        let lease = now() - Duration::hours(1);
        hold(&pool, "maybe-live", "unknown", 0.61, lease, false).await;

        let early = reconcile_concluded_unknown_holds_at(&pool, now())
            .await
            .unwrap();
        assert_eq!(early.still_held, 1);
        assert_eq!(early.released, 0);
        assert_eq!(state_of(&pool, "maybe-live").await.0, "unknown");

        let late = reconcile_concluded_unknown_holds_at(
            &pool,
            lease + Duration::hours(UNKNOWN_HOLD_GRACE_HOURS),
        )
        .await
        .unwrap();
        assert_eq!(late.released, 0);
        assert_eq!(late.still_held, 1);
        assert_eq!(state_of(&pool, "maybe-live").await.0, "unknown");
    }

    /// An expired `pending` hold goes to lease-only `unknown` (held, not
    /// released); a live `pending` hold and already-final rows are untouched.
    #[tokio::test]
    async fn expired_pending_becomes_held_unknown_and_live_rows_are_untouched() {
        let pool = pool().await;
        hold(
            &pool,
            "expired",
            "pending",
            0.61,
            now() - Duration::minutes(5),
            false,
        )
        .await;
        hold(
            &pool,
            "live",
            "pending",
            0.61,
            now() + Duration::hours(1),
            false,
        )
        .await;
        hold(
            &pool,
            "done",
            "settled",
            0.61,
            now() - Duration::hours(9),
            false,
        )
        .await;
        hold(
            &pool,
            "freed",
            "released",
            0.61,
            now() - Duration::hours(9),
            true,
        )
        .await;

        let sweep = reconcile_concluded_unknown_holds_at(&pool, now())
            .await
            .unwrap();

        assert_eq!(sweep.expired_to_unknown, 1);
        assert_eq!(sweep.still_held, 1);
        assert_eq!(sweep.released, 0);
        assert_eq!(state_of(&pool, "expired").await.0, "unknown");
        assert_eq!(state_of(&pool, "live").await.0, "pending");
        assert_eq!(state_of(&pool, "done").await.0, "settled");
        assert_eq!(state_of(&pool, "freed").await.0, "released");
    }
}
