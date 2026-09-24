//! Admission and durable Inbox identity, before the report row or working event.
use super::{
    admission::*,
    convene_plan::{fingerprint, CallKind, ConvenePlan},
    debate::{MemberCallResult, MemberCaller},
    membership::Member,
    store::Trigger,
    ConveneReservation,
};
use chrono::Datelike;
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct ConveneBlocked {
    pub accepted: bool,
    pub code: &'static str,
    pub request_id: String,
    pub decision_id: Option<String>,
    pub message: String,
    pub retryable_after_approval: bool,
    #[serde(skip)]
    pub persistence_failed: bool,
}
impl std::fmt::Display for ConveneBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(id) = &self.decision_id {
            write!(f, " (Decision Inbox: {id})")?;
        }
        Ok(())
    }
}
impl std::error::Error for ConveneBlocked {}
impl ConveneBlocked {
    fn unknown(request_id: &str, message: impl Into<String>) -> Self {
        Self {
            accepted: false,
            code: "council_budget_unknown",
            request_id: request_id.into(),
            decision_id: None,
            message: message.into(),
            retryable_after_approval: false,
            persistence_failed: false,
        }
    }
    fn storage(request_id: &str, message: impl Into<String>) -> Self {
        Self {
            persistence_failed: true,
            ..Self::unknown(request_id, message)
        }
    }
    fn refused(request_id: &str, message: impl Into<String>) -> Self {
        Self {
            code: "council_budget_refused",
            ..Self::unknown(request_id, message)
        }
    }
    fn gate(plan: &ConvenePlan, decision_id: String) -> Self {
        Self {
            code: "council_budget_approval_required",
            decision_id: Some(decision_id),
            retryable_after_approval: true,
            ..Self::unknown(&plan.request_id, gate_copy(plan))
        }
    }
}

pub struct PreparedConvene {
    pub(crate) plan: ConvenePlan,
    pub(crate) owner_session_id: String,
    pub(crate) trigger: Trigger,
    pub(crate) question: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) permit: ConvenePermit,
    pub(crate) reservation: ConveneReservation,
}

pub struct ConvenePermit {
    admission: Arc<dyn ConveneAdmission>,
    batch_id: String,
    legacy: bool,
    closed: std::sync::atomic::AtomicBool,
}
impl ConvenePermit {
    pub async fn close(&self) -> Result<ConveneCloseSummary, ConveneAdmissionError> {
        let summary = self.admission.close(&self.batch_id).await?;
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(summary)
    }
}

impl Drop for ConvenePermit {
    fn drop(&mut self) {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let admission = self.admission.clone();
        let batch_id = self.batch_id.clone();
        // Cancellation releases only unclaimed allocations. Claimed calls
        // keep their unknown-hold lifecycle; the durable lease covers restart.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = admission.close(&batch_id).await {
                    tracing::warn!(%batch_id, %error, "could not close abandoned Council batch");
                }
            });
        }
    }
}

/// All entry points share this construction site. K1 remains open while this
/// implementation records no aggregate reservations.
pub async fn prepare(
    pool: &Pool<Sqlite>,
    trigger: Trigger,
    question: Option<&str>,
    project: Option<&str>,
    caller: &dyn MemberCaller,
    reservation: ConveneReservation,
) -> Result<PreparedConvene, ConveneBlocked> {
    // K1b: replace LegacyAdmission with the durable batch adapter, and set legacy=false.
    let admission: Arc<dyn ConveneAdmission> = Arc::new(LegacyAdmission);
    let members = super::membership::resolve_members().await;
    let (provider, model) = super::membership::chair_route();
    let chair = Member {
        provider,
        model,
        display_name: "Chair".into(),
    };
    prepare_with(
        pool,
        caller,
        reservation,
        PrepareInput {
            trigger,
            question: question.map(str::to_owned),
            project: project.map(str::to_owned),
            members,
            chair,
            admission,
            legacy: true,
        },
    )
    .await
}

