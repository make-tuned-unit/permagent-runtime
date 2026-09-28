//! Native candidate observations. Git evidence is read from the journal-bound
//! execution root; model-editable card projections cannot establish lineage.
use anyhow::{anyhow, bail, Result};
use sqlx::{Pool, Row, Sqlite, Transaction};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalState {
    Succeeded,
    Failed,
    Cancelled,
    Lost,
}
impl TerminalState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCandidate {
    pub attempt_id: String,
    pub goal_id: String,
    pub worker_session_id: String,
    pub worktree: PathBuf,
    pub input_head: String,
    pub trunk_head: String,
    pub head: String,
    pub parent_attempt_id: Option<String>,
    pub substantive: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateResolution {
    Verified(NativeCandidate),
    NoGit,
    Unverifiable(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalObservation {
    Sealed(CandidateResolution),
    AlreadySealed(CandidateResolution),
    Stale,
}

pub(crate) async fn apply_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // Additive migration also works against databases initialized by slice A.
    let columns = sqlx::query("PRAGMA table_info(goal_attempts)")
        .fetch_all(&mut *tx)
        .await?;
    for (name, declaration) in [
        ("parent_candidate_attempt_id", "TEXT"),
        ("baseline_authority", "TEXT NOT NULL DEFAULT 'unverified'"),
        ("accounting_authority", "TEXT NOT NULL DEFAULT 'unverified'"),
    ] {
        if !columns
            .iter()
            .any(|row| row.try_get::<String, _>("name").is_ok_and(|s| s == name))
        {
            sqlx::query(&format!(
                "ALTER TABLE goal_attempts ADD COLUMN {name} {declaration}"
            ))
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query("CREATE TABLE IF NOT EXISTS goal_candidate_observations (
        attempt_id TEXT PRIMARY KEY REFERENCES goal_attempts(attempt_id), goal_id TEXT NOT NULL,
        parent_attempt_id TEXT REFERENCES goal_attempts(attempt_id), input_head TEXT, candidate_head TEXT,
        trunk_head TEXT, worktree_path TEXT, substantive INTEGER NOT NULL,
        provenance_state TEXT NOT NULL CHECK(provenance_state IN ('verified','unverifiable','no_git')),
        reason TEXT, observed_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%dT%H:%M:%fZ','now')))")
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    let mut command = tokio::process::Command::new("git");
    command.arg("-C").arg(root).args(args).kill_on_drop(true);
    Ok(
        tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .map_err(|_| anyhow!("Native Git observation timed out"))??,
    )
}

async fn git_head(root: &Path) -> Result<String> {
    let output = git(root, &["rev-parse", "HEAD"]).await?;
    if !output.status.success() {
        bail!("Git HEAD cannot be observed");
    }
    let head = String::from_utf8(output.stdout)?.trim().to_owned();
    if head.is_empty() {
        bail!("Git HEAD observation was empty");
    }
    Ok(head)
}

/// None means legacy/unverified compatibility execution, never verified lineage.
/// A native first attempt uses current trunk after dependency gates; retries use
/// only a sealed native parent. No user-supplied candidate SHA is accepted.
pub(crate) async fn dispatch_baseline(
    pool: &Pool<Sqlite>,
    goal: &str,
    root: &Path,
) -> Result<Option<(Option<String>, Option<String>)>> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let baseline = dispatch_baseline_tx(&mut tx, goal, root).await?;
    tx.commit().await?;
    Ok(baseline)
}
pub(crate) async fn dispatch_baseline_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal: &str,
    root: &Path,
) -> Result<Option<(Option<String>, Option<String>)>> {
    let row=sqlx::query("SELECT authority,canonical_root,root_status,initial_git_head FROM goal_execution_bindings WHERE goal_id=?")
        .bind(goal).fetch_optional(&mut **tx).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.try_get::<String, _>("authority")? != "native" {
        return Ok(None);
    }
    if row.try_get::<String, _>("root_status")? == "invalid" {
        bail!("Native execution root is unresolved");
    }
    let bound: Option<String> = row.try_get("canonical_root")?;
    if let Some(bound) = bound {
        if std::fs::canonicalize(root)?.to_string_lossy() != bound {
            bail!("Execution root differs from immutable goal binding");
        }
    }
    let prior = sqlx::query(
        "SELECT a.attempt_id,a.execution_state,o.provenance_state,o.candidate_head
        FROM goal_attempts a LEFT JOIN goal_candidate_observations o ON o.attempt_id=a.attempt_id
        WHERE a.goal_id=? AND a.execution_state<>'spawn_failed' ORDER BY a.ordinal DESC LIMIT 1",
    )
    .bind(goal)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(prior) = prior {
        let state: String = prior.try_get("execution_state")?;
        if matches!(state.as_str(), "claimed" | "prepared" | "started") {
            bail!("Prior native attempt has not been terminally observed");
        }
        match prior
            .try_get::<Option<String>, _>("provenance_state")?
            .as_deref()
        {
            Some("verified") => {
                return Ok(Some((
                    prior.try_get("candidate_head")?,
                    Some(prior.try_get("attempt_id")?),
                )))
            }
            Some("no_git") => return Ok(Some((None, Some(prior.try_get("attempt_id")?)))),
            _ => bail!(
                "Prior native candidate is unverifiable; automatic retry cannot guess its baseline"
            ),
        }
    }
    let anchor: Option<String> = row.try_get("initial_git_head")?;
    if let Some(anchor) = anchor {
        let head = git_head(root).await?;
        if !git(root, &["merge-base", "--is-ancestor", &anchor, &head])
            .await?
            .status
            .success()
        {
            bail!("Current trunk is not descended from the original native context anchor");
        }
        Ok(Some((Some(head), None)))
    } else {
        // Explicitly non-Git/no-root execution is permitted. A selected Git repo
        // whose initial HEAD was unreadable must not acquire a guessed baseline.
        let repo = git(root, &["rev-parse", "--is-inside-work-tree"]).await?;
        if repo.status.success() {
            bail!("Original Git context has no verified anchor");
        }
        Ok(Some((None, None)))
    }
}

pub(crate) async fn validate_prepared_head(worktree: &Path, expected: Option<&str>) -> Result<()> {
    if let Some(expected) = expected {
        if git_head(worktree).await? != expected {
            bail!("Prepared worktree does not start at the native input head");
        }
        let status = git(worktree, &["status", "--porcelain"]).await?;
        if !status.status.success() || !status.stdout.is_empty() {
            bail!("Prepared worktree is not pristine before execution");
        }
    }
    Ok(())
}

async fn inspect(row: &sqlx::sqlite::SqliteRow) -> Result<CandidateResolution> {
    if row.try_get::<String, _>("authority")? != "native"
        || row.try_get::<String, _>("baseline_authority")? != "native"
    {
        return Ok(CandidateResolution::Unverifiable(
            "Original task or baseline lacks native authority".into(),
        ));
    }
    let input: Option<String> = row.try_get("input_head")?;
    let Some(input) = input else {
        return Ok(CandidateResolution::NoGit);
    };
    let worktree: Option<String> = row.try_get("worktree_path")?;
    let worktree = PathBuf::from(worktree.ok_or_else(|| anyhow!("Native worker root is absent"))?);
    let canonical = std::fs::canonicalize(&worktree)?;
    if canonical != worktree {
        bail!("Native worker root changed identity");
    }
    let head = git_head(&worktree).await?;
    let status = git(&worktree, &["status", "--porcelain"]).await?;
    if !status.status.success() || !status.stdout.is_empty() {
        bail!("Candidate worktree is dirty or cannot be inspected");
    }
    if !git(&worktree, &["merge-base", "--is-ancestor", &input, &head])
        .await?
        .status
        .success()
    {
        bail!("Candidate head is not descended from this attempt's input");
    }
    let diff = git(&worktree, &["diff", "--quiet", &input, &head]).await?;
    let substantive = match diff.status.code() {
        Some(0) => false,
        Some(1) => true,
        _ => bail!("Candidate diff could not be inspected"),
    };
    if git_head(&worktree).await? != head {
        bail!("Candidate HEAD changed during observation");
    }
    let trunk: Option<String> = row.try_get("trunk_head")?;
    Ok(CandidateResolution::Verified(NativeCandidate {
        attempt_id: row.try_get("attempt_id")?,
        goal_id: row.try_get("goal_id")?,
        worker_session_id: row.try_get("worker_session_id")?,
        worktree,
        input_head: input.clone(),
        trunk_head: trunk.unwrap_or(input),
        head,
        parent_attempt_id: row.try_get("parent_candidate_attempt_id")?,
        substantive,
    }))
}

/// Seals execution failure separately from recoverable candidate evidence.
/// Exact claim/session and active column are checked in the sealing transaction.
pub(crate) async fn observe_worker_terminal(
    pool: &Pool<Sqlite>,
    goal: &str,
    claim: &str,
    session: &str,
    state: TerminalState,
    reason: &str,
    receipt_state: crate::agents::platform_extensions::execution_receipt::ReceiptState,
) -> Result<TerminalObservation> {
    let row=sqlx::query("SELECT a.*,b.authority,COALESCE(parent.trunk_head,a.input_head) AS trunk_head
        FROM goal_attempts a JOIN goal_execution_bindings b ON b.goal_id=a.goal_id
        LEFT JOIN goal_candidate_observations parent ON parent.attempt_id=a.parent_candidate_attempt_id
        WHERE a.goal_id=? AND a.claim_id=? AND a.worker_session_id=?")
        .bind(goal).bind(claim).bind(session).fetch_optional(pool).await?;
    let Some(row) = row else {
        return Ok(TerminalObservation::Stale);
    };
    let evidence = match inspect(&row).await {
        Ok(v) => v,
        Err(error) => CandidateResolution::Unverifiable(error.to_string()),
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let owns_current: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cards c JOIN board_columns bc ON bc.id=c.column_id WHERE c.id=? AND json_extract(c.metadata_json,'$.dispatch_claim_id')=? AND bc.state_binding='in_progress'")
        .bind(goal).bind(claim).fetch_one(&mut *tx).await?;
    if owns_current != 1 {
        return Ok(TerminalObservation::Stale);
    }
    let prior=sqlx::query("SELECT a.execution_state,o.* FROM goal_attempts a JOIN goal_candidate_observations o ON o.attempt_id=a.attempt_id
        WHERE a.goal_id=? AND a.claim_id=? AND a.worker_session_id=?")
        .bind(goal).bind(claim).bind(session).fetch_optional(&mut *tx).await?;
    if let Some(prior) = prior {
        let actual_state: String = prior.try_get("execution_state")?;
        let (head, provenance) = match &evidence {
            CandidateResolution::Verified(c) => (Some(c.head.as_str()), "verified"),
            CandidateResolution::NoGit => (None, "no_git"),
            CandidateResolution::Unverifiable(_) => (None, "unverifiable"),
        };
        if actual_state != state.as_str()
            || prior
                .try_get::<Option<String>, _>("candidate_head")?
                .as_deref()
                != head
            || prior.try_get::<String, _>("provenance_state")? != provenance
        {
            bail!("Repeated terminal observation conflicts with sealed evidence");
        }
        match &evidence {
            CandidateResolution::Verified(candidate) => {
                if prior.try_get::<Option<String>, _>("input_head")?.as_deref()
                    != Some(candidate.input_head.as_str())
                    || prior.try_get::<Option<String>, _>("trunk_head")?.as_deref()
                        != Some(candidate.trunk_head.as_str())
                    || prior
                        .try_get::<Option<String>, _>("worktree_path")?
                        .as_deref()
                        != Some(candidate.worktree.to_string_lossy().as_ref())
                    || prior.try_get::<Option<String>, _>("parent_attempt_id")?
                        != candidate.parent_attempt_id
                    || (prior.try_get::<i64, _>("substantive")? != 0) != candidate.substantive
                {
                    bail!("Repeated terminal observation conflicts with full sealed candidate");
                }
            }
            CandidateResolution::Unverifiable(reason)
                if prior.try_get::<Option<String>, _>("reason")?.as_deref()
                    != Some(reason.as_str()) =>
            {
                bail!("Repeated unverifiable observation changed reason")
            }
            _ => {}
        }
        return Ok(TerminalObservation::AlreadySealed(evidence));
    }
    let current: i64=sqlx::query_scalar("SELECT COUNT(*) FROM goal_attempts a JOIN cards c ON c.id=a.goal_id JOIN board_columns bc ON bc.id=c.column_id
        WHERE a.goal_id=? AND a.claim_id=? AND a.worker_session_id=? AND a.execution_state='started'
        AND bc.state_binding='in_progress' AND json_extract(c.metadata_json,'$.dispatch_claim_id')=?")
        .bind(goal).bind(claim).bind(session).bind(claim).fetch_one(&mut *tx).await?;
    if current != 1 {
        return Ok(TerminalObservation::Stale);
    }
    let (head, trunk, substantive, provenance, problem) = match &evidence {
        CandidateResolution::Verified(c) => (
            Some(c.head.as_str()),
            Some(c.trunk_head.as_str()),
            c.substantive,
            "verified",
            None,
        ),
        CandidateResolution::NoGit => (None, None, false, "no_git", None),
        CandidateResolution::Unverifiable(reason) => {
            (None, None, false, "unverifiable", Some(reason.as_str()))
        }
    };
    sqlx::query("INSERT INTO goal_candidate_observations(attempt_id,goal_id,parent_attempt_id,input_head,candidate_head,trunk_head,worktree_path,substantive,provenance_state,reason)
        VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(row.try_get::<String,_>("attempt_id")?).bind(goal).bind(row.try_get::<Option<String>,_>("parent_candidate_attempt_id")?)
        .bind(row.try_get::<Option<String>,_>("input_head")?).bind(head).bind(trunk).bind(row.try_get::<Option<String>,_>("worktree_path")?)
        .bind(substantive).bind(provenance).bind(problem).execute(&mut *tx).await?;
    sqlx::query("UPDATE goal_attempts SET execution_state=?,terminal_reason=?,ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE goal_id=? AND claim_id=? AND execution_state='started'")
        .bind(state.as_str()).bind(reason).bind(goal).bind(claim).execute(&mut *tx).await?;
    // Display receipt is projected under the same exact-claim transaction.
    // It is never consulted as candidate or contributor authority.
    use crate::agents::platform_extensions::execution_receipt::ExecutionReceipt;
    let projection: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(metadata_json,'$.execution_receipt') FROM cards WHERE id=?",
    )
    .bind(goal)
    .fetch_one(&mut *tx)
    .await?;
    let mut receipt = projection
        .and_then(|value| serde_json::from_str::<ExecutionReceipt>(&value).ok())
        .unwrap_or_else(|| {
            ExecutionReceipt::new(
                row.try_get::<String, _>("worker_key").unwrap_or_default(),
                session,
                serde_json::Value::Null,
                row.try_get::<String, _>("lifecycle_id").unwrap_or_default(),
                row.try_get::<String, _>("claimed_at").unwrap_or_default(),
                row.try_get::<i64, _>("ordinal").unwrap_or(0) as u64,
            )
        });
    receipt.session_id = session.to_owned();
    receipt.worker_key = row.try_get("worker_key")?;
    receipt.dispatched_lifecycle = row.try_get("lifecycle_id")?;
    receipt.process_id = row
        .try_get::<Option<i64>, _>("process_id")?
        .and_then(|pid| u32::try_from(pid).ok());
    receipt.attempt = u64::try_from(row.try_get::<i64, _>("ordinal")?)?;
    receipt.finalize(receipt_state, chrono::Utc::now().to_rfc3339());
    sqlx::query("UPDATE cards SET metadata_json=json_set(metadata_json,'$.execution_receipt',json(?)) WHERE id=?")
        .bind(serde_json::to_string(&receipt)?).bind(goal).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(TerminalObservation::Sealed(evidence))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateLineage {
    pub candidate: NativeCandidate,
    pub editing_attempts: Vec<String>,
    pub task_id: String,
    pub contract_json: String,
    pub project_id: String,
    pub canonical_root: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineageResolution {
    Verified(CandidateLineage),
    NoGit,
    Unverifiable(String),
}

fn candidate_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<NativeCandidate> {
    Ok(NativeCandidate {
        attempt_id: row.try_get("attempt_id")?,
        goal_id: row.try_get("goal_id")?,
        worker_session_id: row.try_get("worker_session_id")?,
        worktree: PathBuf::from(row.try_get::<String, _>("worktree_path")?),
        input_head: row.try_get("input_head")?,
        trunk_head: row.try_get("trunk_head")?,
        head: row.try_get("candidate_head")?,
        parent_attempt_id: row.try_get("parent_attempt_id")?,
        substantive: row.try_get("substantive")?,
    })
}

/// Shared durable resolver for live and later boot recovery. It never consults
/// worker/session/baseline projections, and cannot promote historical metadata.
pub(crate) async fn resolve_candidate(
    pool: &Pool<Sqlite>,
    goal: &str,
) -> Result<LineageResolution> {
    let mut connection = pool.acquire().await?;
    match resolve_candidate_inner(&mut connection, goal).await {
        Ok(value) => Ok(value),
        Err(error) => Ok(LineageResolution::Unverifiable(error.to_string())),
    }
}
async fn resolve_candidate_inner(
    connection: &mut sqlx::SqliteConnection,
    goal: &str,
) -> Result<LineageResolution> {
    let binding = sqlx::query(
        "SELECT authority,task_id,contract_json,project_id,canonical_root,root_status FROM goal_execution_bindings WHERE goal_id=?",
    )
    .bind(goal)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| anyhow!("No native original goal binding"))?;
    if binding.try_get::<String, _>("authority")? != "native" {
        bail!("Original task lacks native provenance");
    }
    let task_id: String = binding.try_get("task_id")?;
    let contract_json: String = binding.try_get("contract_json")?;
    let mut next:Option<String>=sqlx::query_scalar("SELECT attempt_id FROM goal_attempts WHERE goal_id=? AND execution_state<>'spawn_failed' ORDER BY ordinal DESC LIMIT 1")
        .bind(goal).fetch_optional(&mut *connection).await?;
    if next.is_none() {
        bail!("No native worker candidate observation");
    }
    let mut candidate = None;
    let mut editing_attempts = Vec::new();
    let mut expected_head: Option<String> = None;
    let mut expected_trunk: Option<String> = None;
    let mut child_ordinal = i64::MAX;
    while let Some(attempt) = next {
        let row = sqlx::query(
            "SELECT a.ordinal,a.execution_state,a.baseline_authority,a.worker_session_id,o.*
            FROM goal_attempts a JOIN goal_candidate_observations o ON o.attempt_id=a.attempt_id
            WHERE a.attempt_id=? AND a.goal_id=?",
        )
        .bind(&attempt)
        .bind(goal)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| anyhow!("Native attempt has no sealed candidate"))?;
        let ordinal: i64 = row.try_get("ordinal")?;
        if ordinal >= child_ordinal {
            bail!("Candidate parent chain is not monotonic");
        }
        child_ordinal = ordinal;
        if !matches!(
            row.try_get::<String, _>("execution_state")?.as_str(),
            "succeeded" | "failed" | "cancelled" | "lost"
        ) || row.try_get::<String, _>("baseline_authority")? != "native"
        {
            bail!("Candidate is not terminally sealed with native baseline authority");
        }
        match row.try_get::<String, _>("provenance_state")?.as_str() {
            "no_git" if candidate.is_none() => return Ok(LineageResolution::NoGit),
            "verified" => {}
            _ => bail!("Candidate Git provenance is unverifiable"),
        }
        let current = candidate_from_row(&row)?;
        if expected_head
            .as_ref()
            .is_some_and(|head| head != &current.head)
            || expected_trunk
                .as_ref()
                .is_some_and(|head| head != &current.trunk_head)
        {
            bail!("Candidate parent does not match this attempt's immutable input/trunk");
        }
        if current.substantive {
            editing_attempts.push(current.attempt_id.clone());
        }
        if candidate.is_none() {
            candidate = Some(current.clone());
        }
        next = current.parent_attempt_id;
        expected_head = Some(current.input_head);
        expected_trunk = Some(current.trunk_head);
    }
    let candidate = candidate.ok_or_else(|| anyhow!("Native Git candidate absent"))?;
    // A durable candidate is exact-head evidence, not permission to silently
    // review a subsequently changed or dirty checkout.
    if std::fs::canonicalize(&candidate.worktree)? != candidate.worktree
        || git_head(&candidate.worktree).await? != candidate.head
    {
        bail!("Candidate worktree identity or HEAD changed after sealing");
    }
    let status = git(&candidate.worktree, &["status", "--porcelain"]).await?;
    if !status.status.success() || !status.stdout.is_empty() {
        bail!("Sealed candidate worktree is dirty");
    }
    if binding.try_get::<String, _>("root_status")? != "resolved" {
        bail!("Code candidate has no resolved native root");
    }
    let canonical_root = PathBuf::from(binding.try_get::<String, _>("canonical_root")?);
    if std::fs::canonicalize(&canonical_root)? != canonical_root {
        bail!("Native root identity changed");
    }
    let project_id: String = binding.try_get("project_id")?;
    Ok(LineageResolution::Verified(CandidateLineage {
        candidate,
        editing_attempts,
        task_id,
        contract_json,
        project_id,
        canonical_root,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContributorEvidence {
    Known {
        families: std::collections::BTreeSet<String>,
        has_unsettled_calls: bool,
        invocation_ids: std::collections::BTreeSet<String>,
    },
    Unknown(String),
}

/// Physical route evidence is separate from financial settlement. Pending or
/// UNKNOWN calls with a durable known route conservatively exclude that family
/// while preserving all existing holds; opaque/unbound calls cannot establish
/// independent review. No refund or reservation state is changed here.
pub(crate) async fn contributor_evidence(
    pool: &Pool<Sqlite>,
    lineage: &CandidateLineage,
) -> Result<ContributorEvidence> {
    let mut connection = pool.acquire().await?;
    match contributor_evidence_inner(&mut connection, lineage).await {
        Ok(value) => Ok(value),
        Err(error) => Ok(ContributorEvidence::Unknown(error.to_string())),
    }
}
async fn contributor_evidence_inner(
    connection: &mut sqlx::SqliteConnection,
    lineage: &CandidateLineage,
) -> Result<ContributorEvidence> {
    if lineage.editing_attempts.is_empty() {
        bail!("No positive deterministic authorship proof exists for the unedited candidate");
    }
    let mut families = std::collections::BTreeSet::new();
    let mut has_unsettled_calls = false;
    let mut invocation_ids = std::collections::BTreeSet::new();
    for attempt in &lineage.editing_attempts {
        let row=sqlx::query("SELECT accounting_authority,worker_session_id FROM goal_attempts WHERE attempt_id=? AND goal_id=?")
            .bind(attempt).bind(&lineage.candidate.goal_id).fetch_one(&mut *connection).await?;
        if row.try_get::<String, _>("accounting_authority")?
            != crate::native_provider_calls::COVERAGE_VERSION
        {
            bail!("Editing attempt lacks mandatory all-route native accounting coverage");
        }
        let session: String = row.try_get("worker_session_id")?;
        let unlinked:i64=sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM cost_ledger WHERE session_id=? AND (goal_attempt_id IS NULL OR goal_attempt_id<>?)) +
            (SELECT COUNT(*) FROM cost_reservations WHERE session_id=? AND (goal_attempt_id IS NULL OR goal_attempt_id<>?))")
            .bind(&session).bind(attempt).bind(&session).bind(attempt).fetch_one(&mut *connection).await?;
        if unlinked != 0 {
            bail!("Editing session includes unbound accounting rows; history cannot be backfilled");
        }
        let mut authorized = std::collections::BTreeMap::new();
        let mut physical_calls = 0usize;
        for call in sqlx::query("SELECT invocation_id,provider,model,session_id,task_id,dispatch_state,coverage_version FROM goal_attempt_provider_calls WHERE goal_attempt_id=?")
            .bind(attempt).fetch_all(&mut *connection).await? {
            if call.try_get::<String,_>("session_id")?!=session || call.try_get::<String,_>("task_id")?!=lineage.task_id || call.try_get::<String,_>("coverage_version")?!=crate::native_provider_calls::COVERAGE_VERSION {
                bail!("Universal native call identity differs from frozen editing attempt");
            }
            let invocation:String=call.try_get("invocation_id")?;
            let provider: String=call.try_get("provider")?;
            let model:String=call.try_get("model")?;
            let dispatch:String=call.try_get("dispatch_state")?;
            if invocation.trim().is_empty() { bail!("Native call lacks invocation identity"); }
            authorized.insert(invocation.clone(),(provider.clone(),model.clone(),dispatch.clone()));
            match dispatch.as_str() {
                "not_sent"=>{},
                "started_possible"=>{
                    let family=crate::cost_router::reviewer_pick::known_physical_family_of(&provider,&model)
                        .ok_or_else(||anyhow!("Editing invocation has opaque or unknown physical family"))?;
                    families.insert(family);invocation_ids.insert(invocation);physical_calls+=1;
                },
                _=>bail!("Editing invocation dispatch coverage is incomplete"),
            }
        }
        for call in
            sqlx::query("SELECT provider,model,call_id FROM cost_ledger WHERE goal_attempt_id=?")
                .bind(attempt)
                .fetch_all(&mut *connection)
                .await?
        {
            let id: String = call.try_get("call_id")?;
            let provider: String = call.try_get("provider")?;
            let model: String = call.try_get("model")?;
            if !authorized
                .get(&id)
                .is_some_and(|(p, m, d)| p == &provider && m == &model && d == "started_possible")
            {
                bail!("Settled monetary call has no matching native physical authorization");
            }
        }
        for call in sqlx::query("SELECT provider,model,state,dispatch_state,invocation_id FROM cost_reservations WHERE goal_attempt_id=?").bind(attempt).fetch_all(&mut *connection).await? {
            let id:String=call.try_get("invocation_id")?;
            let provider:String=call.try_get("provider")?;
            let model:String=call.try_get("model")?;
            let state:String=call.try_get("state")?;
            let dispatch:String=call.try_get("dispatch_state")?;
            let Some((p,m,native_dispatch))=authorized.get(&id) else { bail!("Paid hold has no matching universal native call authorization"); };
            if p!=&provider || m!=&model || native_dispatch!=&dispatch { bail!("Paid hold route/dispatch differs from native call authorization"); }
            if state=="released" && dispatch=="not_sent" { continue; }
            if dispatch!="started_possible" || state=="released" { bail!("Paid editing invocation coverage is inconsistent"); }
            has_unsettled_calls|=matches!(state.as_str(),"pending"|"unknown");
        }
        if physical_calls == 0 {
            bail!("Substantive editing attempt has no positive physical call evidence");
        }
    }
    Ok(ContributorEvidence::Known {
        families,
        has_unsettled_calls,
        invocation_ids,
    })
}

#[cfg(test)]
#[path = "goal_candidates_tests.rs"]
mod tests;

/// Opaque native completion fence. Public only to preserve externally usable
/// TransitionEffects; neither tools nor serde can mint its private identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedAttempt {
    pub(crate) goal_id: String,
    pub(crate) attempt_id: String,
    pub(crate) claim_id: String,
    pub(crate) worker_session_id: String,
    pub(crate) column_id: String,
    state_binding: String,
}
pub(crate) async fn terminal_fence(
    pool: &Pool<Sqlite>,
    goal: &str,
    claim: &str,
    session: &str,
) -> Result<ExpectedAttempt> {
    let attempt_id:String=sqlx::query_scalar("SELECT a.attempt_id FROM goal_attempts a JOIN goal_candidate_observations o ON o.attempt_id=a.attempt_id
        WHERE a.goal_id=? AND a.claim_id=? AND a.worker_session_id=? AND a.execution_state IN ('succeeded','failed','cancelled','lost')")
        .bind(goal).bind(claim).bind(session).fetch_one(pool).await?;
    let column = sqlx::query("SELECT c.column_id,bc.state_binding FROM cards c JOIN board_columns bc ON bc.id=c.column_id WHERE c.id=?")
        .bind(goal).fetch_one(pool).await?;
    let column_id: String = column.try_get("column_id")?;
    let state_binding: String = column.try_get("state_binding")?;
    if !matches!(state_binding.as_str(), "in_progress" | "review") {
        bail!("Inactive state cannot mint a native terminal fence");
    }
    Ok(ExpectedAttempt {
        goal_id: goal.into(),
        attempt_id,
        claim_id: claim.into(),
        worker_session_id: session.into(),
        column_id,
        state_binding,
    })
}
pub(crate) async fn check_expected_tx(
    tx: &mut Transaction<'_, Sqlite>,
    goal: &str,
    expected: Option<&ExpectedAttempt>,
) -> Result<()> {
    if let Some(expected) = expected {
        if expected.goal_id != goal {
            bail!("Completion fence belongs to a different goal");
        }
        let matches: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM goal_attempts a JOIN cards c ON c.id=a.goal_id
            WHERE a.attempt_id=? AND a.goal_id=? AND a.claim_id=? AND a.worker_session_id=?
            AND a.execution_state IN ('succeeded','failed','cancelled','lost')
            AND json_extract(c.metadata_json,'$.dispatch_claim_id')=? AND c.column_id=?
            AND EXISTS (SELECT 1 FROM board_columns bc WHERE bc.id=c.column_id AND bc.state_binding=?)",
        )
        .bind(&expected.attempt_id)
        .bind(goal)
        .bind(&expected.claim_id)
        .bind(&expected.worker_session_id)
        .bind(&expected.claim_id)
        .bind(&expected.column_id)
        .bind(&expected.state_binding)
        .fetch_one(&mut **tx)
        .await?;
        if matches != 1 {
            bail!("Native completion attempt no longer owns the goal");
        }
    }
    Ok(())
}

/// Only the caller of the checked transition can derive the next-stage token,
/// under that same write transaction. A current-column read alone cannot mint
/// Complete authority. This token denotes approval ownership, not Git success.
pub(crate) async fn refresh_expected_after_transition_tx(
    tx: &mut Transaction<'_, Sqlite>,
    expected: &ExpectedAttempt,
    record: &crate::goal_transition::GoalTransitionRecord,
) -> Result<ExpectedAttempt> {
    if record.card_id != expected.goal_id
        || record.current_state.binding() != expected.state_binding
        || !matches!(
            (record.current_state.binding(), record.new_state.binding()),
            ("in_progress", "review") | ("review", "complete")
        )
    {
        bail!("Native stage token is not derived from its authorized transition");
    }
    let column=sqlx::query("SELECT c.column_id,bc.state_binding FROM cards c JOIN board_columns bc ON bc.id=c.column_id WHERE c.id=?")
        .bind(&expected.goal_id).fetch_one(&mut **tx).await?;
    let mut next = expected.clone();
    next.column_id = column.try_get("column_id")?;
    next.state_binding = column.try_get("state_binding")?;
    if next.state_binding != record.new_state.binding() {
        bail!("Checked transition target changed before token derivation");
    }
    check_expected_tx(tx, &next.goal_id, Some(&next)).await?;
    Ok(next)
}

/// Revalidate durable identity, original contract and exact current candidate
/// immediately before review side effects. Final landing must still consume
/// the returned fence in its actual mutation authority/serialization seam.
pub(crate) async fn revalidate_candidate(
    pool: &Pool<Sqlite>,
    lineage: &CandidateLineage,
) -> Result<ExpectedAttempt> {
    match resolve_candidate(pool, &lineage.candidate.goal_id).await? {
        LineageResolution::Verified(current) if &current == lineage => {}
        _ => bail!("Native candidate or original lineage changed before review"),
    }
    let claim: String =
        sqlx::query_scalar("SELECT claim_id FROM goal_attempts WHERE attempt_id=? AND goal_id=?")
            .bind(&lineage.candidate.attempt_id)
            .bind(&lineage.candidate.goal_id)
            .fetch_one(pool)
            .await?;
    let fence = terminal_fence(
        pool,
        &lineage.candidate.goal_id,
        &claim,
        &lineage.candidate.worker_session_id,
    )
    .await?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    check_expected_tx(&mut tx, &lineage.candidate.goal_id, Some(&fence)).await?;
    let column:Option<String>=sqlx::query_scalar("SELECT bc.state_binding FROM cards c JOIN board_columns bc ON bc.id=c.column_id WHERE c.id=?")
        .bind(&lineage.candidate.goal_id).fetch_one(&mut *tx).await?;
    if !matches!(column.as_deref(), Some("in_progress" | "review")) {
        bail!("Native review cannot act on an inactive goal");
    }
    tx.commit().await?;
    Ok(fence)
}

/// Review scope carried by native code, never reconstructed from model prose
/// or a serialized receipt. Approved quality still comes from the actual review.
#[derive(Debug, Clone)]
pub(crate) struct NativeReviewBinding {
    pub lineage: CandidateLineage,
    pub families: std::collections::BTreeSet<String>,
    pub invocation_ids: std::collections::BTreeSet<String>,
    pub expected: ExpectedAttempt,
}
pub(crate) async fn revalidate_review_binding(
    pool: &Pool<Sqlite>,
    review: &NativeReviewBinding,
) -> Result<ExpectedAttempt> {
    let current = revalidate_candidate(pool, &review.lineage).await?;
    if current.attempt_id != review.expected.attempt_id
        || current.claim_id != review.expected.claim_id
        || current.worker_session_id != review.expected.worker_session_id
    {
        bail!("Approved review belongs to a different native execution attempt");
    }
    match contributor_evidence(pool, &review.lineage).await? {
        ContributorEvidence::Known {
            families,
            invocation_ids,
            ..
        } if families == review.families && invocation_ids == review.invocation_ids => Ok(current),
        _ => bail!("Physical contributor evidence changed after review or is incomplete"),
    }
}

/// Final landing accepts only the exact Complete-stage token derived inside
/// the checked approval transaction; a card already marked Complete is not
/// itself evidence that anything landed successfully.
pub(crate) async fn revalidate_review_binding_for_landing(
    pool: &Pool<Sqlite>,
    review: &NativeReviewBinding,
    approved: &ExpectedAttempt,
) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    revalidate_review_binding_for_landing_tx(&mut tx, review, approved).await?;
    tx.commit().await?;
    Ok(())
}
pub(crate) async fn revalidate_review_binding_for_landing_tx(
    tx: &mut Transaction<'_, Sqlite>,
    review: &NativeReviewBinding,
    approved: &ExpectedAttempt,
) -> Result<()> {
    if approved.state_binding != "complete"
        || approved.goal_id != review.expected.goal_id
        || approved.attempt_id != review.expected.attempt_id
        || approved.claim_id != review.expected.claim_id
        || approved.worker_session_id != review.expected.worker_session_id
    {
        bail!("Landing lacks exact native approval ownership");
    }
    if !matches!(resolve_candidate_inner(&mut **tx,&review.lineage.candidate.goal_id).await?,LineageResolution::Verified(ref current) if current==&review.lineage)
    {
        bail!("Reviewed lineage changed before landing");
    }
    match contributor_evidence_inner(&mut **tx, &review.lineage).await? {
        ContributorEvidence::Known {
            families,
            invocation_ids,
            ..
        } if families == review.families && invocation_ids == review.invocation_ids => {}
        _ => bail!("Physical contributor evidence changed before landing"),
    }
    check_expected_tx(tx, &approved.goal_id, Some(approved)).await?;
    Ok(())
}
