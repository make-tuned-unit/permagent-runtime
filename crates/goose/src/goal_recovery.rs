//! Inspect-only native worker loss recovery. Absence of a heartbeat, mutable
//! receipt or clean checkout is never sufficient ownership/candidate authority.
use anyhow::Result;
use sqlx::{Pool, Row, Sqlite};
use std::sync::Arc;

use crate::execution::manager::AgentManager;
use crate::goal_candidates::{self, LineageResolution, TerminalObservation, TerminalState};

pub(crate) async fn apply_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let columns = sqlx::query("PRAGMA table_info(goal_attempts)")
        .fetch_all(&mut *tx)
        .await?;
    if !columns.iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|value| value == "owner_process_id")
    }) {
        sqlx::query("ALTER TABLE goal_attempts ADD COLUMN owner_process_id INTEGER")
            .execute(&mut *tx)
            .await?;
    }
    if !columns.iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|value| value == "execution_kind")
    }) {
        sqlx::query("ALTER TABLE goal_attempts ADD COLUMN execution_kind TEXT")
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProcessPresence {
    Present,
    Absent,
    Unknown,
}
#[cfg(unix)]
fn process_presence(pid: i64) -> ProcessPresence {
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return ProcessPresence::Unknown;
    }
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return ProcessPresence::Present;
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        ProcessPresence::Absent
    } else {
        ProcessPresence::Unknown
    }
}
#[cfg(not(unix))]
fn process_presence(_pid: i64) -> ProcessPresence {
    ProcessPresence::Unknown
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum NativeInspection {
    LegacyUnverified,
    StillOwned(String),
    Unverifiable(String),
    Candidate(LineageResolution),
    Lost(LostWorker),
}
/// Native construction only. Re-inspected before sealing, never deserialized
/// from a goal projection or supplied root/task/worker identity.
#[derive(Debug)]
pub(crate) struct LostWorker {
    goal_id: String,
    claim_id: String,
    session_id: String,
}

pub(crate) async fn inspect_native_goal(
    pool: &Pool<Sqlite>,
    manager: Option<&Arc<AgentManager>>,
    goal: &str,
) -> Result<NativeInspection> {
    let row=sqlx::query("SELECT b.authority,b.coordinator_session_id,b.project_id,b.canonical_root,b.root_status,
        a.claim_id,a.worker_session_id,a.lifecycle_id,a.owner_process_id,a.process_id,a.execution_kind,a.execution_state,a.claimed_at,
        c.archived_at,c.column_id,json_extract(c.metadata_json,'$.dispatch_claim_id') AS current_claim,bc.state_binding
        FROM goal_execution_bindings b JOIN cards c ON c.id=b.goal_id JOIN board_columns bc ON bc.id=c.column_id
        LEFT JOIN goal_attempts a ON a.attempt_id=(SELECT attempt_id FROM goal_attempts WHERE goal_id=b.goal_id AND execution_state<>'spawn_failed' ORDER BY ordinal DESC LIMIT 1)
        WHERE b.goal_id=?")
        .bind(goal).fetch_optional(pool).await?;
    let Some(row) = row else {
        return Ok(NativeInspection::LegacyUnverified);
    };
    if row.try_get::<String, _>("authority")? != "native" {
        return Ok(NativeInspection::LegacyUnverified);
    }
    let state: Option<String> = row.try_get("execution_state")?;
    if matches!(
        state.as_deref(),
        Some("succeeded" | "failed" | "cancelled" | "lost")
    ) {
        return Ok(NativeInspection::Candidate(
            goal_candidates::resolve_candidate(pool, goal).await?,
        ));
    }
    if state.as_deref() != Some("started") {
        return Ok(NativeInspection::Unverifiable(
            "No started native worker to observe".into(),
        ));
    }
    let claim: String = row.try_get("claim_id")?;
    if row
        .try_get::<Option<String>, _>("current_claim")?
        .as_deref()
        != Some(claim.as_str())
        || row
            .try_get::<Option<String>, _>("state_binding")?
            .as_deref()
            != Some("in_progress")
        || row.try_get::<Option<String>, _>("archived_at")?.is_some()
    {
        return Ok(NativeInspection::Unverifiable("Native worker no longer owns an active goal; no loss callback may overwrite that state".into()));
    }
    let session: Option<String> = row.try_get("worker_session_id")?;
    let Some(session) = session else {
        return Ok(NativeInspection::Unverifiable(
            "Native worker session is not finalized".into(),
        ));
    };
    if let Some(manager) = manager {
        if manager.is_session_busy(&session).await {
            return Ok(NativeInspection::StillOwned(
                "Exact native worker remains busy".into(),
            ));
        }
    }
    let lifecycle: String = row.try_get("lifecycle_id")?;
    let owner: Option<i64> = row.try_get("owner_process_id")?;
    let coordinator: Option<String> = row.try_get("coordinator_session_id")?;
    let leases = sqlx::query_as::<_, crate::harness_lease::Lease>(
        "SELECT * FROM harness_process_leases WHERE lifecycle_id=?",
    )
    .bind(&lifecycle)
    .fetch_all(pool)
    .await?;
    // A trusted lease can conservatively block loss even when an old attempt
    // has no new PID field. It never backfills historical ownership authority.
    for lease in &leases {
        if crate::harness_lease::classify(lease, chrono::Utc::now())
            == crate::harness_lease::LeaseLiveness::Live
            || process_presence(lease.pid) == ProcessPresence::Present
        {
            return Ok(NativeInspection::StillOwned(
                "Native lifecycle has a live lease or process".into(),
            ));
        }
        let claimed =
            chrono::DateTime::parse_from_rfc3339(&row.try_get::<String, _>("claimed_at")?).ok();
        let started = chrono::DateTime::parse_from_rfc3339(&lease.started_at).ok();
        let beat = chrono::DateTime::parse_from_rfc3339(&lease.heartbeat_at).ok();
        let times_match = matches!((started,claimed,beat),(Some(started),Some(claimed),Some(beat)) if started<=claimed && beat>=started && beat<=chrono::Utc::now());
        if !times_match {
            return Ok(NativeInspection::Unverifiable(
                "Native lease timing or root does not match protected ownership".into(),
            ));
        }
        // CLI lease project/root fields are genuinely optional (project
        // lookup and git-toplevel can fail). Missing values confer no authority:
        // positive absence still requires this attempt's immutable owner PID.
        // When present, validate the native Git toplevel rather than comparing
        // a project subdirectory to the lease's repository root.
        if let Some(lease_root) = lease.repo_root.as_ref() {
            let Some(bound_root) = row.try_get::<Option<String>, _>("canonical_root")? else {
                return Ok(NativeInspection::Unverifiable(
                    "Lease has no matching bound execution root".into(),
                ));
            };
            let mut command = tokio::process::Command::new("git");
            command
                .args(["-C", &bound_root, "rev-parse", "--show-toplevel"])
                .kill_on_drop(true);
            let output =
                tokio::time::timeout(std::time::Duration::from_secs(10), command.output()).await;
            let matches = match output {
                Ok(Ok(output)) if output.status.success() => {
                    std::fs::canonicalize(String::from_utf8_lossy(&output.stdout).trim())
                        .ok()
                        .zip(std::fs::canonicalize(lease_root).ok())
                        .is_some_and(|(bound, leased)| bound == leased)
                }
                _ => false,
            };
            if !matches {
                return Ok(NativeInspection::Unverifiable(
                    "Lease repository differs from native execution root".into(),
                ));
            }
        }
        if lease.session_id != coordinator.as_deref().unwrap_or("")
            || lease.project_id.as_deref().is_some_and(|project| {
                row.try_get::<String, _>("project_id")
                    .map_or(true, |bound| project != bound)
            })
            || owner.is_some_and(|pid| lease.pid != pid)
        {
            return Ok(NativeInspection::Unverifiable(
                "Native lifecycle lease does not match immutable ownership".into(),
            ));
        }
    }
    let Some(owner) = owner else {
        return Ok(NativeInspection::Unverifiable(
            "Historical native attempt lacks recorded owner PID".into(),
        ));
    };
    match process_presence(owner) {
        ProcessPresence::Present => {
            return Ok(NativeInspection::StillOwned(
                "Recorded native owner process still exists".into(),
            ))
        }
        ProcessPresence::Unknown => {
            return Ok(NativeInspection::Unverifiable(
                "Owner process absence cannot be established".into(),
            ))
        }
        ProcessPresence::Absent => {}
    }
    match row
        .try_get::<Option<String>, _>("execution_kind")?
        .as_deref()
    {
        Some("internal") => {}
        Some("external") => match row
            .try_get::<Option<i64>, _>("process_id")?
            .map(process_presence)
        {
            Some(ProcessPresence::Absent) => {}
            Some(ProcessPresence::Present) => {
                return Ok(NativeInspection::StillOwned(
                    "Recorded external worker process still exists".into(),
                ))
            }
            _ => {
                return Ok(NativeInspection::Unverifiable(
                    "External child process absence cannot be established".into(),
                ))
            }
        },
        _ => {
            return Ok(NativeInspection::Unverifiable(
                "Native execution kind was not recorded".into(),
            ))
        }
    }
    Ok(NativeInspection::Lost(LostWorker {
        goal_id: goal.into(),
        claim_id: claim,
        session_id: session,
    }))
}

/// Shared boot/live loss observation: no provider call, repair dispatch, new
/// budget task or reservation mutation. Lost execution can retain a candidate
/// without asserting acceptance or successful landing.
pub(crate) async fn observe_lost_native_worker(
    pool: &Pool<Sqlite>,
    manager: Option<&Arc<AgentManager>>,
    goal: &str,
) -> Result<NativeInspection> {
    let inspection = inspect_native_goal(pool, manager, goal).await?;
    let NativeInspection::Lost(worker) = inspection else {
        return Ok(inspection);
    };
    match goal_candidates::observe_worker_terminal(
        pool,
        &worker.goal_id,
        &worker.claim_id,
        &worker.session_id,
        TerminalState::Lost,
        "Native owner and required child processes are absent; execution outcome was not observed",
        crate::agents::platform_extensions::execution_receipt::ReceiptState::Failed,
    )
    .await?
    {
        TerminalObservation::Sealed(_) | TerminalObservation::AlreadySealed(_) => Ok(
            NativeInspection::Candidate(goal_candidates::resolve_candidate(pool, goal).await?),
        ),
        TerminalObservation::Stale => Ok(NativeInspection::Unverifiable(
            "Goal changed while loss was being observed".into(),
        )),
    }
}
