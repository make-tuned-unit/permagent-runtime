//! Two-round debate plus chair synthesis. Callers inject [`MemberCaller`]
//! so tests never hit a live provider.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use super::membership::Member;
use super::verdict;
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::base::Provider;
use crate::providers::base::ProviderUsage;
use crate::providers::canonical::{
    cache_savings_of, cost_breakdown, maybe_get_pricing, worst_case_pricing,
};
use crate::providers::RetryConfig;
use crate::session::{budget_task_id, CostLedgerRow, CostTier, Session, SessionManager};

fn council_ledger_row(
    session: &Session,
    session_id: &str,
    provider: &str,
    model: &str,
    cost_tier: CostTier,
    usage: &ProviderUsage,
) -> Result<CostLedgerRow, String> {
    // B4 debt: this intentionally mirrors the primary Agent ledger-row
    // builder until the two paths can share one constructor without widening
    // this Council slice. Keep attribution and pricing fields in lockstep.
    let pricing = maybe_get_pricing(provider, model);
    let breakdown = pricing
        .as_ref()
        .and_then(|p| cost_breakdown(&usage.usage, p));
    let estimated = if cost_tier.is_chargeable() && breakdown.is_none() {
        worst_case_pricing(provider).and_then(|p| cost_breakdown(&usage.usage, &p))
    } else {
        None
    };
    let (cost_usd, input_cost, output_cost, cache_read_cost, cache_write_cost, is_estimated) =
        match (
            cost_tier.is_chargeable(),
            breakdown.as_ref(),
            estimated.as_ref(),
        ) {
            (false, _, _) => (0.0, 0.0, 0.0, 0.0, 0.0, false),
            (true, Some(b), _) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                false,
            ),
            (true, None, Some(b)) => (
                b.total_cost,
                b.input_cost,
                b.output_cost,
                b.cache_read_cost,
                b.cache_write_cost,
                true,
            ),
            (true, None, None) => {
                return Err("council provider usage has no safe pricing".to_string())
            }
        };
    let tok = |value: Option<i32>| i64::from(value.unwrap_or(0).max(0));
    let is_headless = !session.session_type.is_interactive();
    Ok(CostLedgerRow {
        call_id: usage
            .invocation_id
            .clone()
            .ok_or_else(|| "council usage has no invocation identity".to_string())?,
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: session_id.to_string(),
        parent_session_id: session.parent_session_id.clone(),
        task_id: budget_task_id(&session.extension_data),
        goal_id: crate::session::goal_id(&session.extension_data),
        subagent_id: None,
        provider: Some(provider.to_string()),
        model: Some(model.to_string()),
        cost_tier,
        is_headless,
        input_tokens: tok(usage.usage.input_tokens),
        output_tokens: tok(usage.usage.output_tokens),
        cache_read_tokens: tok(usage.usage.cache_read_input_tokens),
        cache_write_tokens: tok(usage.usage.cache_write_input_tokens),
        input_cost,
        output_cost,
        cache_read_cost,
        cache_write_cost,
        cost_usd,
        cache_savings_usd: if cost_tier.is_chargeable() {
            pricing
                .as_ref()
                .map(|p| cache_savings_of(&usage.usage, p))
                .unwrap_or(0.0)
        } else {
            0.0
        },
        is_estimated,
    })
}

pub const MEMBER_TIMEOUT_SECS: u64 = 90;
/// LiveCaller owns the authoritative timeout so it can mark a paid hold
/// unknown after dispatch. Keep the orchestration guard slightly longer; an
/// equal deadline could drop LiveCaller before its unknown transition runs.
const MEMBER_TIMEOUT_GUARD_SECS: u64 = MEMBER_TIMEOUT_SECS + 5;
pub const MAX_ACTIONS: usize = 5;

