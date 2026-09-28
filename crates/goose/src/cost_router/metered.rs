use std::sync::Arc;

use crate::cost_router::budget::{load_budget_config, BudgetScope};
use crate::session::{
    budget_task_id, goal_id, CostLedgerRow, CostReservationOutcome, CostTier, Session,
    SessionManager,
};

#[derive(Debug, Clone, Copy)]
pub enum MeteredCallKind {
    Decision,
}

pub struct MeteredCallRequest {
    pub call_id: String,
    pub kind: MeteredCallKind,
    pub backend: String,
    pub model: Option<String>,
    pub estimated_input_tokens: u64,
    pub input_token_upper_bound: u64,
    pub input_usd_per_mtok: f64,
}

#[derive(Debug)]
enum MeteredHold {
    Reserved { reservation_id: String },
    LocalFree,
}

#[derive(Debug)]
pub struct MeteredCallContext {
    call_id: String,
    kind: MeteredCallKind,
    backend: String,
    model: Option<String>,
    session_id: String,
    parent_session_id: Option<String>,
    task_id: Option<String>,
    goal_id: Option<String>,
    goal_attempt_id: Option<String>,
    is_headless: bool,
    estimated_input_tokens: u64,
    input_usd_per_mtok: f64,
    hold: MeteredHold,
}