pub(crate) struct PrepareInput {
    pub trigger: Trigger,
    pub question: Option<String>,
    pub project: Option<String>,
    pub members: Vec<Member>,
    pub chair: Member,
    pub admission: Arc<dyn ConveneAdmission>,
    pub legacy: bool,
}

/// Sunday and Monday catch-up identify the same scheduled occurrence.
pub fn weekly_occurrence(now: chrono::DateTime<chrono::Local>) -> String {
    (now.date_naive() - chrono::Duration::days(i64::from(now.weekday().num_days_from_sunday())))
        .to_string()
}

pub(crate) async fn prepare_with(
    pool: &Pool<Sqlite>,
    caller: &dyn MemberCaller,
    reservation: ConveneReservation,
    input: PrepareInput,
) -> Result<PreparedConvene, ConveneBlocked> {
    let context = caller.context().ok_or_else(|| {
        ConveneBlocked::unknown("", "Council requires a harness session for attribution")
    })?;
    let parent = context
        .manager
        .get_session(&context.session_id, false)
        .await
        .map_err(|e| ConveneBlocked::unknown("", e.to_string()))?;
    let identity = match input.trigger {
        Trigger::Weekly => {
            serde_json::json!(["council", "weekly", weekly_occurrence(chrono::Local::now())])
        }
        Trigger::OnDemand => serde_json::json!([
            "council",
            "on_demand",
            parent.id,
            crate::session::budget_task_id(&parent.extension_data),
            input.question,
            input.project
        ]),
    };
    let request_id = fingerprint(&identity);
    let storage = |e: sqlx::Error| ConveneBlocked::storage(&request_id, e.to_string());
    if super::store::has_running(pool)
        .await
        .map_err(|e| ConveneBlocked::unknown(&request_id, e))?
    {
        return Err(ConveneBlocked::refused(
            &request_id,
            "a council session is already running",
        ));
    }
    let plan = ConvenePlan::build(request_id.clone(), input.members, input.chair, caller)
        .await
        .map_err(|e| ConveneBlocked::unknown(&request_id, e))?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS council_convene_requests (
        request_id TEXT PRIMARY KEY, owner_session_id TEXT NOT NULL, plan_hash TEXT NOT NULL,
        bound_usd REAL NOT NULL, decision_id TEXT, state TEXT NOT NULL DEFAULT 'pending')",
    )
    .execute(pool)
    .await
    .map_err(storage)?;
    // A second daemon must not create a second card while create_decision's
    // transaction and the request-pointer update straddle a crash or race.
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS council_convene_decision_identity ON decisions (json_extract(payload_json, '$.proposal')) WHERE kind = 'choice' AND json_extract(payload_json, '$.proposal') LIKE 'council_convene:%'")
        .execute(pool).await.map_err(storage)?;
    let existing = sqlx::query(
        "SELECT owner_session_id, state FROM council_convene_requests WHERE request_id = ?",
    )
    .bind(&request_id)
    .fetch_optional(pool)
    .await
    .map_err(storage)?;
    let owner_session_id = if let Some(row) = existing {
        let state: String = row.try_get("state").map_err(storage)?;
        if state != "pending" {
            return Err(ConveneBlocked::refused(
                &request_id,
                "This Council occurrence was already admitted or cancelled; it cannot run again",
            ));
        }
        row.try_get::<String, _>("owner_session_id")
            .map_err(storage)?
    } else {
        // This child, never the coordinator, owns the stable task. The request
        // row reuses it across gates/retries; per-seat calls do not reset it.
        let child = context
            .manager
            .create_session_with_parent(
                Some(&parent.id),
                parent.working_dir.clone(),
                "Council convene".into(),
                crate::session::SessionType::SubAgent,
                parent.goose_mode,
            )
            .await
            .map_err(|e| ConveneBlocked::storage(&request_id, e.to_string()))?;
        context
            .manager
            .begin_budget_task(&child.id)
            .await
            .map_err(|e| ConveneBlocked::storage(&request_id, e.to_string()))?;
        sqlx::query("INSERT INTO council_convene_requests(request_id, owner_session_id, plan_hash, bound_usd) VALUES (?, ?, ?, ?)")
            .bind(&request_id).bind(&child.id).bind(&plan.plan_fingerprint).bind(plan.bound_usd)
            .execute(pool).await.map_err(storage)?;
        child.id
    };
    // Search the decision's durable identity as well as the request row. If a
    // crash followed create_decision but preceded the pointer update, retry
    // recovers the card instead of creating another.
    let request_marker = format!("council_convene:{request_id}:%");
    let plan_marker = format!("council_convene:{request_id}:{}", plan.plan_fingerprint);
    let cancelled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM decisions WHERE json_extract(payload_json, '$.proposal') LIKE ? AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ? AND answer_choice_id = 'cancel' AND status <> 'open')")
        .bind(&request_marker).bind(&owner_session_id).fetch_one(pool).await.map_err(storage)?;
    if cancelled {
        sqlx::query("UPDATE council_convene_requests SET state = 'cancelled' WHERE request_id = ?")
            .bind(&request_id)
            .execute(pool)
            .await
            .map_err(storage)?;
        return Err(ConveneBlocked::refused(
            &request_id,
            "Council authorization was cancelled for this occurrence",
        ));
    }
    let card_id: Option<String> = sqlx::query_scalar("SELECT id FROM decisions WHERE json_extract(payload_json, '$.proposal') = ? AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ? ORDER BY created_at DESC LIMIT 1")
        .bind(&plan_marker).bind(&owner_session_id).fetch_optional(pool).await.map_err(storage)?;
    let mut approval = None;
    if let Some(id) = card_id.as_deref() {
        let card = crate::decisions::get_decision(pool, id)
            .await
            .map_err(|e| ConveneBlocked::storage(&request_id, e))?
            .ok_or_else(|| ConveneBlocked::storage(&request_id, "Council decision disappeared"))?;
        if card.status == "open" {
            return Err(ConveneBlocked::gate(&plan, id.into()));
        }
        if card.answer_choice_id.as_deref() == Some("approve")
            && card.answer.as_deref() == Some("choice")
        {
            approval = Some(ConveneGateApproval {
                decision_id: id.into(),
                plan_hash: plan.plan_fingerprint.clone(),
            });
        } else {
            return Err(ConveneBlocked::refused(
                &request_id,
                "Council decision did not authorize this plan",
            ));
        }
    }
    let previous_card: Option<String> = if card_id.is_none() {
        sqlx::query_scalar("SELECT id FROM decisions WHERE json_extract(payload_json, '$.proposal') LIKE ? AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ? ORDER BY created_at DESC LIMIT 1")
            .bind(&request_marker).bind(&owner_session_id).fetch_optional(pool).await.map_err(storage)?
    } else {
        None
    };
    // A changed scope always needs a fresh bounded choice, even if its new
    // total would fall below the soft gate. No old approval reaches admission.
    let outcome = if let Some(id) = previous_card {
        let previous = crate::decisions::get_decision(pool, &id)
            .await
            .map_err(|e| ConveneBlocked::storage(&request_id, e))?
            .ok_or_else(|| ConveneBlocked::storage(&request_id, "Council decision disappeared"))?;
        let evidence = &previous.payload["reservation_gate"];
        ConveneAdmissionOutcome::NeedsGate {
            scope: if evidence["scope"].as_str() == Some("session") {
                crate::cost_router::budget::BudgetScope::Session
            } else {
                crate::cost_router::budget::BudgetScope::Task
            },
            spent_usd: evidence["settled_usd"].as_f64().ok_or_else(|| {
                ConveneBlocked::unknown(&request_id, "Previous gate has no settled evidence")
            })?,
            held_usd: evidence["held_usd"].as_f64().ok_or_else(|| {
                ConveneBlocked::unknown(&request_id, "Previous gate has no held evidence")
            })?,
            requested_usd: plan.bound_usd,
            ceiling_usd: evidence["ceiling_usd"].as_f64().ok_or_else(|| {
                ConveneBlocked::unknown(&request_id, "Previous gate has no ceiling evidence")
            })?,
        }
    } else {
        input
            .admission
            .admit(ConveneBatchRequest {
                request_id: request_id.clone(),
                plan_hash: plan.plan_fingerprint.clone(),
                owner_session_id: owner_session_id.clone(),
                slots: plan.slots.iter().map(|s| s.planned.clone()).collect(),
                lease_until: (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),
                gate_approval: approval,
            })
            .await
            .map_err(|e| ConveneBlocked::unknown(&request_id, e.to_string()))?
    };
    let batch_id = match outcome {
        ConveneAdmissionOutcome::Admitted { batch_id, .. } => batch_id,
        ConveneAdmissionOutcome::AlreadyAdmitted { .. } => return Err(ConveneBlocked::refused(&request_id, "This Council occurrence was already admitted; replay cannot launch another run")),
        ConveneAdmissionOutcome::PlanChanged { .. } => return Err(ConveneBlocked::refused(&request_id, "The admitted Council plan changed; start a new convene occurrence")),
        ConveneAdmissionOutcome::Unknown { reason } => return Err(ConveneBlocked::unknown(&request_id, reason)),
        ConveneAdmissionOutcome::Refused { spent_usd, held_usd, requested_usd, ceiling_usd, .. } => return Err(ConveneBlocked::refused(&request_id,
            format!("Council hard limit refused authorization: settled ${spent_usd:.2}; holds ${held_usd:.2}; requested ${requested_usd:.2}; ceiling ${ceiling_usd:.2}. Approval cannot override this limit."))),
        ConveneAdmissionOutcome::NeedsGate { scope, spent_usd, held_usd, requested_usd, ceiling_usd } => {
            let mut request = crate::cost_router::budget::with_requesting_session_id(
                crate::cost_router::budget::reservation_gate_decision_request(scope, spent_usd, held_usd, requested_usd, ceiling_usd, 0.0, None, None), &owner_session_id);
            let copy = gate_copy(&plan);
            request.detail = Some(format!("{copy}\n\nSettled spend and holds are the last admission observation; admission rechecks safety on explicit re-convene."));
            request.payload["question"] = copy.into();
            request.payload["options"] = serde_json::json!([
                {"id": "approve", "label": "Approve bounded Council convene"},
                {"id": "cancel", "label": "Cancel this occurrence"}
            ]);
            request.payload["default"] = "cancel".into();
            // ChoicePayload denies unknown fields. Its existing proposal
            // marker stores the durable request+plan identity without changing
            // a shared decision schema or relying on prose parsing.
            request.payload["proposal"] = plan_marker.clone().into();
            let card = match crate::decisions::create_decision(pool, request).await {
                Ok(card) => card,
                Err(error) => {
                    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM decisions WHERE kind = 'choice' AND json_extract(payload_json, '$.proposal') = ? AND json_extract(payload_json, '$.reservation_gate.requesting_session_id') = ?")
                        .bind(&plan_marker).bind(&owner_session_id).fetch_optional(pool).await.map_err(storage)?;
                    if let Some(id) = existing { return Err(ConveneBlocked::gate(&plan, id)); }
                    return Err(ConveneBlocked::storage(&request_id, error));
                }
            };
            if card.kind != "choice" { return Err(ConveneBlocked::storage(&request_id, "Council authorization card was rejected as malformed")); }
            sqlx::query("UPDATE council_convene_requests SET plan_hash = ?, bound_usd = ?, decision_id = ? WHERE request_id = ?")
                .bind(&plan.plan_fingerprint).bind(plan.bound_usd).bind(&card.id).bind(&request_id).execute(pool).await.map_err(storage)?;
            return Err(ConveneBlocked::gate(&plan, card.id));
        }
    };
    let permit = ConvenePermit {
        admission: input.admission,
        batch_id,
        legacy: input.legacy,
        closed: std::sync::atomic::AtomicBool::new(false),
    };
    let changed = sqlx::query("UPDATE council_convene_requests SET state = 'admitted', plan_hash = ?, bound_usd = ? WHERE request_id = ? AND state = 'pending'")
        .bind(&plan.plan_fingerprint).bind(plan.bound_usd).bind(&request_id).execute(pool).await.map_err(storage)?.rows_affected();
    if changed != 1 {
        permit
            .close()
            .await
            .map_err(|e| ConveneBlocked::unknown(&request_id, e.to_string()))?;
        return Err(ConveneBlocked::refused(
            &request_id,
            "Council occurrence already consumed",
        ));
    }
    Ok(PreparedConvene {
        plan,
        owner_session_id,
        trigger: input.trigger,
        question: input.question,
        project: input.project,
        permit,
        reservation,
    })
}