/// How many trailing report lines the verdict re-ask quotes back. Enough for
/// the chair to recognize its own ruling, short enough that the nag stays a
/// cheap call rather than a second synthesis.
const NAG_TAIL_LINES: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round1Take {
    #[serde(default)]
    pub projects_need_attention: Vec<String>,
    #[serde(default)]
    pub signs_to_recognize: Vec<String>,
    #[serde(default)]
    pub missing_patterns: Vec<String>,
    #[serde(default)]
    pub promising_analytics: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round2Take {
    #[serde(default)]
    pub votes: Vec<String>,
    #[serde(default)]
    pub dissent: Option<String>,
    #[serde(default)]
    pub revised: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChairReport {
    #[serde(default)]
    pub headline: String,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub consensus: Vec<String>,
    #[serde(default)]
    pub dissent: Vec<Value>,
    #[serde(default)]
    pub actions: Vec<ChairAction>,
    /// Present for an active Build request. The daemon supplies project and
    /// memory identity after parsing; the chair only authors the work graph.
    #[serde(default)]
    pub dag: Option<super::plan::CouncilDagDraft>,
    /// True when the chair never produced a parseable `VERDICT:` line, even
    /// after one re-ask. The report's markdown then carries
    /// [`verdict::NO_VERDICT_FLAG`] in place of a ruling, so the gap is durable
    /// in the record rather than inferred later. Not a stored column: the
    /// markdown itself is the record, and `verdict::parse` reads it back.
    #[serde(default)]
    pub verdict_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChairAction {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub project_name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct MemberResult {
    pub member: Member,
    pub status: String,
    pub raw: Option<String>,
    pub parsed: Option<Value>,
    pub error: Option<String>,
    /// Authoritative provider usage retained for the Council's durable cost
    /// ledger. It is never inferred from the response text.
    pub usage: Option<ProviderUsage>,
}

#[derive(Debug, Clone)]
pub struct MemberCallResult {
    pub text: String,
    pub usage: ProviderUsage,
}

/// Durable identity for all Council provider calls. LiveCaller is deliberately
/// not constructible without an existing harness session and manager.
#[derive(Clone)]
pub struct CouncilCallContext {
    pub manager: Arc<SessionManager>,
    pub session_id: String,
}

/// Narrow provider boundary used by LiveCaller. Keeping construction and the
/// physical call behind this seam lets tests prove accounting ordering without
/// registering credentials or contacting a network provider.
#[async_trait::async_trait]
pub(crate) trait CouncilProviderFactory: Send + Sync {
    async fn create(&self, provider: &str, model: &str)
        -> Result<Arc<dyn CouncilProvider>, String>;
}

#[async_trait::async_trait]
pub(crate) trait CouncilProvider: Send + Sync {
    fn cost_tier(&self) -> CostTier;
    fn model_config(&self) -> ModelConfig;
    fn retry_config(&self) -> RetryConfig;
    async fn complete(
        &self,
        session_id: &str,
        system: &str,
        messages: &[Message],
    ) -> Result<(Message, ProviderUsage), String>;
}

struct DefaultCouncilProviderFactory;

struct ProviderAdapter {
    provider: Arc<dyn Provider>,
}

#[async_trait::async_trait]
impl CouncilProviderFactory for DefaultCouncilProviderFactory {
    async fn create(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Arc<dyn CouncilProvider>, String> {
        let provider = crate::providers::create_with_named_model(provider, model, Vec::new())
            .await
            .map_err(|e| format!("provider init failed: {e}"))?;
        Ok(Arc::new(ProviderAdapter { provider }))
    }
}

#[async_trait::async_trait]
impl CouncilProvider for ProviderAdapter {
    fn cost_tier(&self) -> CostTier {
        self.provider.cost_tier()
    }

    fn model_config(&self) -> ModelConfig {
        self.provider.get_model_config()
    }

    fn retry_config(&self) -> RetryConfig {
        self.provider.retry_config()
    }

    async fn complete(
        &self,
        session_id: &str,
        system: &str,
        messages: &[Message],
    ) -> Result<(Message, ProviderUsage), String> {
        self.provider
            // permagent-dispatch: seam=council_provider_adapter_transport_v1 class=excluded reason=caller_reservation_settlement authority=council_live_caller
            .complete(
                &self.provider.get_model_config(),
                session_id,
                system,
                messages,
                &[],
            )
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
pub trait MemberCaller: Send + Sync {
    async fn complete(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<MemberCallResult, String>;
}

pub struct LiveCaller {
    context: CouncilCallContext,
    provider_factory: Arc<dyn CouncilProviderFactory>,
    timeout: std::time::Duration,
    budget_config: Option<crate::cost_router::budget::BudgetConfig>,
}

struct ReservationUnknownGuard {
    manager: Arc<SessionManager>,
    reservation_id: Option<String>,
    armed: bool,
}

impl ReservationUnknownGuard {
    fn new(manager: Arc<SessionManager>, reservation_id: Option<String>) -> Self {
        Self {
            manager,
            armed: reservation_id.is_some(),
            reservation_id,
        }
    }

    async fn mark_unknown(&mut self) -> Result<(), String> {
        if !self.armed {
            return Ok(());
        }
        let Some(id) = self.reservation_id.as_deref() else {
            self.armed = false;
            return Ok(());
        };
        self.manager
            .mark_provider_invocation_unknown(id)
            .await
            .map_err(|e| e.to_string())?;
        self.armed = false;
        Ok(())
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ReservationUnknownGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(reservation_id) = self.reservation_id.clone() else {
            return;
        };
        let manager = Arc::clone(&self.manager);
        // Cancellation cannot await the durable transition. Schedule it on
        // the current runtime; the lease remains the restart-safe fallback if
        // the runtime itself is already shutting down.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = manager
                    .mark_provider_invocation_unknown(&reservation_id)
                    .await
                {
                    tracing::warn!(
                        reservation_id = %reservation_id,
                        "could not mark cancelled Council reservation unknown: {error}"
                    );
                }
            });
        }
    }
}

impl LiveCaller {
    pub fn new(manager: Arc<SessionManager>, session_id: impl Into<String>) -> Self {
        Self {
            context: CouncilCallContext {
                manager,
                session_id: session_id.into(),
            },
            provider_factory: Arc::new(DefaultCouncilProviderFactory),
            timeout: std::time::Duration::from_secs(MEMBER_TIMEOUT_SECS),
            budget_config: None,
        }
    }

    #[cfg(test)]
    fn new_with_factory(
        manager: Arc<SessionManager>,
        session_id: impl Into<String>,
        provider_factory: Arc<dyn CouncilProviderFactory>,
        timeout: std::time::Duration,
        budget_config: crate::cost_router::budget::BudgetConfig,
    ) -> Self {
        Self {
            context: CouncilCallContext {
                manager,
                session_id: session_id.into(),
            },
            provider_factory,
            timeout,
            budget_config: Some(budget_config),
        }
    }
}

async fn record_council_budget_block(
    context: &CouncilCallContext,
    session: &Session,
    outcome: &crate::session::CostReservationOutcome,
) -> String {
    let detail = format!("council provider call blocked before dispatch: {outcome:?}");
    let Some(card_id) = crate::session::goal_id(&session.extension_data) else {
        return detail;
    };
    let Ok(pool) = context.manager.pool_clone().await else {
        return format!("{detail}; Decision Inbox unavailable");
    };
    let is_gate = matches!(
        outcome,
        crate::session::CostReservationOutcome::NeedsGate { .. }
    );
    let kind = if is_gate { "choice" } else { "unblock" };
    let open = crate::decisions::find_open_decision_for_goal(&pool, &card_id, kind)
        .await
        .ok()
        .flatten()
        .is_some_and(|d| !is_gate || d.headline.contains("authorization needs approval"));
    if !open {
        let request = if is_gate {
            let crate::session::CostReservationOutcome::NeedsGate {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
            } = outcome
            else {
                unreachable!()
            };
            let cfg = crate::cost_router::budget::load_budget_config();
            crate::cost_router::budget::reservation_gate_decision_request(
                *scope,
                *spent_usd,
                *held_usd,
                *requested_usd,
                *ceiling_usd,
                match scope {
                    crate::cost_router::budget::BudgetScope::Task => cfg.task.gate,
                    crate::cost_router::budget::BudgetScope::Session => cfg.session.gate,
                },
                Some(card_id.clone()),
                None,
            )
        } else {
            let reason = if matches!(
                outcome,
                crate::session::CostReservationOutcome::Refused { .. }
            ) {
                crate::decisions::UnblockReason::TokenBudget
            } else {
                crate::decisions::UnblockReason::Stuck
            };
            crate::decisions::NewDecision {
                kind: kind.to_string(),
                goal_id: Some(card_id.clone()),
                headline: Some(crate::decisions::truncate_for_headline(
                    "Council provider spend authorization blocked this goal",
                )),
                detail: Some(detail.clone()),
                payload: serde_json::to_value(crate::decisions::UnblockPayload {
                    reason,
                    spent: None,
                    cap: None,
                })
                .unwrap_or_default(),
                ..Default::default()
            }
        };
        if let Err(error) = crate::decisions::create_decision(&pool, request).await {
            tracing::error!(goal_id = %card_id, "could not create Council budget decision: {error}");
        }
    }
    if let Err(error) =
        crate::goal_transition::park_goal(&pool, &card_id, crate::decisions::ACTOR_SYSTEM, &detail)
            .await
    {
        tracing::warn!(goal_id = %card_id, "could not park Council budget-blocked goal: {error}");
    }
    if let Some(kill) = crate::agents::platform_extensions::orchestrator::take_goal_worker(&card_id)
    {
        kill.kill();
    }
    detail
}

#[async_trait::async_trait]
impl MemberCaller for LiveCaller {
    async fn complete(
        &self,
        provider: &str,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<MemberCallResult, String> {
        let session = self
            .context
            .manager
            .get_session(&self.context.session_id, false)
            .await
            .map_err(|e| format!("council session context is invalid: {e}"))?;
        let task_id = budget_task_id(&session.extension_data).ok_or_else(|| {
            "council provider call requires a durable budget task identity".to_string()
        })?;
        // Validate durable attribution before constructing the provider. A
        // malformed or stale Council context must not even initialize a
        // provider, since a factory may perform credential or transport work.
        let p = self.provider_factory.create(provider, model).await?;
        let invocation_id = uuid::Uuid::new_v4().to_string();
        let tier = p.cost_tier();
        let config = p.model_config();
        let attempts = p.retry_config().max_physical_attempts();
        let bound =
            crate::cost_router::plan_reservation_bound(provider, model, tier, &config, attempts)
                .map_err(|e| format!("council provider call is not priced safely: {e}"))?;
        let reservation_id = if let Some(bound) = bound {
            let lease_until = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
            match self
                .context
                .manager
                .reserve_provider_invocation(
                    &invocation_id,
                    &self.context.session_id,
                    Some(&task_id),
                    bound.amount_usd,
                    &lease_until,
                    &self
                        .budget_config
                        .clone()
                        .unwrap_or_else(crate::cost_router::budget::load_budget_config),
                )
                .await
                .map_err(|e| format!("council provider reservation failed: {e}"))?
            {
                crate::session::CostReservationOutcome::Granted { reservation_id }
                | crate::session::CostReservationOutcome::AlreadyReserved { reservation_id } => {
                    Some(reservation_id)
                }
                other => {
                    let detail = record_council_budget_block(&self.context, &session, &other).await;
                    return Err(detail);
                }
            }
        } else {
            None
        };
        let mut unknown_guard =
            ReservationUnknownGuard::new(Arc::clone(&self.context.manager), reservation_id.clone());
        let msg = crate::conversation::message::Message::user().with_text(user);
        // Use one explicit physical call here. complete_fast can silently
        // issue a second provider request, which cannot share this invocation
        // identity; the next contract revision can add a separate reserved
        // fallback invocation without weakening this accounting seam.
        let result = tokio::time::timeout(
            self.timeout,
            // permagent-dispatch: seam=council_live_provider_transport_v1 class=excluded reason=caller_reservation_settlement authority=council_live_caller
            p.complete(&self.context.session_id, system, std::slice::from_ref(&msg)),
        )
        .await;
        let (response, mut usage) = match result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                unknown_guard
                    .mark_unknown()
                    .await
                    .map_err(|e| format!("provider failed and unknown marking failed: {e}"))?;
                return Err(format!("model call failed: {error}"));
            }
            Err(_) => {
                unknown_guard
                    .mark_unknown()
                    .await
                    .map_err(|e| format!("provider timed out and unknown marking failed: {e}"))?;
                return Err(format!(
                    "model call timed out after {}s",
                    self.timeout.as_secs_f64()
                ));
            }
        };
        usage = usage.with_invocation_id(invocation_id);
        if usage.usage.input_tokens.is_none()
            && usage.usage.output_tokens.is_none()
            && usage.usage.total_tokens.is_none()
            && usage.usage.cache_read_input_tokens.is_none()
            && usage.usage.cache_write_input_tokens.is_none()
        {
            unknown_guard.mark_unknown().await.map_err(|e| {
                format!("provider usage was missing and unknown marking failed: {e}")
            })?;
            return Err("model call returned no authoritative usage".to_string());
        }
        let row = match council_ledger_row(
            &session,
            &self.context.session_id,
            provider,
            model,
            tier,
            &usage,
        ) {
            Ok(row) => row,
            Err(error) => {
                unknown_guard
                    .mark_unknown()
                    .await
                    .map_err(|e| format!("usage pricing failed and unknown marking failed: {e}"))?;
                return Err(error);
            }
        };
        let total = usage.usage.total_tokens.unwrap_or(0);
        let input = usage.usage.input_tokens.unwrap_or(0);
        let output = usage.usage.output_tokens.unwrap_or(0);
        if let Some(id) = reservation_id.as_deref() {
            if let Err(error) = self
                .context
                .manager
                .settle_provider_invocation(
                    id,
                    &row,
                    None,
                    Some(total),
                    Some(input),
                    Some(output),
                    total,
                    input,
                    output,
                )
                .await
            {
                unknown_guard.mark_unknown().await.map_err(|e| {
                    format!("council settlement failed and unknown marking failed: {e}")
                })?;
                return Err(format!("council usage settlement failed: {error}"));
            }
        } else {
            self.context
                .manager
                .append_usage_and_rollup(
                    &row,
                    None,
                    Some(total),
                    Some(input),
                    Some(output),
                    total,
                    input,
                    output,
                )
                .await
                .map_err(|e| format!("council usage accounting failed: {e}"))?;
        }
        unknown_guard.disarm();
        Ok(MemberCallResult {
            text: response.as_concat_text(),
            usage,
        })
    }
}

pub fn extract_json(text: &str) -> Option<Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str(text.get(start..=end)?).ok()
}

pub fn parse_round1(text: &str) -> Option<Round1Take> {
    extract_json(text).and_then(|v| serde_json::from_value(v).ok())
}

pub fn parse_round2(text: &str) -> Option<Round2Take> {
    extract_json(text).and_then(|v| serde_json::from_value(v).ok())
}

pub fn parse_chair(text: &str) -> ChairReport {
    if let Some(mut v) = extract_json(text) {
        // Parse the work graph independently. A model can produce a useful
        // report with one malformed DAG field; that should reject the DAG,
        // not erase the Council's consensus, dissent, and narrative.
        let dag = v
            .as_object_mut()
            .and_then(|object| object.remove("dag"))
            .and_then(|value| serde_json::from_value(value).ok());
        if let Ok(mut report) = serde_json::from_value::<ChairReport>(v) {
            report.dag = dag;
            if report.actions.len() > MAX_ACTIONS {
                report.actions.truncate(MAX_ACTIONS);
            }
            if report.markdown.trim().is_empty() {
                report.markdown = text.to_string();
            }
            if report.headline.trim().is_empty() {
                report.headline = "Weekly council report".to_string();
            }
            return report;
        }
    }
    ChairReport {
        headline: "Weekly council report".to_string(),
        markdown: text.to_string(),
        consensus: Vec::new(),
        dissent: Vec::new(),
        actions: Vec::new(),
        dag: None,
        verdict_missing: false,
    }
}

pub fn round1_system() -> &'static str {
    "You are a member of a Council of LLMs advising one builder about their week. \
     You see a factual brief of their projects, boards, activity, analytics and open decisions. \
     Reply with ONLY JSON: \
     {\"projects_need_attention\":[string],\"signs_to_recognize\":[string],\
      \"missing_patterns\":[string],\"promising_analytics\":[string],\"confidence\":0.0}. \
     Be specific. Name projects. Do not invent numbers that were not in the brief. \
     You are one voice; another model will chair a synthesis."
}

pub fn round2_system() -> &'static str {
    "You already filed an independent take. Now you see the other council members' summaries. \
     Reply with ONLY JSON: \
     {\"votes\":[string],\"dissent\":string,\"revised\":string}. \
     votes: which peer claims you endorse (quote them briefly). \
     dissent: the ONE thing you would bet against the majority on, or null. \
     revised: a short restatement of your position after hearing the others."
}

pub fn chair_system() -> String {
    format!(
        "You chair a Council of LLMs. You have the same factual brief the members saw, \
         plus their round-1 takes and round-2 rebuttals. Write a weekly report the builder \
         can digest and act on. Reply with ONLY JSON: \
         {{\"headline\":string,\"markdown\":string,\"consensus\":[string],\
          \"dissent\":[{{\"model\":string,\"claim\":string}}],\
          \"actions\":[{{\"project_id\":string,\"project_name\":string,\"title\":string,\"description\":string}}],\
          \"dag\":null OR {{\"budget_limit\":integer,\"nodes\":[{{\
            \"id\":string,\"title\":string,\"description\":string,\"files\":[string],\
            \"symbols\":[string],\"pattern_references\":[string],\"acceptance_criteria\":[string],\
            \"dependencies\":[string],\
            \"required_capabilities\":[string],\"estimated_budget\":integer,\
            \"risk\":\"low\"|\"medium\"|\"high\",\
            \"verification\":{{\"command\":string,\"required\":boolean}}}}]}}}}. \
         headline: <= 80 characters. markdown: the full report in markdown, with named dissent. \
         actions: at most 5, each a concrete next step tied to a real project_id from the brief. \
         When the brief contains an Active Build project, include one DAG with 2-12 small nodes. \
         Give every node exact file/symbol boundaries, established-pattern references, acceptance \
         criteria and verification that a basic coding model can execute without guessing. Prefer \
         surgical edits over rewrites. Dependencies name earlier node ids and must be acyclic. \
         required_capabilities use concrete capabilities such as code_edit, shell, web_search, mcp, \
         or review. estimated_budget is a relative 1-10 cost unit; budget_limit covers the sum. \
         High-risk nodes must require verification. For a portfolio-only weekly report, set dag null. \
         You MAY advise. Prefer fewer, sharper actions. {}",
        verdict::prompt_clause()
    )
}

/// The re-ask system prompt. Deliberately narrow: one line, nothing else.
pub fn verdict_nag_system() -> &'static str {
    "You chaired a council report that did not end with its required ruling. Reply with \
     exactly one line and nothing else — no preamble, no JSON, no markdown fences."
}