#[derive(Debug, thiserror::Error)]
pub enum MeteredCallError {
    #[error("session {session_id} has no durable task identity")]
    MissingTaskIdentity { session_id: String },
    #[error("task identity is not bound to session {session_id}")]
    TaskIdentityMismatch { session_id: String },
    #[error("{scope:?} budget ceiling exhausted")]
    CeilingExhausted {
        scope: BudgetScope,
        spent_usd: f64,
        held_usd: f64,
        requested_usd: f64,
        ceiling_usd: f64,
        approval_required: bool,
    },
    #[error("invalid input-token bound or rate: {reason}")]
    InvalidBound { reason: String },
    #[error("budget cannot be established: {reason}")]
    BudgetUnknown { reason: String },
    #[error("call identity already used: {call_id}")]
    InvocationNotFresh { call_id: String },
    #[error("accounting storage failed")]
    Storage(#[source] anyhow::Error),
}

fn input_cost_usd(tokens: u64, rate_per_mtok: f64) -> Result<f64, MeteredCallError> {
    let cost = (tokens as f64 / 1_000_000.0) * rate_per_mtok;
    if !cost.is_finite() || cost < 0.0 {
        return Err(MeteredCallError::InvalidBound {
            reason: "input bound arithmetic overflowed or was not finite".to_string(),
        });
    }
    Ok(cost)
}

fn to_i64(value: u64, field: &str) -> Result<i64, MeteredCallError> {
    i64::try_from(value).map_err(|_| MeteredCallError::InvalidBound {
        reason: format!("{field} exceeds i64"),
    })
}

fn to_i32(value: i64, field: &str) -> Result<i32, MeteredCallError> {
    i32::try_from(value).map_err(|_| MeteredCallError::InvalidBound {
        reason: format!("{field} exceeds i32"),
    })
}

async fn checked_task_id(
    manager: Arc<SessionManager>,
    session: &Session,
) -> Result<String, MeteredCallError> {
    let Some(task_id) = budget_task_id(&session.extension_data) else {
        return Err(MeteredCallError::MissingTaskIdentity {
            session_id: session.id.clone(),
        });
    };
    let persisted = manager
        .get_session(&session.id, false)
        .await
        .map_err(MeteredCallError::Storage)?;
    if budget_task_id(&persisted.extension_data).as_deref() != Some(task_id.as_str()) {
        return Err(MeteredCallError::TaskIdentityMismatch {
            session_id: session.id.clone(),
        });
    }
    Ok(task_id)
}

fn validate_request(request: &MeteredCallRequest) -> Result<(), MeteredCallError> {
    if request.call_id.trim().is_empty() {
        return Err(MeteredCallError::InvalidBound {
            reason: "call_id must not be empty".to_string(),
        });
    }
    if !request.input_usd_per_mtok.is_finite() || request.input_usd_per_mtok <= 0.0 {
        return Err(MeteredCallError::InvalidBound {
            reason: "input rate must be finite and positive".to_string(),
        });
    }
    if request.input_token_upper_bound == 0 {
        return Err(MeteredCallError::InvalidBound {
            reason: "input token upper bound must be positive".to_string(),
        });
    }
    if request.estimated_input_tokens > request.input_token_upper_bound {
        return Err(MeteredCallError::InvalidBound {
            reason: "estimated input tokens exceed the upper bound".to_string(),
        });
    }
    Ok(())
}

pub async fn reserve_metered_call(
    manager: Arc<SessionManager>,
    session: &Session,
    request: MeteredCallRequest,
) -> Result<MeteredCallContext, MeteredCallError> {
    validate_request(&request)?;
    let requested_usd =
        input_cost_usd(request.input_token_upper_bound, request.input_usd_per_mtok)?;
    let pool = manager
        .pool_clone()
        .await
        .map_err(MeteredCallError::Storage)?;
    let binding = crate::native_provider_calls::active_worker_binding(&pool, &session.id)
        .await
        .map_err(MeteredCallError::Storage)?;
    if binding.is_some() && request.model.as_deref().map_or(true, str::is_empty) {
        return Err(MeteredCallError::BudgetUnknown {
            reason: "native metered provider call has no exact model identity".to_string(),
        });
    }
    let task_id = if let Some(binding) = &binding {
        binding.task_id.clone()
    } else {
        checked_task_id(Arc::clone(&manager), session).await?
    };
    let lease_until = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let outcome = manager
        .reserve_provider_invocation_with_identity(
            &request.call_id,
            &session.id,
            Some(&task_id),
            requested_usd,
            &lease_until,
            &load_budget_config(),
            &crate::session::session_manager::ProviderReservationIdentity {
                goal_attempt_id: binding.as_ref().map(|binding| binding.attempt_id.clone()),
                provider: request.backend.clone(),
                model: request.model.clone().unwrap_or_default(),
            },
        )
        .await
        .map_err(MeteredCallError::Storage)?;

    let reservation_id = match outcome {
        CostReservationOutcome::Granted { reservation_id } => reservation_id,
        CostReservationOutcome::AlreadyReserved { .. }
        | CostReservationOutcome::AlreadySettled { .. } => {
            return Err(MeteredCallError::InvocationNotFresh {
                call_id: request.call_id,
            })
        }
        CostReservationOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => {
            return Err(MeteredCallError::CeilingExhausted {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                approval_required: true,
            })
        }
        CostReservationOutcome::Refused {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => {
            return Err(MeteredCallError::CeilingExhausted {
                scope,
                spent_usd,
                held_usd,
                requested_usd,
                ceiling_usd,
                approval_required: false,
            })
        }
        CostReservationOutcome::Unknown { reason } => {
            return Err(MeteredCallError::BudgetUnknown { reason })
        }
    };

    if let Some(binding) = &binding {
        let pool = manager
            .pool_clone()
            .await
            .map_err(MeteredCallError::Storage)?;
        crate::native_provider_calls::record(
            &pool,
            &request.call_id,
            binding,
            &request.backend,
            request.model.as_deref().unwrap_or_default(),
        )
        .await
        .map_err(MeteredCallError::Storage)?;
    }

    if !manager
        .mark_provider_invocation_dispatch_possible(&reservation_id)
        .await
        .map_err(MeteredCallError::Storage)?
    {
        return Err(MeteredCallError::BudgetUnknown {
            reason: "metered provider reservation disappeared before dispatch".to_string(),
        });
    }

    Ok(MeteredCallContext {
        call_id: request.call_id,
        kind: request.kind,
        backend: request.backend,
        model: request.model,
        session_id: session.id.clone(),
        parent_session_id: session.parent_session_id.clone(),
        task_id: Some(task_id),
        goal_id: binding
            .as_ref()
            .map(|binding| binding.goal_id.clone())
            .or_else(|| goal_id(&session.extension_data)),
        goal_attempt_id: binding.map(|binding| binding.attempt_id),
        is_headless: !session.session_type.is_interactive(),
        estimated_input_tokens: request.estimated_input_tokens,
        input_usd_per_mtok: request.input_usd_per_mtok,
        hold: MeteredHold::Reserved { reservation_id },
    })
}

#[cfg(test)]
async fn unreserved_local_call(
    manager: Arc<SessionManager>,
    session: &Session,
    call_id: String,
    kind: MeteredCallKind,
    backend: String,
    model: Option<String>,
    estimated_input_tokens: u64,
) -> MeteredCallContext {
    let binding = match manager.pool_clone().await {
        Ok(pool) => crate::native_provider_calls::active_worker_binding(&pool, &session.id)
            .await
            .ok()
            .flatten(),
        Err(_) => None,
    };
    local_call_context(
        session,
        call_id,
        kind,
        backend,
        model,
        estimated_input_tokens,
        binding,
    )
}

/// Record native LocalFree routes before transport. This deliberately returns
/// an error if route provenance cannot be durably written; a local model call
/// must not proceed on posthoc accounting alone.
pub(crate) async fn record_local_provider_call(
    manager: Arc<SessionManager>,
    session: &Session,
    provider: &str,
    model: &str,
    estimated_input_tokens: u64,
) -> Result<MeteredCallContext, MeteredCallError> {
    let call_id = uuid::Uuid::now_v7().to_string();
    let pool = manager
        .pool_clone()
        .await
        .map_err(MeteredCallError::Storage)?;
    let binding = crate::native_provider_calls::active_worker_binding(&pool, &session.id)
        .await
        .map_err(MeteredCallError::Storage)?;
    if let Some(binding) = &binding {
        crate::native_provider_calls::record(&pool, &call_id, binding, provider, model)
            .await
            .map_err(MeteredCallError::Storage)?;
    }
    Ok(local_call_context(
        session,
        call_id,
        MeteredCallKind::Decision,
        provider.to_string(),
        Some(model.to_string()),
        estimated_input_tokens,
        binding,
    ))
}

pub(crate) async fn mark_metered_call_dispatch_possible(
    manager: Arc<SessionManager>,
    call: &MeteredCallContext,
) -> Result<(), MeteredCallError> {
    let Some(expected_attempt_id) = call.goal_attempt_id.as_deref() else {
        return Ok(());
    };
    let pool = manager
        .pool_clone()
        .await
        .map_err(MeteredCallError::Storage)?;
    let binding = crate::native_provider_calls::active_worker_binding(&pool, &call.session_id)
        .await
        .map_err(MeteredCallError::Storage)?
        .ok_or_else(|| MeteredCallError::BudgetUnknown {
            reason: "native worker binding disappeared before decision dispatch".to_string(),
        })?;
    if binding.attempt_id != expected_attempt_id
        || Some(binding.goal_id.as_str()) != call.goal_id.as_deref()
        || Some(binding.task_id.as_str()) != call.task_id.as_deref()
    {
        return Err(MeteredCallError::BudgetUnknown {
            reason: "native worker binding changed before decision dispatch".to_string(),
        });
    }
    crate::native_provider_calls::mark_dispatch_possible(&pool, &call.call_id, &binding)
        .await
        .map_err(MeteredCallError::Storage)
}

fn local_call_context(
    session: &Session,
    call_id: String,
    kind: MeteredCallKind,
    backend: String,
    model: Option<String>,
    estimated_input_tokens: u64,
    binding: Option<crate::goal_attempts::WorkerBinding>,
) -> MeteredCallContext {
    MeteredCallContext {
        call_id,
        kind,
        backend,
        model,
        session_id: session.id.clone(),
        parent_session_id: session.parent_session_id.clone(),
        task_id: binding
            .as_ref()
            .map(|binding| binding.task_id.clone())
            .or_else(|| budget_task_id(&session.extension_data)),
        goal_id: binding
            .as_ref()
            .map(|binding| binding.goal_id.clone())
            .or_else(|| goal_id(&session.extension_data)),
        goal_attempt_id: binding.map(|binding| binding.attempt_id),
        is_headless: !session.session_type.is_interactive(),
        estimated_input_tokens,
        input_usd_per_mtok: 0.0,
        hold: MeteredHold::LocalFree,
    }
}

pub enum MeteredCallFinish {
    Usage {
        input_tokens: u64,
        is_estimated: bool,
    },
    NotSent,
    UsageUnknown,
}

fn ledger_row_for(
    call: &MeteredCallContext,
    input_tokens: u64,
    is_estimated: bool,
) -> Result<CostLedgerRow, MeteredCallError> {
    let input_i64 = to_i64(input_tokens, "input_tokens")?;
    let (cost_tier, input_cost, cost_usd) = match call.hold {
        MeteredHold::Reserved { .. } => {
            let usd = input_cost_usd(input_tokens, call.input_usd_per_mtok)?;
            (CostTier::PaidApi, usd, usd)
        }
        MeteredHold::LocalFree => (CostTier::LocalFree, 0.0, 0.0),
    };

    Ok(CostLedgerRow {
        call_id: call.call_id.clone(),
        ts: chrono::Utc::now().to_rfc3339(),
        session_id: call.session_id.clone(),
        parent_session_id: call.parent_session_id.clone(),
        task_id: call.task_id.clone(),
        goal_id: call.goal_id.clone(),
        goal_attempt_id: call.goal_attempt_id.clone(),
        subagent_id: None,
        provider: Some(call.backend.clone()),
        model: call.model.clone(),
        cost_tier,
        is_headless: call.is_headless,
        input_tokens: input_i64,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        input_cost,
        output_cost: 0.0,
        cache_read_cost: 0.0,
        cache_write_cost: 0.0,
        cost_usd,
        cache_savings_usd: 0.0,
        is_estimated,
        provenance: Default::default(),
        outcome: Default::default(),
    })
}

pub async fn finish_metered_call(
    manager: Arc<SessionManager>,
    call: &MeteredCallContext,
    finish: MeteredCallFinish,
) -> Result<(), MeteredCallError> {
    let _ = call.kind;
    match finish {
        MeteredCallFinish::NotSent => {
            if call.goal_attempt_id.is_some() {
                let pool = manager
                    .pool_clone()
                    .await
                    .map_err(MeteredCallError::Storage)?;
                crate::native_provider_calls::mark_not_sent(&pool, &call.call_id)
                    .await
                    .map_err(MeteredCallError::Storage)?;
            }
            if let MeteredHold::Reserved { reservation_id } = &call.hold {
                manager
                    .release_provider_invocation(reservation_id)
                    .await
                    .map_err(MeteredCallError::Storage)?;
            }
            Ok(())
        }
        MeteredCallFinish::UsageUnknown => {
            if let MeteredHold::Reserved { reservation_id } = &call.hold {
                manager
                    .mark_provider_invocation_unknown(reservation_id)
                    .await
                    .map_err(MeteredCallError::Storage)?;
            }
            Ok(())
        }
        MeteredCallFinish::Usage {
            input_tokens,
            is_estimated,
        } => {
            let settled_input_tokens = if is_estimated && input_tokens == 0 {
                call.estimated_input_tokens
            } else {
                input_tokens
            };
            let row = ledger_row_for(call, settled_input_tokens, is_estimated)?;
            let input_i32 = to_i32(row.input_tokens, "input_tokens")?;
            let total_i32 = input_i32;
            match &call.hold {
                MeteredHold::Reserved { reservation_id } => manager
                    .settle_provider_invocation(
                        reservation_id,
                        &row,
                        None, // schedule_id
                        None, // current_total
                        None, // current_input
                        None, // current_output — the worker omitted this one
                        total_i32,
                        input_i32,
                        0,
                    )
                    .await
                    .map_err(MeteredCallError::Storage)?,
                MeteredHold::LocalFree => manager
                    .append_usage_and_rollup(&row, None, None, None, None, total_i32, input_i32, 0)
                    .await
                    .map_err(MeteredCallError::Storage)?,
            };
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseMode;
    use crate::session::{
        ExtensionData, SessionType, BUDGET_TASK_EXTENSION_NAME, BUDGET_TASK_EXTENSION_VERSION,
    };
    use tempfile::TempDir;

    async fn fixture() -> (TempDir, Arc<SessionManager>, Session) {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let session = manager
            .create_session(
                dir.path().to_path_buf(),
                "metered-test".to_string(),
                SessionType::Hidden,
                GooseMode::Auto,
            )
            .await
            .unwrap();
        (dir, manager, session)
    }

    fn make_paid_row(
        session: &Session,
        call_id: &str,
        task_id: Option<String>,
        cost_usd: f64,
    ) -> CostLedgerRow {
        CostLedgerRow {
            call_id: call_id.to_string(),
            ts: chrono::Utc::now().to_rfc3339(),
            session_id: session.id.clone(),
            parent_session_id: session.parent_session_id.clone(),
            task_id,
            goal_id: None,
            goal_attempt_id: None,
            subagent_id: None,
            provider: Some("seed".to_string()),
            model: Some("seed-model".to_string()),
            cost_tier: CostTier::PaidApi,
            is_headless: true,
            input_tokens: 1,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            input_cost: cost_usd,
            output_cost: 0.0,
            cache_read_cost: 0.0,
            cache_write_cost: 0.0,
            cost_usd,
            cache_savings_usd: 0.0,
            is_estimated: false,
            provenance: Default::default(),
            outcome: Default::default(),
        }
    }

    fn request(call_id: &str, bound: u64, estimated: u64) -> MeteredCallRequest {
        MeteredCallRequest {
            call_id: call_id.to_string(),
            kind: MeteredCallKind::Decision,
            backend: "judge-backend".to_string(),
            model: Some("judge-1".to_string()),
            estimated_input_tokens: estimated,
            input_token_upper_bound: bound,
            input_usd_per_mtok: 2.0,
        }
    }

    #[tokio::test]
    async fn missing_identity_rejects_before_reservation() {
        let (_dir, manager, session) = fixture().await;
        let err = reserve_metered_call(Arc::clone(&manager), &session, request("m1", 100, 80))
            .await
            .unwrap_err();
        assert!(matches!(err, MeteredCallError::MissingTaskIdentity { .. }));

        let pool = manager.pool_clone().await.unwrap();
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = 'm1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn mismatched_identity_rejects_before_reservation() {
        let (_dir, manager, session) = fixture().await;
        let _task_id = manager.begin_budget_task(&session.id).await.unwrap();
        let persisted = manager.get_session(&session.id, false).await.unwrap();
        let mut tampered = persisted.clone();
        let mut extension = ExtensionData::new();
        extension.set_extension_state(
            BUDGET_TASK_EXTENSION_NAME,
            BUDGET_TASK_EXTENSION_VERSION,
            serde_json::Value::String("different-task".to_string()),
        );
        tampered.extension_data = extension;

        let err = reserve_metered_call(Arc::clone(&manager), &tampered, request("m2", 100, 80))
            .await
            .unwrap_err();
        assert!(matches!(err, MeteredCallError::TaskIdentityMismatch { .. }));

        let pool = manager.pool_clone().await.unwrap();
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = 'm2'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn exhausted_ceiling_rejects_before_reservation() {
        let (_dir, manager, session) = fixture().await;
        let task_id = manager.begin_budget_task(&session.id).await.unwrap();
        manager
            .append_cost_ledger(&make_paid_row(
                &session,
                "seed-call",
                Some(task_id.clone()),
                10.0,
            ))
            .await
            .unwrap();
        let bounded = manager.get_session(&session.id, false).await.unwrap();
        let err = reserve_metered_call(Arc::clone(&manager), &bounded, request("m3", 100, 80))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            MeteredCallError::CeilingExhausted {
                approval_required: false,
                ..
            }
        ));

        let pool = manager.pool_clone().await.unwrap();
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE invocation_id = 'm3'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0);
    }

    #[tokio::test]
    async fn storage_failure_is_surfaced() {
        let (_dir, manager, session) = fixture().await;
        manager.begin_budget_task(&session.id).await.unwrap();
        let bound = manager.get_session(&session.id, false).await.unwrap();
        let pool = manager.pool_clone().await.unwrap();
        pool.close().await;

        let err = reserve_metered_call(Arc::clone(&manager), &bound, request("m4", 100, 80))
            .await
            .unwrap_err();
        assert!(matches!(err, MeteredCallError::Storage(_)));
    }

    #[tokio::test]
    async fn local_free_call_writes_non_chargeable_row_without_task_identity() {
        let (_dir, manager, session) = fixture().await;
        manager
            .append_cost_ledger(&make_paid_row(&session, "seed-session", None, 100.0))
            .await
            .unwrap();
        let local = unreserved_local_call(
            Arc::clone(&manager),
            &session,
            "local-1".to_string(),
            MeteredCallKind::Decision,
            "local-judge".to_string(),
            Some("offline".to_string()),
            33,
        )
        .await;
        finish_metered_call(
            Arc::clone(&manager),
            &local,
            MeteredCallFinish::Usage {
                input_tokens: 33,
                is_estimated: false,
            },
        )
        .await
        .unwrap();

        let pool = manager.pool_clone().await.unwrap();
        let (chargeable, cost_usd): (i64, f64) = sqlx::query_as(
            "SELECT is_chargeable, cost_usd FROM cost_ledger WHERE call_id = 'local-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(chargeable, 0);
        assert_eq!(cost_usd, 0.0);
    }

    #[tokio::test]
    async fn reused_call_id_is_rejected_and_settlement_is_idempotent() {
        let (_dir, manager, session) = fixture().await;
        manager.begin_budget_task(&session.id).await.unwrap();
        let bound = manager.get_session(&session.id, false).await.unwrap();
        let call = reserve_metered_call(Arc::clone(&manager), &bound, request("dup-1", 100, 80))
            .await
            .unwrap();

        let err = reserve_metered_call(Arc::clone(&manager), &bound, request("dup-1", 100, 80))
            .await
            .unwrap_err();
        assert!(matches!(err, MeteredCallError::InvocationNotFresh { .. }));

        finish_metered_call(
            Arc::clone(&manager),
            &call,
            MeteredCallFinish::Usage {
                input_tokens: 120,
                is_estimated: false,
            },
        )
        .await
        .unwrap();
        finish_metered_call(
            Arc::clone(&manager),
            &call,
            MeteredCallFinish::Usage {
                input_tokens: 120,
                is_estimated: false,
            },
        )
        .await
        .unwrap();

        let pool = manager.pool_clone().await.unwrap();
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cost_ledger WHERE call_id = 'dup-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn overrun_is_recorded_honestly_not_clamped() {
        let (_dir, manager, session) = fixture().await;
        manager.begin_budget_task(&session.id).await.unwrap();
        let bound = manager.get_session(&session.id, false).await.unwrap();
        let call = reserve_metered_call(Arc::clone(&manager), &bound, request("ovr-1", 100, 80))
            .await
            .unwrap();

        finish_metered_call(
            Arc::clone(&manager),
            &call,
            MeteredCallFinish::Usage {
                input_tokens: 250,
                is_estimated: false,
            },
        )
        .await
        .unwrap();

        let pool = manager.pool_clone().await.unwrap();
        let (tokens, cost): (i64, f64) = sqlx::query_as(
            "SELECT input_tokens, cost_usd FROM cost_ledger WHERE call_id = 'ovr-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tokens, 250);
        assert!((cost - (250.0 / 1_000_000.0 * 2.0)).abs() < 1e-12);
    }
}