fn gate_copy(plan: &ConvenePlan) -> String {
    format!("Authorize up to ${} for one Council convene: {} seats, two rounds, chair synthesis and one possible verdict re-ask; approval does not start it — convene again to run", plan.bound_usd, plan.members.len())
}

pub(crate) struct PlannedCaller<'a> {
    pub caller: &'a dyn MemberCaller,
    pub prepared: &'a PreparedConvene,
}
#[async_trait::async_trait]
impl MemberCaller for PlannedCaller<'_> {
    async fn complete(
        &self,
        _provider: &str,
        _model: &str,
        _system: &str,
        _user: &str,
    ) -> Result<MemberCallResult, String> {
        Err("Council dispatch requires a frozen call slot".into())
    }
    async fn complete_slot(
        &self,
        kind: CallKind,
        member: &Member,
        system: &str,
        user: &str,
    ) -> Result<MemberCallResult, String> {
        let slot = self
            .prepared
            .plan
            .slots
            .iter()
            .find(|s| {
                s.kind == kind
                    && s.planned.provider == member.provider
                    && s.planned.model.as_deref() == Some(member.model.as_str())
            })
            .ok_or_else(|| "Council call is absent from the admitted plan".to_string())?;
        let context = self
            .caller
            .context()
            .ok_or_else(|| "Council caller lost its harness context".to_string())?;
        let owner = context
            .manager
            .get_session(&self.prepared.owner_session_id, false)
            .await
            .map_err(|e| e.to_string())?;
        // create_session_with_parent copies the owner's task identity. Never
        // begin a second task for an invocation or mutate the coordinator.
        let child = context
            .manager
            .create_session_with_parent(
                Some(&owner.id),
                owner.working_dir,
                format!("Council {}", slot.planned.slot_id),
                crate::session::SessionType::SubAgent,
                owner.goose_mode,
            )
            .await
            .map_err(|e| e.to_string())?;
        let reservation_id = if !self.prepared.permit.legacy && slot.planned.bound_usd.is_some() {
            match self
                .prepared
                .permit
                .admission
                .claim(
                    &self.prepared.permit.batch_id,
                    &slot.planned.slot_id,
                    &child.id,
                )
                .await
                .map_err(|e| e.to_string())?
            {
                ConveneClaimOutcome::Claimed { reservation_id, .. } => Some(reservation_id),
                other => return Err(format!("Council slot cannot dispatch: {other:?}")),
            }
        } else {
            None
        };
        self.caller
            .complete_planned(slot, &child.id, reservation_id.as_deref(), system, user)
            .await
    }
}

