//! Native goal execution identities. Card metadata is a display projection,
//! never authority for this journal. Existing goals are not backfilled.
use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite, Transaction};
use std::path::Path;

/// Trusted native dispatch input; not deserializable from tool/HTTP arguments.
#[derive(Debug, Clone)]
pub struct ClaimInput {
    pub(crate) claim_id: String,
    pub(crate) lifecycle_id: String,
    pub(crate) worker_key: String,
    pub(crate) execution_root: Option<String>,
    pub(crate) input_head: Option<String>,
}

/// Engine start capability issued only after the atomic native claim.
#[derive(Clone)]
pub struct StartPermit {
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) goal_id: String,
    pub(crate) claim_id: String,
}

pub(crate) async fn apply_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS goal_execution_bindings (
        goal_id TEXT PRIMARY KEY REFERENCES cards(id), project_id TEXT NOT NULL,
        task_id TEXT, coordinator_session_id TEXT, contract_json TEXT,
        contract_sha256 TEXT, authority TEXT NOT NULL, canonical_root TEXT, initial_git_head TEXT, root_status TEXT NOT NULL DEFAULT 'absent',
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS goal_attempts (
        attempt_id TEXT PRIMARY KEY, goal_id TEXT NOT NULL REFERENCES goal_execution_bindings(goal_id),
        ordinal INTEGER NOT NULL, claim_id TEXT NOT NULL UNIQUE, lifecycle_id TEXT NOT NULL,
        worker_key TEXT NOT NULL, execution_root TEXT, input_head TEXT,
        worker_session_id TEXT UNIQUE, worktree_path TEXT, process_id INTEGER,
        execution_state TEXT NOT NULL CHECK(execution_state IN ('claimed','prepared','started','spawn_failed','succeeded','failed','cancelled','lost')),
        claimed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        started_at TEXT, finalized_at TEXT, ended_at TEXT, terminal_reason TEXT,
        UNIQUE(goal_id, ordinal))").execute(pool).await?;
    crate::goal_candidates::apply_schema(pool).await?;
    crate::native_provider_calls::apply_schema(pool).await?;
    Ok(())
}

