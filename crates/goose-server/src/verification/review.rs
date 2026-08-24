//! The independent cross-family review — WIRED.
//!
//! [`permagent::cost_router::review_gate`] has held the decision core since it
//! was written: the five lenses, the strict parse, the default-to-reject
//! [`gate_decision`]. Nothing called it. This module is the call.
//!
//! It runs at the END of goal verification, AFTER the typed checks and the local
//! rubric verifier have already said pass — because a review is only worth
//! paying for on work that survived the machine checks, and because the one
//! failure the machine checks structurally cannot see (tests weakened to make
//! them pass) is exactly what the reviewer is asked to look for.
//!
//! Three properties are load-bearing:
//!
//! - **Different family.** [`permagent::cost_router::select_reviewer`] chooses a
//!   model from a family other than the worker's. When it cannot, the review is
//!   UNAVAILABLE — which becomes Uncertain-and-parked, never a Pass.
//! - **Default to reject.** Every ambiguous outcome — an unreadable verdict, an
//!   un-evidenced approval, a refused spend, a transport failure — blocks. The
//!   only path to Pass is an APPROVE that stated what it checked.
//! - **A reject is actionable.** The grounded lens findings become the corrective
//!   plan the next attempt is dispatched with (`last_check_output`, which
//!   `retry_context_block` reads back into the brief). The same finding surviving
//!   [`REVIEW_ESCALATE_AT`] rounds stops looping and goes to the Decision Inbox.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use permagent::cost_router::{
    build_review_prompt, build_rubric_prompt, gate_decision, parse_review, review_required,
    reviewer_spend_gate, select_reviewer, GateDecision, ReviewFinding, ReviewLens, ReviewOutcome,
    ReviewVerdict, ReviewerPick, ReviewerSelection, SpendDecision, REVIEW_ESCALATE_AT,
    REVIEW_RUBRIC_SYSTEM_PROMPT, REVIEW_SYSTEM_PROMPT,
};

/// Where the review lands on the goal card: inside the verdict record the
/// verifier already writes, so this costs no extra metadata write.
pub const REVIEW_RECORD_KEY: &str = "independent_review";

/// The per-project / per-goal knob. Absent ⇒ ON (the default): a goal that
/// finishes without an independent review is exactly what this gate exists to
/// prevent, so opting OUT has to be deliberate.
pub const REVIEW_GATE_KEY: &str = "independent_review";

/// Hard cap on the diff text handed to the reviewer. A review that has to read
/// half a megabyte is neither cheap nor careful; past this the diff is truncated
/// and the reviewer is TOLD it was, so it can return UNCERTAIN rather than
/// approve what it never saw.
pub const MAX_DIFF_CHARS: usize = 60_000;

/// Hard cap on the artifact text handed to the rubric reviewer.
pub const MAX_ARTIFACT_CHARS: usize = 60_000;

/// Rough token estimate used only for the recorded cost estimate: ~4 chars/token.
const CHARS_PER_TOKEN: usize = 4;

/// Output tokens one review costs — the verdict block is a handful of lines.
const REVIEW_OUTPUT_TOKENS: u64 = 400;

// ── The persisted record ─────────────────────────────────────────────────────

/// Which rubric the reviewer applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    /// The five code lenses over a diff.
    Code,
    /// The four prose lenses over an artifact, graded against the goal's own
    /// acceptance criteria. This is what a `prose`/`docs`/`research` goal gets —
    /// until now those finished unjudged.
    Rubric,
}

impl ReviewMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewMode::Code => "code",
            ReviewMode::Rubric => "rubric",
        }
    }

    fn lenses(&self) -> Vec<String> {
        match self {
            ReviewMode::Code => ReviewLens::code()
                .iter()
                .map(|l| l.as_str().to_string())
                .collect(),
            ReviewMode::Rubric => ReviewLens::prose()
                .iter()
                .map(|l| l.as_str().to_string())
                .collect(),
        }
    }

    fn system_prompt(&self) -> &'static str {
        match self {
            ReviewMode::Code => REVIEW_SYSTEM_PROMPT,
            ReviewMode::Rubric => REVIEW_RUBRIC_SYSTEM_PROMPT,
        }
    }
}

/// What the gate did. Distinct from the reviewer's own verdict: the reviewer
/// says APPROVE, deterministic Rust says whether that is allowed to mean done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    /// Reviewed and approved with evidence — the Pass stands.
    Passed,
    /// Concrete findings — the goal goes back with them as the corrective plan.
    Rejected,
    /// Default-to-reject: could not sign off. Not a completion.
    Blocked,
    /// The same review recurred until the loop had to stop — to the human.
    Escalated,
    /// The review could not run at all (no cross-family model, spend refused,
    /// transport failure). Uncertain and parked — never a Pass.
    Unavailable,
    /// The trigger said this diff does not warrant a paid review (pure
    /// test/doc/format, or a few lines).
    Skipped,
    /// Turned off for this project or goal.
    Disabled,
}

