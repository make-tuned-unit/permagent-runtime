use super::*;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

fn valid_origin(task_id: &str) -> Value {
    json!({
        "budget_task.v1": task_id,
        "budget_task_contract.v1": {
            "task_id": task_id,
            "text": "Inspect the repository and complete the requested change."
        }
    })
}

async fn pool() -> Pool<Sqlite> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    setup_schema(&pool).await;
    pool
}

async fn file_pool(path: &Path) -> Pool<Sqlite> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    setup_schema(&pool).await;
    pool
}

async fn setup_schema(pool: &Pool<Sqlite>) {
    sqlx::query("CREATE TABLE projects(id TEXT PRIMARY KEY, root_path TEXT)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE sessions(id TEXT PRIMARY KEY, extension_data TEXT, working_dir TEXT)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("CREATE TABLE board_columns(id TEXT PRIMARY KEY, state_binding TEXT)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE cards(id TEXT PRIMARY KEY, column_id TEXT, metadata_json TEXT)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects(id,root_path) VALUES('project-1',NULL)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO board_columns(id,state_binding) VALUES('doing','in_progress')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cards(id,column_id,metadata_json) VALUES('goal-1','doing','{}')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cards(id,column_id,metadata_json) VALUES('goal-2','doing','{}')")
        .execute(pool)
        .await
        .unwrap();
    apply_schema(pool).await.unwrap();
}

async fn add_coordinator(
    pool: &Pool<Sqlite>,
    extension_data: Option<Value>,
    working_dir: Option<&str>,
) {
    sqlx::query("INSERT INTO sessions(id,extension_data,working_dir) VALUES('coordinator',?,?)")
        .bind(extension_data.map(|v| v.to_string()))
        .bind(working_dir)
        .execute(pool)
        .await
        .unwrap();
}

fn claim(claim_id: &str, root: Option<String>) -> ClaimInput {
    ClaimInput {
        claim_id: claim_id.to_owned(),
        lifecycle_id: format!("life-{claim_id}"),
        worker_key: "local-worker".to_owned(),
        execution_root: root,
        input_head: None,
    }
}

async fn bind_native(
    pool: &Pool<Sqlite>,
    temp: &TempDir,
    extension_data: Value,
    task_id: &str,
) -> String {
    let root = std::fs::canonicalize(temp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    add_coordinator(pool, Some(extension_data), Some(&root)).await;
    let mut tx = pool.begin().await.unwrap();
    bind_new_goal_tx(
        &mut tx,
        "goal-1",
        "project-1",
        Some(task_id),
        Some("coordinator"),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    root
}

async fn insert_claim(pool: &Pool<Sqlite>, claim_id: &str, root: Option<String>) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    claim_tx(&mut tx, "goal-1", "project-1", &claim(claim_id, root)).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn native_non_git_binding_keeps_exact_origin_and_accepts_canonical_root() {
    let pool = pool().await;
    let temp = tempfile::tempdir().unwrap();
    let origin = valid_origin("task-1");
    let root = bind_native(&pool, &temp, origin.clone(), "task-1").await;

    let row = sqlx::query("SELECT authority,task_id,contract_json,contract_sha256,canonical_root,initial_git_head,root_status FROM goal_execution_bindings WHERE goal_id='goal-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let contract = origin["budget_task_contract.v1"].clone();
    let canonical_contract = serde_json::to_string(&contract).unwrap();
    assert_eq!(row.try_get::<String, _>("authority").unwrap(), "native");
    assert_eq!(row.try_get::<String, _>("task_id").unwrap(), "task-1");
    assert_eq!(
        row.try_get::<String, _>("contract_json").unwrap(),
        canonical_contract
    );
    assert_eq!(
        row.try_get::<String, _>("contract_sha256").unwrap(),
        hex::encode(Sha256::digest(canonical_contract.as_bytes()))
    );
    assert_eq!(row.try_get::<String, _>("canonical_root").unwrap(), root);
    assert_eq!(
        row.try_get::<Option<String>, _>("initial_git_head")
            .unwrap(),
        None
    );
    assert_eq!(row.try_get::<String, _>("root_status").unwrap(), "resolved");

    assert!(
        insert_claim(&pool, "wrong-root", Some("/another/root".to_owned()))
            .await
            .is_err()
    );
    insert_claim(&pool, "claim-1", Some(root.clone()))
        .await
        .unwrap();
    let ordinal: i64 =
        sqlx::query_scalar("SELECT ordinal FROM goal_attempts WHERE claim_id='claim-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ordinal, 1);
}

#[tokio::test]
async fn native_binding_rejects_wrong_task_or_contract_and_invalid_root() {
    let pool = pool().await;
    let temp = tempfile::tempdir().unwrap();
    let origin = valid_origin("task-1");
    let root = std::fs::canonicalize(temp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    add_coordinator(&pool, Some(origin.clone()), Some(&root)).await;

    let mut tx = pool.begin().await.unwrap();
    assert!(bind_new_goal_tx(
        &mut tx,
        "goal-1",
        "project-1",
        Some("other-task"),
        Some("coordinator"),
        None,
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();

    let mut wrong_contract = origin.clone();
    wrong_contract["budget_task_contract.v1"]["text"] = json!("a different brief");
    sqlx::query("UPDATE sessions SET extension_data=? WHERE id='coordinator'")
        .bind(wrong_contract.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(bind_new_goal_tx(
        &mut tx,
        "goal-1",
        "project-1",
        Some("task-1"),
        Some("coordinator"),
        Some(&origin["budget_task_contract.v1"]),
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();

    sqlx::query("UPDATE sessions SET extension_data=?,working_dir='/definitely/not/a/real/path' WHERE id='coordinator'")
        .bind(origin.to_string())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE projects SET root_path='/definitely/not/a/real/path' WHERE id='project-1'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    bind_new_goal_tx(
        &mut tx,
        "goal-1",
        "project-1",
        Some("task-1"),
        Some("coordinator"),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT root_status FROM goal_execution_bindings WHERE goal_id='goal-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "invalid");
    assert!(insert_claim(&pool, "claim-invalid-root", None)
        .await
        .is_err());
}

#[tokio::test]
async fn attempt_history_is_append_only_across_release_and_terminal_state() {
    let pool = pool().await;
    sqlx::query("INSERT INTO goal_execution_bindings(goal_id,project_id,authority) VALUES('goal-1','project-1','legacy_unverified')")
        .execute(&pool)
        .await
        .unwrap();

    insert_claim(&pool, "claim-1", None).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    release_tx(&mut tx, "goal-1", "claim-1", "refunded/released")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    insert_claim(&pool, "claim-2", None).await.unwrap();
    let rows = sqlx::query(
        "SELECT ordinal,execution_state FROM goal_attempts WHERE goal_id='goal-1' ORDER BY ordinal",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].try_get::<i64, _>("ordinal").unwrap(), 1);
    assert_eq!(
        rows[0].try_get::<String, _>("execution_state").unwrap(),
        "spawn_failed"
    );
    assert_eq!(rows[1].try_get::<i64, _>("ordinal").unwrap(), 2);

    sqlx::query("UPDATE goal_attempts SET execution_state='succeeded',ended_at='now' WHERE claim_id='claim-2'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    release_tx(&mut tx, "goal-1", "claim-2", "late release")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let terminal: String =
        sqlx::query_scalar("SELECT execution_state FROM goal_attempts WHERE claim_id='claim-2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(terminal, "succeeded");
}

#[tokio::test]
async fn journal_identity_gates_finalize_start_and_late_callbacks() {
    let pool = pool().await;
    let temp = tempfile::tempdir().unwrap();
    let root = bind_native(&pool, &temp, valid_origin("task-1"), "task-1").await;
    sqlx::query("UPDATE cards SET metadata_json=? WHERE id='goal-1'")
        .bind(json!({"dispatch_claim_id":"claim-1"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    insert_claim(&pool, "claim-1", Some(root)).await.unwrap();

    let permit = StartPermit {
        pool: pool.clone(),
        goal_id: "goal-1".to_owned(),
        claim_id: "claim-1".to_owned(),
    };
    assert!(
        permit.start().await.is_err(),
        "a prepared worker cannot start before finalize"
    );
    assert!(
        !is_current_worker(&pool, "goal-1", "claim-1", "worker-session")
            .await
            .unwrap()
    );
    assert!(worker_binding(&pool, "coordinator")
        .await
        .unwrap()
        .is_none());

    permit
        .prepare_worker("wrong-task-session", temp.path(), Some("other-task"))
        .await
        .expect_err("a caller-supplied task cannot replace the frozen task");
    permit
        .prepare_worker("worker-session", temp.path(), Some("task-1"))
        .await
        .unwrap();
    assert!(permit
        .prepare_worker("other-session", temp.path(), Some("task-1"))
        .await
        .is_err());
    let mut tx = pool.begin().await.unwrap();
    assert!(finalize_tx(
        &mut tx,
        "goal-1",
        "claim-1",
        Some("wrong-session"),
        Some(77),
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    finalize_tx(
        &mut tx,
        "goal-1",
        "claim-1",
        Some("worker-session"),
        Some(77),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    finalize_tx(
        &mut tx,
        "goal-1",
        "claim-1",
        Some("worker-session"),
        Some(77),
    )
    .await
    .unwrap();
    assert!(finalize_tx(
        &mut tx,
        "goal-1",
        "claim-1",
        Some("worker-session"),
        Some(78),
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();

    permit.start().await.unwrap();
    assert!(
        is_current_worker(&pool, "goal-1", "claim-1", "worker-session")
            .await
            .unwrap()
    );
    let binding = worker_binding(&pool, "worker-session")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.goal_id, "goal-1");
    assert_eq!(binding.task_id, "task-1");
    assert_eq!(binding.worker_session_id, "worker-session");
    assert!(worker_binding(&pool, "coordinator")
        .await
        .unwrap()
        .is_none());

    // Leaving the active column revokes callbacks even if the claim projection
    // and the pre-terminal-observation journal state are unchanged.
    sqlx::query("INSERT INTO board_columns(id,state_binding) VALUES('review','review')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cards SET column_id='review' WHERE id='goal-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !is_current_worker(&pool, "goal-1", "claim-1", "worker-session")
            .await
            .unwrap()
    );
    sqlx::query("UPDATE cards SET column_id='doing' WHERE id='goal-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        is_current_worker(&pool, "goal-1", "claim-1", "worker-session")
            .await
            .unwrap()
    );

    // Mutable card metadata alone cannot create or impersonate an attempt.
    sqlx::query("UPDATE cards SET metadata_json=? WHERE id='goal-1'")
        .bind(json!({"dispatch_claim_id":"forged-claim"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !is_current_worker(&pool, "goal-1", "forged-claim", "worker-session")
            .await
            .unwrap()
    );
    assert!(worker_binding(&pool, "worker-session")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn missing_journal_and_untrusted_session_metadata_never_authorize_a_worker() {
    let pool = pool().await;
    sqlx::query("UPDATE cards SET metadata_json=? WHERE id='goal-1'")
        .bind(json!({"dispatch_claim_id":"forged-claim"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(finalize_tx(
        &mut tx,
        "goal-1",
        "forged-claim",
        Some("forged-session"),
        Some(99),
    )
    .await
    .is_err());
    tx.rollback().await.unwrap();
    let no_journal = StartPermit {
        pool: pool.clone(),
        goal_id: "goal-1".to_owned(),
        claim_id: "forged-claim".to_owned(),
    };
    assert!(no_journal.start().await.is_err());

    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    add_coordinator(&pool, None, Some(&root)).await;
    sqlx::query(
        "INSERT INTO sessions(id,extension_data,working_dir) VALUES('forged-coordinator',?,?)",
    )
    .bind(json!({"created_by_session_id":"native-looking"}).to_string())
    .bind(&root)
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    bind_new_goal_tx(
        &mut tx,
        "goal-1",
        "project-1",
        Some("task-1"),
        Some("coordinator"),
        None,
    )
    .await
    .unwrap();
    bind_new_goal_tx(
        &mut tx,
        "goal-2",
        "project-1",
        Some("task-1"),
        Some("forged-coordinator"),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let authorities: Vec<(String, String)> =
        sqlx::query_as("SELECT goal_id,authority FROM goal_execution_bindings ORDER BY goal_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        authorities,
        vec![
            ("goal-1".to_owned(), "unverified".to_owned()),
            ("goal-2".to_owned(), "unverified".to_owned()),
        ]
    );
}

#[tokio::test]
async fn claim_rollback_and_duplicate_claim_id_are_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let pool = file_pool(&temp.path().join("lineage.sqlite")).await;
    sqlx::query("INSERT INTO goal_execution_bindings(goal_id,project_id,authority) VALUES('goal-1','project-1','legacy_unverified')")
        .execute(&pool)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    claim_tx(&mut tx, "goal-1", "project-1", &claim("claim-1", None))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let rolled_back: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_attempts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rolled_back, 0);

    let (first, second) = tokio::join!(
        insert_claim(&pool, "claim-1", None),
        insert_claim(&pool, "claim-1", None),
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_attempts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn orchestrator_derived_contract_binds_when_it_carries_the_native_text() {
    // Regression: `roadmap_task_contract` supplies a derived object (source,
    // session_id, budget_task_id) that never equals the raw native contract.
    let pool = pool().await;
    let temp = tempfile::tempdir().unwrap();
    let origin = valid_origin("task-1");
    let root = std::fs::canonicalize(temp.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    add_coordinator(&pool, Some(origin.clone()), Some(&root)).await;
    let text = origin["budget_task_contract.v1"]["text"].clone();
    let derived = |text: Value, task: &str| {
        json!({"text": text, "source": "budget_task_origin", "source_message_id": null,
               "session_id": "coordinator", "budget_task_id": task})
    };
    for (contract, ok) in [
        (derived(text.clone(), "task-1"), true),
        (derived(json!("a different brief"), "task-1"), false),
        (derived(text.clone(), "other-task"), false),
    ] {
        let mut tx = pool.begin().await.unwrap();
        let result = bind_new_goal_tx(
            &mut tx,
            "goal-1",
            "project-1",
            Some("task-1"),
            Some("coordinator"),
            Some(&contract),
        )
        .await;
        assert_eq!(result.is_ok(), ok, "{contract}");
        tx.rollback().await.unwrap();
    }
}