/// Only called while materializing a newly approved native goal. Missing
/// source evidence stays explicitly unverified, including non-coding goals.
pub(crate) async fn bind_new_goal_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal_id: &str,
    project_id: &str,
    task_id: Option<&str>,
    coordinator: Option<&str>,
    contract: Option<&serde_json::Value>,
) -> Result<()> {
    let source = if let Some(coordinator) = coordinator {
        sqlx::query("SELECT extension_data,working_dir FROM sessions WHERE id=?")
            .bind(coordinator)
            .fetch_optional(&mut **tx)
            .await?
    } else {
        None
    };
    let source_data: Option<crate::session::ExtensionData> = source
        .as_ref()
        .map(|row| row.try_get::<Option<String>, _>("extension_data"))
        .transpose()?
        .flatten()
        .map(|s| serde_json::from_str(&s))
        .transpose()?;
    let native_task = source_data
        .as_ref()
        .and_then(crate::session::budget_task_id);
    let native_contract = source_data
        .as_ref()
        .and_then(crate::session::session_manager::budget_task_contract);
    if native_task.is_some() && native_task.as_deref() != task_id {
        bail!("Native task identity differs from roadmap binding");
    }
    if let (Some(supplied), Some(native)) = (contract, native_contract) {
        if supplied != native {
            bail!("Original contract differs from native task origin");
        }
    }
    let known = native_task.is_some()
        && coordinator.is_some()
        && native_contract
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .is_some_and(|text| {
                !text.trim().is_empty()
                    && text.len() <= crate::session::session_manager::MAX_BUDGET_TASK_CONTRACT_BYTES
            });
    let exact = native_contract.map(serde_json::to_string).transpose()?;
    let digest = exact
        .as_ref()
        .map(|s| hex::encode(Sha256::digest(s.as_bytes())));
    let project_root: Option<String> =
        sqlx::query_scalar("SELECT root_path FROM projects WHERE id=?")
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    let selected_root = project_root.or_else(|| {
        source
            .as_ref()
            .and_then(|r| r.try_get::<String, _>("working_dir").ok())
    });
    let root = selected_root
        .as_ref()
        .and_then(|r| std::fs::canonicalize(r).ok())
        .map(|p| p.to_string_lossy().into_owned());
    let root_status = match (&selected_root, &root) {
        (None, _) => "absent",
        (_, Some(_)) => "resolved",
        _ => "invalid",
    };
    let head = if let Some(root) = root.as_ref() {
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    } else {
        None
    };
    sqlx::query("INSERT INTO goal_execution_bindings
        (goal_id,project_id,task_id,coordinator_session_id,contract_json,contract_sha256,authority,canonical_root,initial_git_head,root_status)
        VALUES (?,?,?,?,?,?,?,?,?,?)")
        .bind(goal_id).bind(project_id).bind(native_task).bind(coordinator).bind(exact).bind(digest)
        .bind(if known { "native" } else { "unverified" }).bind(root).bind(head).bind(root_status).execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn claim_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal_id: &str,
    project_id: &str,
    input: &ClaimInput,
) -> Result<()> {
    // Record legacy execution without manufacturing original task authority.
    sqlx::query(
        "INSERT OR IGNORE INTO goal_execution_bindings
        (goal_id,project_id,authority) VALUES (?,?,'legacy_unverified')",
    )
    .bind(goal_id)
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    let binding_project: String =
        sqlx::query_scalar("SELECT project_id FROM goal_execution_bindings WHERE goal_id=?")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    if binding_project != project_id {
        bail!("Goal journal project mismatch");
    }
    let root_status: String =
        sqlx::query_scalar("SELECT root_status FROM goal_execution_bindings WHERE goal_id=?")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    if root_status == "invalid" {
        bail!("Selected native execution root could not be resolved");
    }
    let bound_root: Option<String> =
        sqlx::query_scalar("SELECT canonical_root FROM goal_execution_bindings WHERE goal_id=?")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    if bound_root
        .as_ref()
        .is_some_and(|root| input.execution_root.as_ref() != Some(root))
    {
        bail!("Native execution root differs from frozen goal root");
    }
    let baseline = if let Some(root) = input.execution_root.as_ref() {
        crate::goal_candidates::dispatch_baseline_tx(tx, goal_id, Path::new(root)).await?
    } else {
        None
    };
    let (parent, baseline_authority) = if let Some((native_head, parent)) = baseline {
        if input.input_head != native_head {
            bail!("Dispatch baseline differs from native observed input");
        }
        (parent, "native")
    } else {
        let native_no_root:i64=sqlx::query_scalar("SELECT COUNT(*) FROM goal_execution_bindings WHERE goal_id=? AND authority='native' AND root_status='absent'")
            .bind(goal_id).fetch_one(&mut **tx).await?;
        if native_no_root == 1 && input.execution_root.is_none() && input.input_head.is_none() {
            (None, "native")
        } else {
            (None, "unverified")
        }
    };
    let ordinal: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(ordinal),0)+1 FROM goal_attempts WHERE goal_id=?")
            .bind(goal_id)
            .fetch_one(&mut **tx)
            .await?;
    sqlx::query("INSERT INTO goal_attempts
        (attempt_id,goal_id,ordinal,claim_id,lifecycle_id,worker_key,execution_root,input_head,parent_candidate_attempt_id,baseline_authority,execution_state)
        VALUES (?,?,?,?,?,?,?,?,?,?,'claimed')")
        .bind(uuid::Uuid::now_v7().to_string()).bind(goal_id).bind(ordinal).bind(&input.claim_id)
        .bind(&input.lifecycle_id).bind(&input.worker_key).bind(&input.execution_root).bind(&input.input_head).bind(parent).bind(baseline_authority)
        .execute(&mut **tx).await?;
    Ok(())
}

impl StartPermit {
    /// Actual start gate: engine awaits this before entering its provider loop
    /// or spawning an external child. No model/configured route is inferred.
    pub(crate) async fn start(&self) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let changed = sqlx::query("UPDATE goal_attempts SET execution_state='started', started_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
            WHERE goal_id=? AND claim_id=? AND execution_state='prepared' AND finalized_at IS NOT NULL
            AND EXISTS (SELECT 1 FROM cards c JOIN board_columns bc ON bc.id=c.column_id
                WHERE c.id=goal_attempts.goal_id AND json_extract(c.metadata_json,'$.dispatch_claim_id')=? AND bc.state_binding='in_progress')")
            .bind(&self.goal_id).bind(&self.claim_id).bind(&self.claim_id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            bail!("Worker cannot start before native finalization or after losing its claim");
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn mark_provider_accounting(&self, session: &str) -> Result<()> {
        let changed=sqlx::query("UPDATE goal_attempts SET accounting_authority=? WHERE goal_id=? AND claim_id=? AND worker_session_id=? AND execution_state='prepared'")
            .bind(crate::native_provider_calls::COVERAGE_VERSION).bind(&self.goal_id).bind(&self.claim_id).bind(session).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            bail!("Provider accounting cannot bind a different native worker");
        }
        Ok(())
    }

    pub(crate) async fn prepare_worker(
        &self,
        session: &str,
        worktree: &Path,
        task: Option<&str>,
    ) -> Result<()> {
        let path = std::fs::canonicalize(worktree)?
            .to_string_lossy()
            .into_owned();
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT a.execution_state,a.worker_session_id,a.execution_root,a.input_head,b.task_id,
            json_extract(c.metadata_json,'$.dispatch_claim_id') AS active_claim,
            bc.state_binding FROM goal_attempts a JOIN goal_execution_bindings b ON b.goal_id=a.goal_id
            JOIN cards c ON c.id=a.goal_id JOIN board_columns bc ON bc.id=c.column_id
            WHERE a.goal_id=? AND a.claim_id=?")
            .bind(&self.goal_id).bind(&self.claim_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| anyhow!("No native attempt claim exists"))?;
        if row.try_get::<Option<String>, _>("active_claim")?.as_deref() != Some(&self.claim_id)
            || row
                .try_get::<Option<String>, _>("state_binding")?
                .as_deref()
                != Some("in_progress")
        {
            bail!("Native claim no longer owns this goal");
        }
        let root: Option<String> = row.try_get("execution_root")?;
        if let Some(root) = root {
            let root = std::path::PathBuf::from(root);
            let expected = if row.try_get::<Option<String>, _>("input_head")?.is_some() {
                root.parent()
                    .ok_or_else(|| anyhow!("Git goal has no worktree parent"))?
                    .join(".permagent-goal-worktrees")
                    .join(session)
            } else {
                root
            };
            if std::fs::canonicalize(expected)?.to_string_lossy() != path {
                bail!("Prepared worker root does not match native claim");
            }
        }
        crate::goal_candidates::validate_prepared_head(
            Path::new(&path),
            row.try_get::<Option<String>, _>("input_head")?.as_deref(),
        )
        .await?;
        let bound_task: Option<String> = row.try_get("task_id")?;
        if bound_task.as_deref().is_some_and(|t| Some(t) != task) {
            bail!("Native worker task mismatch");
        }
        if row.try_get::<String, _>("execution_state")? != "claimed" {
            bail!("Native attempt already prepared or ended");
        }
        sqlx::query("UPDATE goal_attempts SET worker_session_id=?,worktree_path=?,execution_state='prepared' WHERE goal_id=? AND claim_id=? AND execution_state='claimed'")
            .bind(session).bind(path).bind(&self.goal_id).bind(&self.claim_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn finalize_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal: &str,
    claim: &str,
    session: Option<&str>,
    pid: Option<i64>,
) -> Result<()> {
    let row = sqlx::query("SELECT worker_session_id,execution_state,process_id,finalized_at FROM goal_attempts WHERE goal_id=? AND claim_id=?")
        .bind(goal).bind(claim).fetch_optional(&mut **tx).await?;
    let row = row.ok_or_else(|| anyhow!("Native dispatch attempt is missing"))?;
    {
        if !matches!(
            row.try_get::<String, _>("execution_state")?.as_str(),
            "prepared" | "started"
        ) || row
            .try_get::<Option<String>, _>("worker_session_id")?
            .as_deref()
            != session
        {
            bail!("Started worker does not match native dispatch finalization");
        }
        if row.try_get::<Option<String>, _>("finalized_at")?.is_some() {
            if row.try_get::<Option<i64>, _>("process_id")? != pid {
                bail!("Repeated finalization differs from native identity");
            }
            return Ok(());
        }
        sqlx::query("UPDATE goal_attempts SET process_id=?,finalized_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE goal_id=? AND claim_id=?")
            .bind(pid).bind(goal).bind(claim).execute(&mut **tx).await?;
    }
    Ok(())
}

pub(crate) async fn release_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal: &str,
    claim: &str,
    reason: &str,
) -> Result<()> {
    sqlx::query("UPDATE goal_attempts SET execution_state=CASE WHEN execution_state='started' THEN 'failed' ELSE 'spawn_failed' END,ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),terminal_reason=? WHERE goal_id=? AND claim_id=? AND execution_state IN ('claimed','prepared','started')")
        .bind(reason).bind(goal).bind(claim).execute(&mut **tx).await?;
    Ok(())
}

/// Late callbacks must match the active native claim and exact worker identity.
pub(crate) async fn is_current_worker(
    pool: &Pool<Sqlite>,
    goal: &str,
    claim: &str,
    session: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM goal_attempts a JOIN cards c ON c.id=a.goal_id
        JOIN board_columns bc ON bc.id=c.column_id
        WHERE bc.state_binding='in_progress' AND a.goal_id=? AND a.claim_id=? AND a.worker_session_id=? AND a.execution_state='started'
        AND json_extract(c.metadata_json,'$.dispatch_claim_id')=?",
    )
    .bind(goal)
    .bind(claim)
    .bind(session)
    .bind(claim)
    .fetch_one(pool)
    .await?
        == 1)
}

pub(crate) async fn record_pid(
    pool: &Pool<Sqlite>,
    goal: &str,
    claim: &str,
    pid: u32,
) -> Result<()> {
    let changed = sqlx::query("UPDATE goal_attempts SET process_id=? WHERE goal_id=? AND claim_id=? AND execution_state='started' AND finalized_at IS NOT NULL")
        .bind(i64::from(pid)).bind(goal).bind(claim).execute(pool).await?.rows_affected();
    if changed != 1 {
        bail!("PID does not belong to a started native attempt");
    }
    Ok(())
}

pub(crate) async fn cancel_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal: &str,
    claim: &str,
) -> Result<()> {
    sqlx::query("UPDATE goal_attempts SET execution_state='cancelled',ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),terminal_reason='Worker start cancelled' WHERE goal_id=? AND claim_id=? AND execution_state IN ('claimed','prepared','started')")
        .bind(goal).bind(claim).execute(&mut **tx).await?;
    Ok(())
}