impl ReviewDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewDecision::Passed => "passed",
            ReviewDecision::Rejected => "rejected",
            ReviewDecision::Blocked => "blocked",
            ReviewDecision::Escalated => "escalated",
            ReviewDecision::Unavailable => "unavailable",
            ReviewDecision::Skipped => "skipped",
            ReviewDecision::Disabled => "disabled",
        }
    }

    /// Whether the verified PASS may stand. Only two outcomes let work finish:
    /// a real approval, and a review that was deliberately not required.
    pub fn allows_completion(&self) -> bool {
        matches!(
            self,
            ReviewDecision::Passed | ReviewDecision::Skipped | ReviewDecision::Disabled
        )
    }

    /// Whether this outcome hands the goal to a person rather than to a retry.
    pub fn parks(&self) -> bool {
        matches!(
            self,
            ReviewDecision::Blocked | ReviewDecision::Escalated | ReviewDecision::Unavailable
        )
    }
}

/// The review, as written onto the goal card. Everything the UI needs to say WHO
/// reviewed, through WHICH lenses, and WHY it landed where it did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndependentReview {
    pub version: u32,
    pub mode: ReviewMode,
    pub decision: ReviewDecision,
    /// The reviewer's own verdict token, when one was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<ReviewVerdict>,
    /// The reviewer's one-line statement of what it examined.
    #[serde(default)]
    pub checked: String,
    /// Grounded findings only — anything citing no concrete evidence was dropped
    /// before it got here, so nothing loops the harness on vapor.
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    /// The lens set applied, named so the card can list them.
    #[serde(default)]
    pub lenses: Vec<String>,
    /// Who reviewed. `None` when no reviewer could be chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<ReviewerPick>,
    /// The property the whole gate rests on, recorded rather than assumed.
    pub cross_family: bool,
    /// Estimated USD for this one review. `None` means unpriced — which is also
    /// why it was refused, never a quiet $0.00.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Why the review could not run or could not sign off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// A zero-finding approval on a substantive change: allowed, but recorded so
    /// a lazy sign-off is auditable.
    #[serde(default)]
    pub rubber_stamp: bool,
    /// Stable fingerprint of this outcome, so an identical re-review is
    /// recognizable across rounds.
    #[serde(default)]
    pub fingerprint: String,
    /// How many CONSECUTIVE rounds have produced this same fingerprint.
    #[serde(default)]
    pub consecutive_identical: u32,
    /// One plain sentence for the summary layer of the evidence digest.
    pub summary: String,
    pub reviewed_at: String,
}