pub fn summarize_round1(member: &Member, take: &Round1Take) -> String {
    format!(
        "### {} / {}\nattention: {}\nsigns: {}\nmissing: {}\nanalytics: {}\nconfidence: {:?}",
        member.display_name,
        member.model,
        take.projects_need_attention.join("; "),
        take.signs_to_recognize.join("; "),
        take.missing_patterns.join("; "),
        take.promising_analytics.join("; "),
        take.confidence
    )
}

async fn call_one(
    caller: &dyn MemberCaller,
    member: &Member,
    system: &str,
    user: &str,
) -> MemberResult {
    // permagent-dispatch: seam=council_member_dispatch_v1 class=excluded reason=caller_owned_dispatch authority=council_member_caller
    let fut = caller.complete(&member.provider, &member.model, system, user);
    match tokio::time::timeout(
        std::time::Duration::from_secs(MEMBER_TIMEOUT_GUARD_SECS),
        fut,
    )
    .await
    {
        Ok(Ok(call)) => MemberResult {
            member: member.clone(),
            status: "ok".to_string(),
            raw: Some(call.text),
            parsed: None,
            error: None,
            usage: Some(call.usage),
        },
        Ok(Err(e)) => MemberResult {
            member: member.clone(),
            status: "error".to_string(),
            raw: None,
            parsed: None,
            error: Some(e),
            usage: None,
        },
        Err(_) => MemberResult {
            member: member.clone(),
            status: "timeout".to_string(),
            raw: None,
            parsed: None,
            error: Some(format!("timed out after {MEMBER_TIMEOUT_SECS}s")),
            usage: None,
        },
    }
}