/// The sweep's parent is stable as well, so an hourly retry creates no new
/// harness sessions while waiting for the occurrence's Inbox answer.
pub async fn weekly_parent(manager: &crate::session::SessionManager) -> Result<String, String> {
    let pool = manager.pool_clone().await.map_err(|e| e.to_string())?;
    sqlx::query("CREATE TABLE IF NOT EXISTS council_weekly_parents (occurrence TEXT PRIMARY KEY, session_id TEXT NOT NULL)")
        .execute(&pool).await.map_err(|e| e.to_string())?;
    let occurrence = weekly_occurrence(chrono::Local::now());
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT session_id FROM council_weekly_parents WHERE occurrence = ?",
    )
    .bind(&occurrence)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    {
        return Ok(id);
    }
    let session = manager
        .create_session(
            std::env::current_dir().unwrap_or_default(),
            "Council weekly".into(),
            crate::session::SessionType::Scheduled,
            crate::config::GooseMode::Auto,
        )
        .await
        .map_err(|e| e.to_string())?;
    // Two daemons can both miss the SELECT above. The first INSERT wins; the
    // loser adopts the winner's parent and drops its own empty session, so
    // the occurrence still has exactly one attribution parent.
    sqlx::query(
        "INSERT OR IGNORE INTO council_weekly_parents(occurrence, session_id) VALUES (?, ?)",
    )
    .bind(&occurrence)
    .bind(&session.id)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let winner: String =
        sqlx::query_scalar("SELECT session_id FROM council_weekly_parents WHERE occurrence = ?")
            .bind(&occurrence)
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())?;
    if winner != session.id {
        if let Err(e) = manager.delete_session(&session.id).await {
            tracing::warn!(target: "permagentd::council", "could not drop the losing weekly parent session {}: {e}", session.id);
        }
    }
    Ok(winner)
}