/// Native-only lookup used by physical accounting. Parent/reviewer sessions
/// are never matched or inherited here. Legacy/unverified bindings are None.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerBinding {
    pub attempt_id: String,
    pub goal_id: String,
    pub task_id: String,
    pub worker_session_id: String,
}

pub(crate) async fn worker_binding(
    pool: &Pool<Sqlite>,
    session: &str,
) -> Result<Option<WorkerBinding>> {
    let row = sqlx::query("SELECT a.attempt_id,a.goal_id,b.task_id,a.worker_session_id
        FROM goal_attempts a JOIN goal_execution_bindings b ON b.goal_id=a.goal_id
        WHERE a.worker_session_id=? AND a.execution_state='started' AND b.authority='native' AND b.task_id IS NOT NULL")
        .bind(session).fetch_optional(pool).await?;
    row.map(|row| {
        Ok(WorkerBinding {
            attempt_id: row.try_get("attempt_id")?,
            goal_id: row.try_get("goal_id")?,
            task_id: row.try_get("task_id")?,
            worker_session_id: row.try_get("worker_session_id")?,
        })
    })
    .transpose()
}

/// A known model-capable delegation outside the universal native invocation
/// journal permanently invalidates this editor's coverage. No child/parent
/// inference or reviewer-persona metadata can restore automatic authority.
pub(crate) async fn mark_unreported_delegation(pool: &Pool<Sqlite>, session: &str) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let native: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_attempts a JOIN goal_execution_bindings b ON b.goal_id=a.goal_id WHERE a.worker_session_id=? AND b.authority='native'")
        .bind(session).fetch_one(&mut *tx).await?;
    if native == 0 {
        tx.commit().await?;
        return Ok(());
    }
    if native != 1 {
        bail!("Model-capable delegation has ambiguous native editor identity");
    }
    let changed = sqlx::query(
        "UPDATE goal_attempts SET accounting_authority='unreported_delegation'
        WHERE worker_session_id=? AND execution_state='started' AND EXISTS (
          SELECT 1 FROM cards c JOIN board_columns col ON col.id=c.column_id
          WHERE c.id=goal_attempts.goal_id AND col.state_binding='in_progress'
          AND json_extract(c.metadata_json,'$.dispatch_claim_id')=goal_attempts.claim_id)",
    )
    .bind(session)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        bail!("Model-capable delegation no longer owns the native editing attempt");
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
#[path = "goal_attempts_tests.rs"]
mod tests;