pub async fn run_round1(
    caller: &dyn MemberCaller,
    members: &[Member],
    brief_markdown: &str,
) -> Vec<MemberResult> {
    let mut futs = Vec::new();
    for m in members {
        let m = m.clone();
        let brief = brief_markdown.to_string();
        // Sequential join of spawned tasks so a hung member cannot stall the rest.
        futs.push(async move { call_one(caller, &m, round1_system(), &brief).await });
    }
    let mut out = Vec::new();
    for fut in futs {
        let mut r = fut.await;
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round1(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
        out.push(r);
    }
    out
}

pub async fn run_round1_parallel(
    caller: &dyn MemberCaller,
    members: &[Member],
    brief_markdown: &str,
) -> Vec<MemberResult> {
    let handles: Vec<_> = members
        .iter()
        .map(|m| {
            let m = m.clone();
            let brief = brief_markdown.to_string();
            async move { call_one(caller, &m, round1_system(), &brief).await }
        })
        .collect();
    let mut out = futures::future::join_all(handles).await;
    for r in &mut out {
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round1(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
    }
    out
}

pub async fn run_round2_parallel(
    caller: &dyn MemberCaller,
    round1: &[MemberResult],
) -> Vec<MemberResult> {
    let survivors: Vec<&MemberResult> = round1.iter().filter(|r| r.status == "ok").collect();
    if survivors.len() < 2 {
        return Vec::new();
    }
    let peer_digest: String = survivors
        .iter()
        .map(|r| {
            format!(
                "### {} / {}\n{}",
                r.member.display_name,
                r.member.model,
                r.raw.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let handles: Vec<_> = survivors
        .iter()
        .map(|r| {
            let member = r.member.clone();
            let own = r.raw.clone().unwrap_or_default();
            let digest = peer_digest.clone();
            async move {
                let user = format!(
                    "Your round-1 take:\n{own}\n\nPeer takes:\n{digest}\n\nNow vote and, if you must, dissent."
                );
                call_one(caller, &member, round2_system(), &user).await
            }
        })
        .collect();
    let mut out = futures::future::join_all(handles).await;
    for r in &mut out {
        if r.status == "ok" {
            if let Some(raw) = &r.raw {
                r.parsed = parse_round2(raw).and_then(|t| serde_json::to_value(t).ok());
            }
        }
    }
    out
}

pub async fn run_chair(
    caller: &dyn MemberCaller,
    chair: &Member,
    brief_markdown: &str,
    round1: &[MemberResult],
    round2: &[MemberResult],
) -> Result<ChairReport, String> {
    let mut user = String::from("## Brief\n\n");
    user.push_str(brief_markdown);
    user.push_str("\n\n## Round 1\n\n");
    for r in round1 {
        user.push_str(&format!(
            "### {} / {} ({})\n{}\n\n",
            r.member.display_name,
            r.member.model,
            r.status,
            r.raw.as_deref().unwrap_or(r.error.as_deref().unwrap_or(""))
        ));
    }
    if !round2.is_empty() {
        user.push_str("## Round 2\n\n");
        for r in round2 {
            user.push_str(&format!(
                "### {} / {} ({})\n{}\n\n",
                r.member.display_name,
                r.member.model,
                r.status,
                r.raw.as_deref().unwrap_or(r.error.as_deref().unwrap_or(""))
            ));
        }
    }
    let raw = caller
        // permagent-dispatch: seam=council_chair_dispatch_v1 class=excluded reason=caller_owned_dispatch authority=council_member_caller
        .complete(&chair.provider, &chair.model, &chair_system(), &user)
        .await?;
    let mut report = parse_chair(&raw.text);
    apply_verdict_gate(caller, chair, &mut report).await;
    Ok(report)
}

/// The nag. A chair report whose ruling is absent or unparseable is never
/// silently accepted: it costs one narrow re-ask, and if that also fails the
/// gap is written into the report itself as [`verdict::NO_VERDICT_FLAG`] and
/// flagged on [`ChairReport::verdict_missing`], so the briefing and the
/// rendered report both say the chair did not rule.
async fn apply_verdict_gate(caller: &dyn MemberCaller, chair: &Member, report: &mut ChairReport) {
    let problem = match verdict::parse(&report.markdown) {
        Ok(_) => return,
        Err(problem) => problem,
    };
    tracing::warn!(
        target: "permagent::council",
        "chair verdict unusable ({}); re-asking once", problem.describe()
    );
    let appended = match ask_for_verdict_line(caller, chair, &report.markdown, &problem).await {
        Some(v) => v.render(),
        None => {
            report.verdict_missing = true;
            verdict::NO_VERDICT_FLAG.to_string()
        }
    };
    report.markdown = format!("{}\n\n{}", report.markdown.trim_end(), appended);
}

/// One bounded re-ask for the verdict line alone. Returns `None` when the chair
/// errors, times out, or answers with something the strict parser still
/// rejects — the caller then flags rather than retrying again.
async fn ask_for_verdict_line(
    caller: &dyn MemberCaller,
    chair: &Member,
    markdown: &str,
    problem: &verdict::VerdictProblem,
) -> Option<verdict::ChairVerdict> {
    let lines: Vec<&str> = markdown.lines().collect();
    let tail = lines[lines.len().saturating_sub(NAG_TAIL_LINES)..].join("\n");
    let user = format!("{}\n\nYour report ended:\n\n{tail}", problem.nag());
    let result = call_one(caller, chair, verdict_nag_system(), &user).await;
    if result.status != "ok" {
        return None;
    }
    verdict::parse(result.raw.as_deref()?).ok()
}

/// True when at least one member answered round 1.
pub fn any_ok(round: &[MemberResult]) -> bool {
    round.iter().any(|r| r.status == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseMode;
    use crate::providers::base::Usage;
    use crate::session::SessionType;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    struct Scripted {
        replies: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl MemberCaller for Scripted {
        async fn complete(
            &self,
            _provider: &str,
            _model: &str,
            _system: &str,
            _user: &str,
        ) -> Result<MemberCallResult, String> {
            let mut q = self.replies.lock().unwrap();
            if q.is_empty() {
                return Err("empty script".into());
            }
            let text = q.remove(0);
            Ok(MemberCallResult {
                text,
                usage: ProviderUsage::new("test-model".to_string(), Default::default()),
            })
        }
    }

    #[derive(Clone, Copy)]
    enum FakeCall {
        Success,
        Error,
        MissingUsage,
        Wait,
    }

    struct FakeProvider {
        manager: Arc<SessionManager>,
        session_id: String,
        tier: CostTier,
        config: ModelConfig,
        call: FakeCall,
        dispatches: Arc<AtomicUsize>,
        saw_pending: Arc<AtomicBool>,
        started: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl CouncilProvider for FakeProvider {
        fn cost_tier(&self) -> CostTier {
            self.tier
        }

        fn model_config(&self) -> ModelConfig {
            self.config.clone()
        }

        fn retry_config(&self) -> RetryConfig {
            RetryConfig::default()
        }

        async fn complete(
            &self,
            _session_id: &str,
            _system: &str,
            _messages: &[Message],
        ) -> Result<(Message, ProviderUsage), String> {
            let pool = self
                .manager
                .pool_clone()
                .await
                .map_err(|error| error.to_string())?;
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM cost_reservations WHERE session_id = ? AND state = 'pending'",
            )
            .bind(&self.session_id)
            .fetch_one(&pool)
            .await
            .map_err(|error| error.to_string())?;
            self.saw_pending.store(pending > 0, Ordering::SeqCst);
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            match self.call {
                FakeCall::Success => Ok((
                    Message::assistant().with_text("fake response"),
                    ProviderUsage::new(
                        "gpt-4o".to_string(),
                        Usage {
                            input_tokens: Some(10),
                            output_tokens: Some(5),
                            total_tokens: Some(15),
                            ..Default::default()
                        },
                    ),
                )),
                FakeCall::Error => Err("fake provider failure".to_string()),
                FakeCall::MissingUsage => Ok((
                    Message::assistant().with_text("usage omitted"),
                    ProviderUsage::new("gpt-4o".to_string(), Usage::default()),
                )),
                FakeCall::Wait => std::future::pending().await,
            }
        }
    }

    struct FakeFactory {
        provider: Arc<FakeProvider>,
    }

    #[async_trait::async_trait]
    impl CouncilProviderFactory for FakeFactory {
        async fn create(
            &self,
            _provider: &str,
            _model: &str,
        ) -> Result<Arc<dyn CouncilProvider>, String> {
            Ok(self.provider.clone())
        }
    }

    async fn test_session() -> (TempDir, Arc<SessionManager>, Session) {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "Council provider seam".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        manager.begin_budget_task(&session.id).await.unwrap();
        let session = manager.get_session(&session.id, false).await.unwrap();
        (temp, manager, session)
    }

    fn fake_config() -> ModelConfig {
        ModelConfig {
            model_name: "gpt-4o".to_string(),
            context_limit: Some(1_000),
            max_tokens: Some(100),
            ..Default::default()
        }
    }

    fn fake_caller(
        manager: Arc<SessionManager>,
        session: &Session,
        tier: CostTier,
        call: FakeCall,
        timeout: std::time::Duration,
        budget: crate::cost_router::budget::BudgetConfig,
    ) -> (LiveCaller, Arc<AtomicUsize>, Arc<AtomicBool>, Arc<Notify>) {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let saw_pending = Arc::new(AtomicBool::new(false));
        let started = Arc::new(Notify::new());
        let provider = Arc::new(FakeProvider {
            manager: manager.clone(),
            session_id: session.id.clone(),
            tier,
            config: fake_config(),
            call,
            dispatches: dispatches.clone(),
            saw_pending: saw_pending.clone(),
            started: started.clone(),
        });
        let caller = LiveCaller::new_with_factory(
            manager,
            session.id.clone(),
            Arc::new(FakeFactory { provider }),
            timeout,
            budget,
        );
        (caller, dispatches, saw_pending, started)
    }

    fn normal_budget() -> crate::cost_router::budget::BudgetConfig {
        crate::cost_router::budget::budget_config_from(
            Some(2.0),
            Some(5.0),
            Some(10.0),
            Some(10.0),
            Some(25.0),
            Some(50.0),
        )
    }

    async fn reservation_state(manager: &SessionManager, session: &Session) -> Option<String> {
        let pool = manager.pool_clone().await.unwrap();
        sqlx::query_scalar(
            "SELECT state FROM cost_reservations WHERE session_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&session.id)
        .fetch_optional(&pool)
        .await
        .unwrap()
    }

    async fn wait_for_unknown(manager: &SessionManager, session: &Session) {
        for _ in 0..100 {
            if reservation_state(manager, session).await.as_deref() == Some("unknown") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("reservation did not become unknown");
    }

    #[tokio::test]
    async fn live_caller_reserves_before_dispatch_and_settles_exact_usage() {
        let (_temp, manager, session) = test_session().await;
        let (caller, dispatches, saw_pending, _) = fake_caller(
            manager.clone(),
            &session,
            CostTier::PaidApi,
            FakeCall::Success,
            std::time::Duration::from_millis(100),
            normal_budget(),
        );
        let result = MemberCaller::complete(&caller, "openai", "gpt-4o", "sys", "user")
            .await
            .unwrap();
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        assert!(saw_pending.load(Ordering::SeqCst));
        assert_eq!(
            reservation_state(&manager, &session).await.as_deref(),
            Some("settled")
        );
        let pool = manager.pool_clone().await.unwrap();
        let (input, output, provider, model): (i64, i64, String, String) =
            sqlx::query_as("SELECT input_tokens, output_tokens, provider, model FROM cost_ledger WHERE session_id = ?")
                .bind(&session.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!((input, output), (10, 5));
        assert_eq!(
            (provider, model),
            ("openai".to_string(), "gpt-4o".to_string())
        );
        assert!(result.usage.invocation_id.is_some());
        assert_eq!(
            manager
                .get_session(&session.id, false)
                .await
                .unwrap()
                .accumulated_total_tokens,
            Some(15)
        );
    }

    #[tokio::test]
    async fn live_caller_marks_provider_error_and_missing_usage_unknown() {
        for call in [FakeCall::Error, FakeCall::MissingUsage] {
            let (_temp, manager, session) = test_session().await;
            let (caller, dispatches, saw_pending, _) = fake_caller(
                manager.clone(),
                &session,
                CostTier::PaidApi,
                call,
                std::time::Duration::from_millis(100),
                normal_budget(),
            );
            assert!(
                MemberCaller::complete(&caller, "openai", "gpt-4o", "sys", "user")
                    .await
                    .is_err()
            );
            assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            assert!(saw_pending.load(Ordering::SeqCst));
            assert_eq!(
                reservation_state(&manager, &session).await.as_deref(),
                Some("unknown")
            );
        }
    }

    #[tokio::test]
    async fn live_caller_marks_timeout_and_cancellation_unknown() {
        let (_temp, manager, session) = test_session().await;
        let (caller, _, _, _started) = fake_caller(
            manager.clone(),
            &session,
            CostTier::PaidApi,
            FakeCall::Wait,
            std::time::Duration::from_millis(10),
            normal_budget(),
        );
        assert!(
            MemberCaller::complete(&caller, "openai", "gpt-4o", "sys", "user")
                .await
                .is_err()
        );
        assert_eq!(
            reservation_state(&manager, &session).await.as_deref(),
            Some("unknown")
        );

        let (_temp, manager, session) = test_session().await;
        let (caller, _, _, started) = fake_caller(
            manager.clone(),
            &session,
            CostTier::PaidApi,
            FakeCall::Wait,
            std::time::Duration::from_secs(30),
            normal_budget(),
        );
        let task = tokio::spawn(async move {
            MemberCaller::complete(&caller, "openai", "gpt-4o", "sys", "user").await
        });
        started.notified().await;
        task.abort();
        let _ = task.await;
        wait_for_unknown(&manager, &session).await;
    }

    #[tokio::test]
    async fn gate_refusal_never_dispatches_or_creates_a_hold() {
        let (_temp, manager, session) = test_session().await;
        let (caller, dispatches, _, _) = fake_caller(
            manager.clone(),
            &session,
            CostTier::PaidApi,
            FakeCall::Success,
            std::time::Duration::from_millis(100),
            crate::cost_router::budget::budget_config_from(
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
            ),
        );
        assert!(
            MemberCaller::complete(&caller, "openai", "gpt-4o", "sys", "user")
                .await
                .is_err()
        );
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
        assert_eq!(reservation_state(&manager, &session).await, None);
    }

    #[tokio::test]
    async fn local_and_subscription_calls_skip_holds_but_keep_tier_attribution() {
        for tier in [CostTier::LocalFree, CostTier::Subscription] {
            let (_temp, manager, session) = test_session().await;
            let (caller, dispatches, saw_pending, _) = fake_caller(
                manager.clone(),
                &session,
                tier,
                FakeCall::Success,
                std::time::Duration::from_millis(100),
                normal_budget(),
            );
            MemberCaller::complete(&caller, "local-test", "local-model", "sys", "user")
                .await
                .unwrap();
            assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            assert!(!saw_pending.load(Ordering::SeqCst));
            assert_eq!(reservation_state(&manager, &session).await, None);
            let pool = manager.pool_clone().await.unwrap();
            let stored_tier: String =
                sqlx::query_scalar("SELECT cost_tier FROM cost_ledger WHERE session_id = ?")
                    .bind(&session.id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(stored_tier, tier.as_str());
        }
    }

    fn member(p: &str) -> Member {
        Member {
            provider: p.into(),
            display_name: p.into(),
            model: "m".into(),
        }
    }

    #[test]
    fn extracts_json_from_fenced_prose() {
        let text = "Sure.\n```json\n{\"projects_need_attention\":[\"Permagent\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[],\"confidence\":0.8}\n```";
        let take = parse_round1(text).unwrap();
        assert_eq!(take.projects_need_attention, vec!["Permagent"]);
        assert_eq!(take.confidence, Some(0.8));
    }

    #[test]
    fn one_member_error_does_not_block_parse_of_the_rest() {
        let ok = MemberResult {
            member: member("a"),
            status: "ok".into(),
            raw: Some("{\"projects_need_attention\":[\"X\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[]}".into()),
            parsed: None,
            error: None,
            usage: None,
        };
        let err = MemberResult {
            member: member("b"),
            status: "error".into(),
            raw: None,
            parsed: None,
            error: Some("boom".into()),
            usage: None,
        };
        assert!(any_ok(&[ok, err]));
    }

    #[test]
    fn chair_caps_actions_at_five() {
        let actions: Vec<String> = (0..8).map(|i| format!("{{\"title\":\"a{i}\"}}")).collect();
        let json = format!(
            "{{\"headline\":\"H\",\"markdown\":\"# hi\",\"consensus\":[],\"dissent\":[],\"actions\":[{}]}}",
            actions.join(",")
        );
        let report = parse_chair(&json);
        assert_eq!(report.actions.len(), MAX_ACTIONS);
        assert_eq!(report.headline, "H");
    }

    #[test]
    fn malformed_dag_does_not_erase_a_useful_report() {
        let json = r##"{
          "headline":"Keep the report",
          "markdown":"# Useful synthesis",
          "consensus":["Ship surgically"],
          "dissent":[],
          "actions":[],
          "dag":{"budget_limit":"not-a-number","nodes":[]}
        }"##;
        let report = parse_chair(json);
        assert_eq!(report.headline, "Keep the report");
        assert_eq!(report.consensus, vec!["Ship surgically"]);
        assert!(report.dag.is_none());
    }

    /// RED-FIRST (a): an unparseable / absent verdict must NOT be silently
    /// accepted. Before the nag path existed, `run_chair` returned the report
    /// verbatim and nothing anywhere noticed the chair never ruled.
    #[tokio::test]
    async fn chair_without_a_verdict_line_is_flagged_not_silently_accepted() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nEverything looks fine.\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                // The re-ask is answered with prose, not a verdict line.
                "I would rather not commit.".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(
            report.markdown.contains("NO VERDICT LINE"),
            "an absent verdict must surface, got: {}",
            report.markdown
        );
        assert!(report.verdict_missing);
        // The flag must not read back as a ruling.
        assert!(verdict::parse(&report.markdown).is_err());
        // The original report is preserved, not replaced by the complaint.
        assert!(report.markdown.contains("Everything looks fine."));
    }

    /// The nag recovers: one narrow re-ask, and the canonical line is appended.
    #[tokio::test]
    async fn a_missing_verdict_is_re_asked_once_and_recovered() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nBody.\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                "VERDICT: HOLD — do less this week and finish the migration".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(!report.verdict_missing);
        assert!(!report.markdown.contains("NO VERDICT LINE"));
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Hold);
        assert_eq!(v.rationale, "do less this week and finish the migration");
    }

    /// A compliant chair costs exactly one call — the nag never fires.
    #[tokio::test]
    async fn a_ruled_report_is_not_re_asked() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nBody.\\n\\nVERDICT: ACT — \
                  file the two homepage cards\",\"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                // Deliberately poisoned: if this is ever consumed, the assert
                // below fails.
                "VERDICT: HOLD — the nag fired when it should not have".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        assert!(!report.verdict_missing);
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Act);
        assert_eq!(v.rationale, "file the two homepage cards");
    }

    /// A malformed value is treated exactly like an absent one — the strict
    /// parser is what makes the field queryable.
    #[tokio::test]
    async fn a_malformed_verdict_value_takes_the_nag_path() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"headline\":\"H\",\"markdown\":\"# Report\\n\\nVERDICT: MAYBE — it depends\",\
                  \"consensus\":[],\"dissent\":[],\"actions\":[]}"
                    .into(),
                "VERDICT: WATCH — nothing to start, keep an eye on churn".into(),
            ]),
        };
        let report = run_chair(&caller, &member("chair"), "brief", &[], &[])
            .await
            .unwrap();
        let v = verdict::parse(&report.markdown).unwrap();
        assert_eq!(v.verdict, verdict::Verdict::Watch);
    }

    #[test]
    fn the_chair_prompt_states_the_marker_convention() {
        let system = chair_system();
        assert!(system.contains(verdict::VERDICT_MARKER));
        assert!(system.contains("ACT|WATCH|HOLD"));
    }

    #[tokio::test]
    async fn round1_survives_a_failing_member() {
        let caller = Scripted {
            replies: Mutex::new(vec![
                "{\"projects_need_attention\":[\"P\"],\"signs_to_recognize\":[],\"missing_patterns\":[],\"promising_analytics\":[]}".into(),
            ]),
        };
        // Second complete will fail (empty script) — only one reply queued, two members.
        let out = run_round1_parallel(&caller, &[member("ok"), member("fail")], "brief").await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|r| r.status == "ok"));
        assert!(out.iter().any(|r| r.status == "error"));
    }
}