#[cfg(test)]
mod tests {
    use super::super::admission::scripted::ScriptedAdmission;
    use super::*;
    use crate::{
        model::ModelConfig,
        session::{CostTier, SessionManager},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Scripted {
        context: super::super::debate::CouncilCallContext,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl MemberCaller for Scripted {
        fn context(&self) -> Option<super::super::debate::CouncilCallContext> {
            Some(self.context.clone())
        }
        async fn plan_model(
            &self,
            member: &Member,
        ) -> Result<(ModelConfig, CostTier, u32), String> {
            Ok((
                ModelConfig {
                    model_name: member.model.clone(),
                    context_limit: Some(1000),
                    max_tokens: Some(100),
                    ..Default::default()
                },
                CostTier::PaidApi,
                2,
            ))
        }
        async fn complete(
            &self,
            _provider: &str,
            _model: &str,
            _system: &str,
            _user: &str,
        ) -> Result<MemberCallResult, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err("unexpected dispatch".into())
        }
    }
    async fn fixture() -> (tempfile::TempDir, Pool<Sqlite>, Scripted) {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_owned()));
        let parent = manager
            .create_session(
                dir.path().to_owned(),
                "Council parent".into(),
                crate::session::SessionType::User,
                crate::config::GooseMode::Auto,
            )
            .await
            .unwrap();
        manager.begin_budget_task(&parent.id).await.unwrap();
        let pool = manager.pool_clone().await.unwrap();
        (
            dir,
            pool,
            Scripted {
                context: super::super::debate::CouncilCallContext {
                    manager,
                    session_id: parent.id,
                },
                calls: AtomicUsize::new(0),
            },
        )
    }
    fn input(admission: Arc<dyn ConveneAdmission>) -> PrepareInput {
        PrepareInput {
            trigger: Trigger::Weekly,
            question: None,
            project: None,
            members: (0..11)
                .map(|i| Member {
                    provider: format!("seat-{i}"),
                    model: "gpt-4o".into(),
                    display_name: format!("Seat {i}"),
                })
                .collect(),
            chair: Member {
                provider: "openai".into(),
                model: "gpt-4o".into(),
                display_name: "Chair".into(),
            },
            admission,
            legacy: false,
        }
    }
    async fn prepare_test(
        pool: &Pool<Sqlite>,
        caller: &Scripted,
        input: PrepareInput,
    ) -> Result<PreparedConvene, ConveneBlocked> {
        let reservation = ConveneReservation {
            _guard: super::super::session_lock().lock().await,
        };
        prepare_with(pool, caller, reservation, input).await
    }
    fn blocked(result: Result<PreparedConvene, ConveneBlocked>) -> ConveneBlocked {
        match result {
            Err(e) => e,
            Ok(_) => panic!("expected refusal before execution"),
        }
    }
    async fn answer(pool: &Pool<Sqlite>, id: &str, choice: &str) {
        crate::decisions::answer_decision(
            pool,
            id,
            &crate::decisions::DecisionAnswer {
                answer: "choice".into(),
                note: None,
                choice_id: Some(choice.into()),
                input_text: None,
            },
            crate::decisions::ACTOR_JESSE,
        )
        .await
        .unwrap();
    }
    async fn count(pool: &Pool<Sqlite>, table: &str) -> i64 {
        // Table names are fixed test literals, never user input.
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(pool)
            .await
            .unwrap()
    }
    async fn gated(caller: &Scripted) -> Arc<ScriptedAdmission> {
        let trial = input(Arc::new(ScriptedAdmission::new(0.0, 1000.0)));
        let plan = ConvenePlan::build("trial".into(), trial.members, trial.chair, caller)
            .await
            .unwrap();
        let max = plan
            .slots
            .iter()
            .filter_map(|s| s.planned.bound_usd)
            .fold(0.0, f64::max);
        assert_eq!(plan.slots.len(), 24);
        assert!(plan.bound_usd > max * 2.0);
        Arc::new(ScriptedAdmission::new(max * 2.0, plan.bound_usd * 2.0))
    }
    /// Ruling K1 acceptance: repeated weekly sweeps reuse the occurrence's
    /// parent — an hourly retry, or a second daemon racing the first, must not
    /// leave a new harness session behind each time.
    #[tokio::test]
    async fn weekly_parent_is_one_session_per_occurrence_even_when_raced() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_owned()));
        let first = weekly_parent(&manager).await.unwrap();
        assert_eq!(weekly_parent(&manager).await.unwrap(), first);

        let pool = manager.pool_clone().await.unwrap();
        sqlx::query("DELETE FROM council_weekly_parents")
            .execute(&pool)
            .await
            .unwrap();
        let before = manager.list_all_sessions().await.unwrap().len();
        let (a, b) = tokio::join!(weekly_parent(&manager), weekly_parent(&manager));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b, "racing sweeps must agree on one parent");
        assert_eq!(
            manager.list_all_sessions().await.unwrap().len(),
            before + 1,
            "the losing sweep's session must not survive"
        );
    }

    #[tokio::test]
    async fn eleven_affordable_seats_gate_before_row_working_or_dispatch_and_reuse_card() {
        let _serial = super::super::test_lock().lock().await;
        let (_dir, pool, caller) = fixture().await;
        let admission = gated(&caller).await;
        let mut events = crate::events::subscribe();
        let first = blocked(prepare_test(&pool, &caller, input(admission.clone())).await);
        assert_eq!(first.code, "council_budget_approval_required");
        assert_eq!(caller.calls.load(Ordering::SeqCst), 0);
        assert_eq!(count(&pool, "council_sessions").await, 0);
        assert_eq!(count(&pool, "decisions").await, 1);
        while let Ok(event) = events.try_recv() {
            assert!(
                !(event.payload["agent_id"] == "council" && event.payload["state"] == "working")
            );
        }
        let session_count = count(&pool, "sessions").await;
        // A new manager instance simulates restart; no in-memory card cache.
        let restarted = Scripted {
            context: super::super::debate::CouncilCallContext {
                manager: Arc::new(SessionManager::new(_dir.path().to_owned())),
                session_id: caller.context.session_id.clone(),
            },
            calls: AtomicUsize::new(0),
        };
        let repeat = blocked(prepare_test(&pool, &restarted, input(admission)).await);
        assert_eq!(repeat.request_id, first.request_id);
        assert_eq!(repeat.decision_id, first.decision_id);
        assert_eq!(count(&pool, "decisions").await, 1);
        assert_eq!(count(&pool, "sessions").await, session_count);
    }
    #[tokio::test]
    async fn changed_plan_gets_fresh_card_and_cancel_stops_occurrence() {
        let _serial = super::super::test_lock().lock().await;
        let (_dir, pool, caller) = fixture().await;
        let admission = gated(&caller).await;
        let first = blocked(prepare_test(&pool, &caller, input(admission.clone())).await);
        answer(&pool, first.decision_id.as_deref().unwrap(), "approve").await;
        let mut changed = input(admission.clone());
        changed.chair.model = "gpt-4.1".into();
        let second = blocked(prepare_test(&pool, &caller, changed).await);
        assert_eq!(second.code, "council_budget_approval_required");
        assert_ne!(first.decision_id, second.decision_id);
        assert_eq!(count(&pool, "decisions").await, 2);
        answer(&pool, second.decision_id.as_deref().unwrap(), "cancel").await;
        let cancelled = blocked(prepare_test(&pool, &caller, input(admission)).await);
        assert_eq!(cancelled.code, "council_budget_refused");
        assert_eq!(count(&pool, "council_sessions").await, 0);
        assert_eq!(caller.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn approval_reuses_child_task_and_replay_cannot_run_again() {
        let _serial = super::super::test_lock().lock().await;
        let (_dir, pool, caller) = fixture().await;
        let parent_before = caller
            .context
            .manager
            .get_session(&caller.context.session_id, false)
            .await
            .unwrap();
        let admission = gated(&caller).await;
        let first = blocked(prepare_test(&pool, &caller, input(admission.clone())).await);
        let child: String =
            sqlx::query_scalar("SELECT owner_session_id FROM council_convene_requests")
                .fetch_one(&pool)
                .await
                .unwrap();
        let child_before = caller
            .context
            .manager
            .get_session(&child, false)
            .await
            .unwrap();
        answer(&pool, first.decision_id.as_deref().unwrap(), "approve").await;
        let ready = prepare_test(&pool, &caller, input(admission.clone()))
            .await
            .unwrap();
        assert_eq!(ready.owner_session_id, child);
        assert_eq!(ready.plan.slots.len(), 24);
        assert_eq!(caller.calls.load(Ordering::SeqCst), 0);
        assert_eq!(count(&pool, "council_sessions").await, 0);
        let child_after = caller
            .context
            .manager
            .get_session(&child, false)
            .await
            .unwrap();
        let parent_after = caller
            .context
            .manager
            .get_session(&caller.context.session_id, false)
            .await
            .unwrap();
        assert_eq!(
            crate::session::budget_task_id(&child_before.extension_data),
            crate::session::budget_task_id(&child_after.extension_data)
        );
        assert_eq!(
            crate::session::budget_task_id(&parent_before.extension_data),
            crate::session::budget_task_id(&parent_after.extension_data)
        );
        assert_ne!(
            crate::session::budget_task_id(&parent_after.extension_data),
            crate::session::budget_task_id(&child_after.extension_data)
        );
        ready.permit.close().await.unwrap();
        drop(ready);
        assert_eq!(
            blocked(prepare_test(&pool, &caller, input(admission)).await).code,
            "council_budget_refused"
        );
        assert_eq!(count(&pool, "decisions").await, 1);
    }
    #[tokio::test]
    async fn decision_persistence_failure_is_not_acceptance() {
        let _serial = super::super::test_lock().lock().await;
        let (_dir, pool, caller) = fixture().await;
        sqlx::query("CREATE TRIGGER fail_council_decision BEFORE INSERT ON decisions BEGIN SELECT RAISE(FAIL, 'scripted decision storage failure'); END")
            .execute(&pool).await.unwrap();
        let result = blocked(prepare_test(&pool, &caller, input(gated(&caller).await)).await);
        assert!(result.persistence_failed);
        assert_eq!(count(&pool, "council_sessions").await, 0);
        assert_eq!(caller.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn test_provider_chair_is_refused_before_admission() {
        let _serial = super::super::test_lock().lock().await;
        let (_dir, pool, caller) = fixture().await;
        let mut request = input(gated(&caller).await);
        request.chair.provider = "permagent-test-custom-headers".into();
        let refused = blocked(prepare_test(&pool, &caller, request).await);
        assert_eq!(
            refused.message,
            "reserved test providers cannot sit on or chair the Council"
        );
        assert_eq!(count(&pool, "decisions").await, 0);
        assert_eq!(count(&pool, "council_sessions").await, 0);
    }
}