impl IndependentReview {
    fn base(mode: ReviewMode, decision: ReviewDecision, summary: String) -> Self {
        Self {
            version: 1,
            mode,
            decision,
            verdict: None,
            checked: String::new(),
            findings: Vec::new(),
            lenses: mode.lenses(),
            reviewer: None,
            cross_family: false,
            estimated_cost_usd: None,
            reason: None,
            rubber_stamp: false,
            fingerprint: String::new(),
            consecutive_identical: 0,
            summary,
            reviewed_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Read the previous round's review off the goal's metadata, so the
    /// repeated-finding cap can count. Lives inside the verdict record the
    /// verifier already writes — no second metadata key, no second write.
    pub fn from_metadata(meta: &serde_json::Map<String, Value>) -> Option<Self> {
        meta.get("dispatch_evidence")?
            .get("verdict")?
            .get(REVIEW_RECORD_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// The corrective plan handed to the next attempt: the lens findings, each
    /// with its concrete evidence, in the shape `retry_context_block` renders.
    pub fn corrective_plan(&self) -> String {
        if self.findings.is_empty() {
            return self
                .reason
                .clone()
                .unwrap_or_else(|| "The independent reviewer did not approve this work.".into());
        }
        let mut out = String::from(
            "An independent reviewer from a different model family read this work after the \
             automated checks passed, and did NOT approve it. Fix each finding below, then \
             finish again. Every finding cites the exact place it was found.\n",
        );
        for f in &self.findings {
            out.push_str(&format!(
                "\n[{}] {}\n    evidence: {}\n",
                f.lens.as_str(),
                f.summary,
                if f.evidence.trim().is_empty() {
                    "—"
                } else {
                    f.evidence.trim()
                }
            ));
        }
        if let Some(pick) = &self.reviewer {
            out.push_str(&format!("\nReviewed by {} ({}).\n", pick.label(), pick.why));
        }
        out
    }
}

// ── The knob ─────────────────────────────────────────────────────────────────

/// Whether the gate runs for this goal. DEFAULT ON — a goal that finishes
/// unreviewed is the thing this prevents, so turning it off is deliberate:
/// `independent_review: false` on the goal card, on the project, or in
/// `verifier.json`. The goal's own setting wins, then the project's, then the
/// global one.
pub fn gate_enabled(
    global_default: bool,
    project_meta: Option<&Value>,
    goal_meta: &serde_json::Map<String, Value>,
) -> bool {
    if let Some(v) = goal_meta.get(REVIEW_GATE_KEY).and_then(|v| v.as_bool()) {
        return v;
    }
    if let Some(v) = project_meta
        .and_then(|m| m.get(REVIEW_GATE_KEY))
        .and_then(|v| v.as_bool())
    {
        return v;
    }
    global_default
}

/// Which rubric a goal gets. A `prose`/`content`/`writing`/`docs`/`research`
/// goal has no diff worth five code lenses — it gets the prose rubric, graded
/// against its own acceptance criteria. Reuses dispatch's own list so the two
/// never drift.
pub fn mode_for_goal(goal_type: Option<&str>) -> ReviewMode {
    match goal_type {
        Some(t)
            if permagent::agents::platform_extensions::orchestrator::NON_CODE_GOAL_TYPES
                .contains(&t) =>
        {
            ReviewMode::Rubric
        }
        _ => ReviewMode::Code,
    }
}

// ── The transport ────────────────────────────────────────────────────────────

/// One question to one model. A trait so the gate's whole decision path is
/// testable against a fake — no test in this tree ever makes a live model call.
#[async_trait::async_trait]
pub trait ReviewerClient: Send + Sync {
    async fn ask(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String, String>;
}

/// The live reviewer: build the provider by name, ask once, take the text.
pub struct LiveReviewer;

#[async_trait::async_trait]
impl ReviewerClient for LiveReviewer {
    async fn ask(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String, String> {
        let p = permagent::providers::create_with_named_model(provider, model, Vec::new())
            .await
            .map_err(|e| format!("could not create provider '{provider}': {e}"))?;
        let model_config = permagent::model::ModelConfig::new(model)
            .map_err(|e| format!("could not configure model '{model}': {e}"))?;
        let message = permagent::conversation::message::Message::user().with_text(user);
        let (reply, _usage) = p
            .complete(
                &model_config,
                &format!("review-gate-{}", uuid::Uuid::new_v4()),
                system,
                std::slice::from_ref(&message),
                &[],
            )
            .await
            .map_err(|e| format!("the reviewer call failed: {e}"))?;
        Ok(reply.as_concat_text())
    }
}

/// How the gate reaches a reviewer. Production uses [`ReviewDeps::live`]; a test
/// substitutes a canned client and a pre-resolved reviewer, so the WIRED path is
/// exercised end to end without a network call or a configured API key.
pub struct ReviewDeps<'a> {
    pub client: &'a dyn ReviewerClient,
    /// Pre-resolved reviewer. `None` ⇒ choose one live.
    pub selection: Option<ReviewerSelection>,
    /// Pre-resolved spend verdict. `None` ⇒ evaluate the caps live.
    pub spend: Option<SpendDecision>,
}

impl ReviewDeps<'static> {
    /// The real thing: choose a reviewer from the models the user has, respect
    /// the live spend caps, and ask the model.
    pub fn live() -> Self {
        Self {
            client: &LiveReviewer,
            selection: None,
            spend: None,
        }
    }
}

impl Default for ReviewDeps<'static> {
    fn default() -> Self {
        Self::live()
    }
}

// ── The run ──────────────────────────────────────────────────────────────────

/// Everything the gate needs about the goal under review. Assembled by the
/// caller from what verification already computed, so the gate itself does no
/// database work and stays testable.
pub struct ReviewInputs {
    pub goal_id: String,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub goal_type: Option<String>,
    /// Paths the goal changed — the review trigger's input.
    pub changed_paths: Vec<String>,
    /// Insertions + deletions.
    pub changed_lines: usize,
    /// The diff text (code mode) or the artifact text (rubric mode).
    pub body: String,
    /// The passing check output the reviewer is asked to distrust.
    pub verify_output: String,
    /// The (provider, model) that produced the work. `None` ⇒ nothing can be
    /// proven cross-family, and the gate says so instead of pretending.
    pub worker: Option<(String, String)>,
    /// The previous round's review, for the repeated-finding cap.
    pub prior: Option<IndependentReview>,
}

impl ReviewInputs {
    fn task_spec(&self) -> String {
        let mut s = format!(
            "TITLE: {}\n\n{}",
            self.title.trim(),
            self.description.trim()
        );
        if !self.acceptance_criteria.is_empty() {
            s.push_str("\n\nACCEPTANCE CRITERIA:\n");
            for (i, c) in self.acceptance_criteria.iter().enumerate() {
                s.push_str(&format!("{}. {}\n", i + 1, c.trim()));
            }
        }
        s
    }
}

/// Truncate to `max` characters, saying so — the reviewer must know it did not
/// see everything, or an APPROVE would be a claim about text it never read.
pub fn truncate_for_review(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // Cut on a char boundary — `char_indices` gives boundaries by construction,
    // so no slice here can split a multi-byte character.
    let cut = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= max)
        .last()
        .unwrap_or(0);
    let (head, _) = text.split_at(cut);
    format!(
        "{}\n\n[TRUNCATED: {} of {} characters shown. You have NOT seen all of this change — \
         if the part you cannot see could carry the answer, the verdict is UNCERTAIN.]",
        head,
        cut,
        text.len()
    )
}

/// Run the gate for one goal.
///
/// `selection` and `spend` are passed in rather than resolved here so the whole
/// decision path is exercised by tests with no config, no network, and no
/// process-global state.
pub async fn run_review(
    inputs: &ReviewInputs,
    selection: &ReviewerSelection,
    spend: Option<&SpendDecision>,
    client: &dyn ReviewerClient,
) -> IndependentReview {
    let mode = mode_for_goal(inputs.goal_type.as_deref());

    // ── The trigger. A pure test/doc/format change, or a few lines, is not
    // worth a paid cross-family round. A PROSE goal is never skipped on these
    // grounds — a doc IS its product, and skipping it is how prose goals ended
    // up finishing unjudged in the first place.
    if mode == ReviewMode::Code {
        let trigger = review_required(&inputs.changed_paths, inputs.changed_lines);
        if let permagent::cost_router::ReviewTrigger::Skip { reason } = trigger {
            let mut r = IndependentReview::base(
                mode,
                ReviewDecision::Skipped,
                format!("No independent review was needed: {reason}."),
            );
            r.reason = Some(reason);
            return r;
        }
    }

    // ── Nothing to read is not something to approve. A degraded diff range or
    // an empty artifact means the reviewer would be signing off on text it never
    // saw, so the review is unavailable rather than vacuously approving.
    if inputs.body.trim().is_empty() {
        let mut r = IndependentReview::base(
            mode,
            ReviewDecision::Unavailable,
            "The independent review could not read what this goal produced, so the work is \
             not confirmed complete."
                .to_string(),
        );
        r.reason = Some(
            "no diff or artifact text could be read for this goal — there was nothing for an \
             independent reviewer to examine"
                .to_string(),
        );
        return r;
    }

    // ── Who reviews.
    let pick = match selection {
        ReviewerSelection::Reviewer(p) => p.as_ref().clone(),
        ReviewerSelection::Unavailable { reason } => {
            let mut r = IndependentReview::base(
                mode,
                ReviewDecision::Unavailable,
                "No independent reviewer was available, so this work is not confirmed complete."
                    .to_string(),
            );
            r.reason = Some(reason.clone());
            return r;
        }
    };

    // ── May we pay for it. A refusal degrades to Uncertain-and-parked; it never
    // degrades to Pass.
    if let Some(SpendDecision::Refuse { reason }) = spend {
        let mut r = IndependentReview::base(
            mode,
            ReviewDecision::Unavailable,
            "The independent review was not run because of the spend cap, so this work is \
             not confirmed complete."
                .to_string(),
        );
        r.reviewer = Some(pick);
        r.reason = Some(reason.clone());
        return r;
    }

    // ── Ask.
    let body = truncate_for_review(
        &inputs.body,
        match mode {
            ReviewMode::Code => MAX_DIFF_CHARS,
            ReviewMode::Rubric => MAX_ARTIFACT_CHARS,
        },
    );
    let prior_findings: Vec<ReviewFinding> = inputs
        .prior
        .as_ref()
        .map(|p| p.findings.clone())
        .unwrap_or_default();
    let user_prompt = match mode {
        ReviewMode::Code => build_review_prompt(
            &inputs.task_spec(),
            &body,
            &inputs.verify_output,
            &prior_findings,
        ),
        ReviewMode::Rubric => build_rubric_prompt(
            &inputs.task_spec(),
            &inputs.acceptance_criteria,
            &body,
            &prior_findings,
        ),
    };
    let estimated = pick.estimate_cost_usd(
        ((user_prompt.len() + mode.system_prompt().len()) / CHARS_PER_TOKEN) as u64,
        REVIEW_OUTPUT_TOKENS,
    );

    let raw = match client
        .ask(
            &pick.provider,
            &pick.model,
            mode.system_prompt(),
            &user_prompt,
        )
        .await
    {
        Ok(raw) => raw,
        Err(e) => {
            // A review that could not run fails SOFT to the human — never a
            // silent "done".
            let mut r = IndependentReview::base(
                mode,
                ReviewDecision::Unavailable,
                "The independent review could not be completed, so this work is not \
                 confirmed complete."
                    .to_string(),
            );
            r.cross_family = pick.cross_family;
            r.estimated_cost_usd = estimated;
            r.reviewer = Some(pick);
            r.reason = Some(e);
            return r;
        }
    };

    // ── Decide. Deterministic Rust owns the verdict → action map.
    let outcome: ReviewOutcome = parse_review(&raw);
    let fingerprint = outcome.fingerprint();
    let consecutive = match inputs.prior.as_ref() {
        Some(p) if p.fingerprint == fingerprint && !fingerprint.is_empty() => {
            p.consecutive_identical.saturating_add(1)
        }
        _ => 1,
    };
    // The trigger already ruled a code diff substantive; a prose artifact always
    // is (there is no "trivial" essay).
    let decision = gate_decision(&outcome, true, consecutive);

    let mut r = IndependentReview::base(mode, ReviewDecision::Blocked, String::new());
    r.verdict = Some(outcome.verdict);
    r.checked = outcome.checked.clone();
    r.findings = outcome.grounded_findings();
    r.cross_family = pick.cross_family;
    r.estimated_cost_usd = estimated;
    r.fingerprint = fingerprint;
    r.consecutive_identical = consecutive;
    let who = pick.label();
    r.reviewer = Some(pick);

    match decision {
        GateDecision::Proceed {
            rubber_stamp_logged,
        } => {
            r.decision = ReviewDecision::Passed;
            r.rubber_stamp = rubber_stamp_logged;
            r.summary = if r.cross_family {
                format!("{who}, from a different model family than the worker, reviewed this work and approved it")
            } else {
                format!("{who} reviewed this work and approved it — note it shares the worker's model family, so this was not an independent cross-family review")
            };
        }
        GateDecision::RequestChanges { findings } => {
            r.decision = ReviewDecision::Rejected;
            r.findings = findings;
            r.summary = format!(
                "{who} reviewed this work and found {} problem(s) — it goes back with them",
                r.findings.len()
            );
        }
        GateDecision::Block { reason } => {
            r.decision = ReviewDecision::Blocked;
            r.summary =
                format!("{who} could not sign off on this work — a human decision is needed");
            r.reason = Some(reason);
        }
        GateDecision::Escalate { reason } => {
            r.decision = ReviewDecision::Escalated;
            r.summary = format!(
                "{who} returned the same review {consecutive} times without resolution — this needs your direction"
            );
            r.reason = Some(reason);
        }
        GateDecision::ReviewUnavailable { reason } => {
            r.decision = ReviewDecision::Unavailable;
            r.summary =
                "The independent review could not be completed, so this work is not confirmed complete."
                    .to_string();
            r.reason = Some(reason);
        }
    }
    r
}

// ── Gathering the diff the reviewer reads ────────────────────────────────────

/// The full patch text for the range the verifier already resolved. Empty when
/// the range degraded — the caller then treats the review as unavailable rather
/// than reviewing nothing and calling it approved.
pub async fn diff_text(working_dir: Option<&Path>, diff_range_args: &[String]) -> String {
    let (Some(wd), false) = (working_dir, diff_range_args.is_empty()) else {
        return String::new();
    };
    let mut args: Vec<String> = vec![
        "diff".to_string(),
        "--no-color".to_string(),
        format!("--unified={}", 3),
    ];
    args.extend(diff_range_args.iter().cloned());
    let out = tokio::process::Command::new("git")
        .args(&args)
        .current_dir(wd)
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(o) => {
            tracing::warn!(
                target: "permagentd::verification",
                "could not read the diff for the independent review: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            String::new()
        }
        Err(e) => {
            tracing::warn!(
                target: "permagentd::verification",
                "could not run git for the independent review: {e}"
            );
            String::new()
        }
    }
}

/// Resolve the worker's (provider, model) — first from the dispatch routing
/// receipt the orchestrator writes, then from the worker session row. `None`
/// when neither recorded it, which the selection then reports honestly.
pub async fn worker_model(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    meta: &serde_json::Map<String, Value>,
) -> Option<(String, String)> {
    if let Some(routing) = meta
        .get("capability_snapshot")
        .and_then(|c| c.get("model_routing"))
    {
        if let (Some(p), Some(m)) = (
            routing.get("provider").and_then(|v| v.as_str()),
            routing.get("model").and_then(|v| v.as_str()),
        ) {
            if !p.is_empty() && !m.is_empty() {
                return Some((p.to_string(), m.to_string()));
            }
        }
    }
    let session_id = meta.get("worker_session_id")?.as_str()?;
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT provider_name, model_config_json FROM sessions WHERE id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let provider = row.0.filter(|s| !s.is_empty())?;
    let model = row
        .1
        .and_then(|j| serde_json::from_str::<Value>(&j).ok())
        .and_then(|v| {
            v.get("model_name")
                .and_then(|m| m.as_str())
                .map(String::from)
        })
        .filter(|s| !s.is_empty())?;
    Some((provider, model))
}

/// Choose the reviewer live: the hand-configured REVIEW role, the derived
/// best-fit map, then the cheapest capable different-family model the user has.
pub async fn select_for_goal(
    worker: Option<(&str, &str)>,
    changed_lines: usize,
) -> ReviewerSelection {
    let configured =
        permagent::cost_router::role_model(permagent::cost_router::WorkflowRole::Review);
    let derived = permagent::cost_router::derived_role_map().await;
    let derived_review = derived
        .get(permagent::cost_router::WorkflowRole::Review)
        .map(|(rm, _)| rm.clone());
    let available = permagent::cost_router::discover_available_models_async().await;
    select_reviewer(
        worker,
        configured.as_ref(),
        derived_review.as_ref(),
        &available,
        changed_lines,
    )
}

/// The spend verdict for one review: the worker session's own cost against the
/// configured ceilings. Unpriced calls lift the band to Soft (visible, not a
/// stop); an unpriced REVIEWER is refused by [`reviewer_spend_gate`] itself.
pub async fn spend_for_goal(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    meta: &serde_json::Map<String, Value>,
    pick: &ReviewerPick,
) -> SpendDecision {
    let spent = match meta.get("worker_session_id").and_then(|v| v.as_str()) {
        Some(sid) => sqlx::query_scalar::<_, Option<f64>>(
            "SELECT accumulated_cost_usd FROM sessions WHERE id = ?",
        )
        .bind(sid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .unwrap_or(0.0),
        None => 0.0,
    };
    let cfg = permagent::cost_router::budget::load_budget_config();
    let verdict = permagent::cost_router::budget_verdict(spent, spent, &cfg);
    reviewer_spend_gate(pick, &verdict)
}

/// The escalation bound, restated where the caller reads it.
pub const ESCALATE_AT: u32 = REVIEW_ESCALATE_AT;

#[cfg(test)]
mod tests {
    use super::*;
    use permagent::cost_router::{ReviewerPick, ReviewerSource};

    /// A reviewer that returns exactly what the test tells it to — no network.
    struct Canned(String);

    #[async_trait::async_trait]
    impl ReviewerClient for Canned {
        async fn ask(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    /// A reviewer whose transport fails.
    struct Broken;

    #[async_trait::async_trait]
    impl ReviewerClient for Broken {
        async fn ask(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, String> {
            Err("connection refused".to_string())
        }
    }

    fn pick(cross_family: bool) -> ReviewerSelection {
        ReviewerSelection::Reviewer(Box::new(ReviewerPick {
            provider: "otherco".into(),
            model: "other-1".into(),
            family: if cross_family { "otherco" } else { "workerco" }.into(),
            worker_family: "workerco".into(),
            source: ReviewerSource::BestFit,
            cross_family,
            cost_hint_per_mtok: 1.5,
            input_usd_per_mtok: 0.5,
            output_usd_per_mtok: 1.0,
            priced: true,
            is_local: false,
            why: "cheapest capable different-family model".into(),
            warning: None,
        }))
    }

    fn code_inputs() -> ReviewInputs {
        ReviewInputs {
            goal_id: "g1".into(),
            title: "Do the thing".into(),
            description: "The thing must be done".into(),
            acceptance_criteria: vec!["the thing is done".into()],
            goal_type: Some("feature".into()),
            changed_paths: vec!["crates/goose/src/lib.rs".into()],
            changed_lines: 120,
            body: "--- a\n+++ b\n+fn thing() {}\n".into(),
            verify_output: "all checks passed".into(),
            worker: Some(("workerco".into(), "worker-1".into())),
            prior: None,
        }
    }

    #[tokio::test]
    async fn an_evidenced_approval_passes() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE\nCHECKED: the new fn and its callers".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Passed);
        assert!(r.decision.allows_completion());
        assert!(r.cross_family);
        assert!(r.summary.contains("different model family"));
    }

    #[tokio::test]
    async fn an_unevidenced_approval_is_blocked_not_passed() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Blocked);
        assert!(!r.decision.allows_completion());
    }

    #[tokio::test]
    async fn a_reject_carries_the_lens_findings_as_the_corrective_plan() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned(
                "VERDICT: REQUEST_CHANGES\nCHECKED: the diff\n\
                 FINDING TEST_INTEGRITY | the assertion was narrowed to always hold | crates/goose/src/lib.rs:42\n"
                    .into(),
            ),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Rejected);
        assert!(!r.decision.allows_completion());
        assert_eq!(r.findings.len(), 1);
        let plan = r.corrective_plan();
        assert!(plan.contains("TEST_INTEGRITY"));
        assert!(plan.contains("crates/goose/src/lib.rs:42"));
        assert!(plan.contains("narrowed"));
    }

    #[tokio::test]
    async fn an_ungrounded_reject_blocks_rather_than_looping_on_vapor() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned(
                "VERDICT: REQUEST_CHANGES\nCHECKED: x\nFINDING CORRECTNESS | feels wrong |\n"
                    .into(),
            ),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Blocked);
    }

    #[tokio::test]
    async fn the_same_review_repeated_escalates_to_a_person() {
        let raw = "VERDICT: REQUEST_CHANGES\nCHECKED: the diff\n\
                   FINDING CORRECTNESS | off by one | src/a.rs:7\n";
        let mut inputs = code_inputs();
        let mut last: Option<IndependentReview> = None;
        for _ in 0..ESCALATE_AT {
            inputs.prior = last.clone();
            last = Some(
                run_review(
                    &inputs,
                    &pick(true),
                    Some(&SpendDecision::Allow),
                    &Canned(raw.into()),
                )
                .await,
            );
        }
        let final_review = last.expect("ran");
        assert_eq!(final_review.decision, ReviewDecision::Escalated);
        assert!(final_review.decision.parks());
        assert_eq!(final_review.consecutive_identical, ESCALATE_AT);
    }

    #[tokio::test]
    async fn a_spend_refusal_degrades_to_uncertain_and_parked_never_to_pass() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Refuse {
                reason: "the task budget has reached its gate".into(),
            }),
            &Canned("VERDICT: APPROVE\nCHECKED: everything".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Unavailable);
        assert!(!r.decision.allows_completion());
        assert!(r.decision.parks());
        assert!(r.reason.as_deref().unwrap().contains("budget"));
    }

    #[tokio::test]
    async fn no_cross_family_reviewer_is_unavailable_never_a_pass() {
        let r = run_review(
            &code_inputs(),
            &ReviewerSelection::Unavailable {
                reason: "nothing from another family".into(),
            },
            None,
            &Canned("VERDICT: APPROVE\nCHECKED: everything".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Unavailable);
        assert!(!r.decision.allows_completion());
    }

    #[tokio::test]
    async fn a_transport_failure_fails_soft_to_the_human() {
        let r = run_review(
            &code_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Broken,
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Unavailable);
        assert!(r.reason.as_deref().unwrap().contains("connection refused"));
    }

    #[tokio::test]
    async fn a_same_family_approval_is_never_described_as_independent() {
        let r = run_review(
            &code_inputs(),
            &pick(false),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE\nCHECKED: the diff".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Passed);
        assert!(!r.cross_family);
        assert!(r.summary.contains("not an independent cross-family review"));
    }

    #[tokio::test]
    async fn a_trivial_or_doc_only_change_skips_the_paid_review() {
        let mut inputs = code_inputs();
        inputs.changed_paths = vec!["README.md".into()];
        let r = run_review(
            &inputs,
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE\nCHECKED: x".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Skipped);
        assert!(r.decision.allows_completion());
    }

    // ── The rubric path: non-code goals used to finish unjudged ─────────────

    fn prose_inputs() -> ReviewInputs {
        ReviewInputs {
            goal_id: "g2".into(),
            title: "Write the launch brief".into(),
            description: "One page, for the board".into(),
            acceptance_criteria: vec![
                "names the three markets".into(),
                "every figure is sourced".into(),
            ],
            goal_type: Some("prose".into()),
            changed_paths: vec!["docs/brief.md".into()],
            changed_lines: 2,
            body: "# Brief\n\nTBD\n".into(),
            verify_output: "no checks declared".into(),
            worker: Some(("workerco".into(), "worker-1".into())),
            prior: None,
        }
    }

    #[test]
    fn non_code_goal_types_get_the_prose_rubric() {
        for t in permagent::agents::platform_extensions::orchestrator::NON_CODE_GOAL_TYPES {
            assert_eq!(mode_for_goal(Some(t)), ReviewMode::Rubric, "{t}");
        }
        assert_eq!(mode_for_goal(Some("feature")), ReviewMode::Code);
        assert_eq!(mode_for_goal(None), ReviewMode::Code);
    }

    #[tokio::test]
    async fn a_prose_goal_is_graded_and_is_never_skipped_for_being_a_doc() {
        let r = run_review(
            &prose_inputs(),
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned(
                "VERDICT: REQUEST_CHANGES\nCHECKED: the brief against both criteria\n\
                 FINDING PLACEHOLDER | the body is still a stub | \"TBD\"\n\
                 FINDING COMPLETENESS | no market is named | criterion 1: names the three markets\n"
                    .into(),
            ),
        )
        .await;
        assert_eq!(r.mode, ReviewMode::Rubric);
        assert_eq!(r.decision, ReviewDecision::Rejected);
        assert_eq!(r.findings.len(), 2);
        assert!(r.lenses.contains(&"COMPLETENESS".to_string()));
        assert!(r.lenses.contains(&"CITATION".to_string()));
        assert!(r.corrective_plan().contains("TBD"));
    }

    #[tokio::test]
    async fn a_prose_goal_can_pass_its_rubric() {
        let mut inputs = prose_inputs();
        inputs.body = "# Brief\n\nThe three markets are A, B and C [source: X].\n".into();
        let r = run_review(
            &inputs,
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE\nCHECKED: both criteria against the brief".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Passed);
        assert_eq!(r.mode, ReviewMode::Rubric);
    }

    // ── The knob ────────────────────────────────────────────────────────────

    #[test]
    fn the_gate_is_on_by_default_and_off_only_when_asked() {
        let empty = serde_json::Map::new();
        assert!(gate_enabled(true, None, &empty));

        let project_off = serde_json::json!({ "independent_review": false });
        assert!(!gate_enabled(true, Some(&project_off), &empty));

        // The goal's own setting wins over the project's.
        let mut goal_on = serde_json::Map::new();
        goal_on.insert("independent_review".into(), Value::Bool(true));
        assert!(gate_enabled(true, Some(&project_off), &goal_on));

        let mut goal_off = serde_json::Map::new();
        goal_off.insert("independent_review".into(), Value::Bool(false));
        assert!(!gate_enabled(true, None, &goal_off));

        // A global default of off still stands when nothing overrides it.
        assert!(!gate_enabled(false, None, &empty));
    }

    #[tokio::test]
    async fn an_unreadable_change_is_unavailable_not_approved() {
        let mut inputs = code_inputs();
        inputs.body = "   \n".into();
        let r = run_review(
            &inputs,
            &pick(true),
            Some(&SpendDecision::Allow),
            &Canned("VERDICT: APPROVE\nCHECKED: everything".into()),
        )
        .await;
        assert_eq!(r.decision, ReviewDecision::Unavailable);
        assert!(!r.decision.allows_completion());
    }

    #[test]
    fn a_truncated_diff_tells_the_reviewer_it_did_not_see_everything() {
        let long = "x".repeat(MAX_DIFF_CHARS + 10);
        let out = truncate_for_review(&long, MAX_DIFF_CHARS);
        assert!(out.contains("TRUNCATED"));
        assert!(out.contains("UNCERTAIN"));
        assert_eq!(truncate_for_review("short", MAX_DIFF_CHARS), "short");
    }

    #[test]
    fn a_prior_review_round_trips_through_the_verdict_record() {
        let r = IndependentReview::base(
            ReviewMode::Code,
            ReviewDecision::Rejected,
            "went back".into(),
        );
        let meta: serde_json::Map<String, Value> = serde_json::from_value(serde_json::json!({
            "dispatch_evidence": { "verdict": { REVIEW_RECORD_KEY: r } }
        }))
        .unwrap();
        let back = IndependentReview::from_metadata(&meta).expect("round trip");
        assert_eq!(back.decision, ReviewDecision::Rejected);
        assert_eq!(back.mode, ReviewMode::Code);
    }
}
