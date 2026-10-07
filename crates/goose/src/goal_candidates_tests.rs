use super::*;
use crate::agents::platform_extensions::execution_receipt::ReceiptState;
use crate::goal_attempts::{self, ClaimInput, StartPermit};
use sqlx::sqlite::SqlitePoolOptions;

struct Fixture {
    _temp: tempfile::TempDir,
    pool: Pool<Sqlite>,
    root: PathBuf,
    anchor: String,
}
fn git_sync(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn commit(root: &Path, name: &str) {
    std::fs::write(root.join(name), name).unwrap();
    git_sync(root, &["add", name]);
    git_sync(root, &["commit", "-q", "-m", name]);
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git_sync(&root, &["init", "-q"]);
        git_sync(&root, &["config", "user.name", "Test"]);
        git_sync(&root, &["config", "user.email", "test@example.invalid"]);
        git_sync(&root, &["config", "commit.gpgsign", "false"]);
        commit(&root, "base.txt");
        let root = std::fs::canonicalize(root).unwrap();
        let anchor = git_sync(&root, &["rev-parse", "HEAD"]);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(temp.path().join("lineage.sqlite"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE cards(id TEXT PRIMARY KEY,column_id TEXT,metadata_json TEXT,archived_at TEXT)",
            "CREATE TABLE sessions(id TEXT PRIMARY KEY)",
            "CREATE TABLE board_columns(id TEXT PRIMARY KEY,state_binding TEXT)",
            "INSERT INTO board_columns VALUES('active','in_progress')",
            "CREATE TABLE cost_ledger(session_id TEXT,goal_attempt_id TEXT,provider TEXT,model TEXT,call_id TEXT)",
            "CREATE TABLE cost_reservations(session_id TEXT,goal_attempt_id TEXT,provider TEXT,model TEXT,state TEXT,dispatch_state TEXT,invocation_id TEXT)",
        ] { sqlx::query(statement).execute(&pool).await.unwrap(); }
        goal_attempts::apply_schema(&pool).await.unwrap();
        crate::session::spectral_schema::apply_harness_process_leases_schema(&pool)
            .await
            .unwrap();
        let fixture = Self {
            _temp: temp,
            pool,
            root,
            anchor,
        };
        fixture.add_goal("goal").await;
        fixture
    }
    async fn add_goal(&self, goal: &str) {
        sqlx::query("INSERT INTO cards VALUES(?,'active','{}',NULL)")
            .bind(goal)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO goal_execution_bindings(goal_id,project_id,task_id,coordinator_session_id,contract_json,contract_sha256,authority,canonical_root,initial_git_head,root_status) VALUES(?,'project','task','coordinator',?,?,'native',?,?,'resolved')")
            .bind(goal).bind("{\"task_id\":\"task\",\"text\":\"Original frozen requirements\"}").bind("fixture-digest").bind(self.root.to_string_lossy().as_ref()).bind(&self.anchor).execute(&self.pool).await.unwrap();
    }
    async fn start(&self, goal: &str, claim: &str, session: &str) -> (StartPermit, PathBuf) {
        let (head, _parent) = dispatch_baseline(&self.pool, goal, &self.root)
            .await
            .unwrap()
            .unwrap();
        let head = head.unwrap();
        sqlx::query("UPDATE cards SET metadata_json=json_set(metadata_json,'$.dispatch_claim_id',?) WHERE id=?").bind(claim).bind(goal).execute(&self.pool).await.unwrap();
        let mut tx = self.pool.begin().await.unwrap();
        goal_attempts::claim_tx(
            &mut tx,
            goal,
            "project",
            &ClaimInput {
                claim_id: claim.into(),
                lifecycle_id: "test".into(),
                worker_key: "test-worker".into(),
                execution_root: Some(self.root.to_string_lossy().into_owned()),
                input_head: Some(head.clone()),
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let worktree = crate::agents::platform_extensions::goal_engine::create_goal_worktree(
            &self.root, &head, session,
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO sessions(id) VALUES(?)")
            .bind(session)
            .execute(&self.pool)
            .await
            .unwrap();
        let permit = StartPermit {
            pool: self.pool.clone(),
            goal_id: goal.into(),
            claim_id: claim.into(),
        };
        permit
            .prepare_worker(session, &worktree, Some("task"))
            .await
            .unwrap();
        permit.mark_provider_accounting(session).await.unwrap();
        let mut tx = self.pool.begin().await.unwrap();
        goal_attempts::finalize_tx(&mut tx, goal, claim, Some(session), None)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        permit.start().await.unwrap();
        (permit, worktree)
    }
    async fn seal(
        &self,
        goal: &str,
        claim: &str,
        session: &str,
        state: TerminalState,
    ) -> TerminalObservation {
        observe_worker_terminal(
            &self.pool,
            goal,
            claim,
            session,
            state,
            "test native terminal",
            ReceiptState::Failed,
        )
        .await
        .unwrap()
    }
}
fn verified(value: LineageResolution) -> CandidateLineage {
    match value {
        LineageResolution::Verified(v) => v,
        other => panic!("expected verified candidate, got {other:?}"),
    }
}

#[tokio::test]
async fn failed_editor_then_no_edit_retry_preserves_candidate_and_original_contributor_attempt() {
    let f = Fixture::new().await;
    let (_permit, first) = f.start("goal", "claim-1", "worker-1").await;
    commit(&first, "change.txt");
    assert!(matches!(
        f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
            .await,
        TerminalObservation::Sealed(CandidateResolution::Verified(_))
    ));
    let original = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    assert_eq!(original.editing_attempts.len(), 1);
    let (_permit, second) = f.start("goal", "claim-2", "worker-2").await;
    assert_eq!(
        git_sync(&second, &["rev-parse", "HEAD"]),
        original.candidate.head
    );
    f.seal("goal", "claim-2", "worker-2", TerminalState::Failed)
        .await;
    let retry = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    assert_eq!(retry.candidate.head, original.candidate.head);
    assert_eq!(retry.editing_attempts, original.editing_attempts);
    assert_eq!(retry.task_id, "task");
    assert!(retry.contract_json.contains("Original frozen requirements"));
    let state: String =
        sqlx::query_scalar("SELECT execution_state FROM goal_attempts WHERE claim_id='claim-2'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(state, "failed");
}

#[tokio::test]
async fn later_edit_retains_parent_edit_and_seal_is_idempotent_but_changed_head_conflicts() {
    let f = Fixture::new().await;
    let (_, first) = f.start("goal", "claim-1", "worker-1").await;
    commit(&first, "first.txt");
    f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
        .await;
    let (_, second) = f.start("goal", "claim-2", "worker-2").await;
    commit(&second, "second.txt");
    f.seal("goal", "claim-2", "worker-2", TerminalState::Succeeded)
        .await;
    let candidate = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    assert_eq!(candidate.editing_attempts.len(), 2);
    assert!(matches!(
        f.seal("goal", "claim-2", "worker-2", TerminalState::Succeeded)
            .await,
        TerminalObservation::AlreadySealed(_)
    ));
    commit(&second, "late.txt");
    assert!(observe_worker_terminal(
        &f.pool,
        "goal",
        "claim-2",
        "worker-2",
        TerminalState::Succeeded,
        "test native terminal",
        ReceiptState::Completed
    )
    .await
    .is_err());
    assert!(matches!(
        resolve_candidate(&f.pool, "goal").await.unwrap(),
        LineageResolution::Unverifiable(_)
    ));
}

#[tokio::test]
async fn mutable_retry_metadata_cannot_select_a_foreign_baseline_or_author() {
    let f = Fixture::new().await;
    let (_, first) = f.start("goal", "claim-1", "worker-1").await;
    commit(&first, "first.txt");
    f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
        .await;
    let candidate = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    sqlx::query("UPDATE cards SET metadata_json=json_set(metadata_json,'$.baseline_commit','forged','$.worker_session_id','foreign','$.redispatch_continued_from',json('{\"base_commit\":\"forged\"}')) WHERE id='goal'").execute(&f.pool).await.unwrap();
    assert_eq!(
        dispatch_baseline(&f.pool, "goal", &f.root)
            .await
            .unwrap()
            .unwrap()
            .0,
        Some(candidate.candidate.head)
    );
    let mut tx = f.pool.begin().await.unwrap();
    assert!(goal_attempts::claim_tx(
        &mut tx,
        "goal",
        "project",
        &ClaimInput {
            claim_id: "wrong".into(),
            lifecycle_id: "test".into(),
            worker_key: "test-worker".into(),
            execution_root: Some(f.root.to_string_lossy().into_owned()),
            input_head: Some(f.anchor.clone())
        }
    )
    .await
    .is_err());
}

#[tokio::test]
async fn sequential_dag_goal_can_start_at_native_advanced_trunk_after_materialization() {
    let f = Fixture::new().await;
    f.add_goal("dependent").await;
    commit(&f.root, "prior-goal-landed.txt");
    let advanced = git_sync(&f.root, &["rev-parse", "HEAD"]);
    assert_ne!(advanced, f.anchor);
    let (_, worker) = f
        .start("dependent", "dependent-claim", "dependent-worker")
        .await;
    assert_eq!(git_sync(&worker, &["rev-parse", "HEAD"]), advanced);
    commit(&worker, "dependent-change.txt");
    f.seal(
        "dependent",
        "dependent-claim",
        "dependent-worker",
        TerminalState::Failed,
    )
    .await;
    assert_eq!(
        verified(resolve_candidate(&f.pool, "dependent").await.unwrap())
            .candidate
            .trunk_head,
        advanced
    );
}

#[tokio::test]
async fn dirty_candidate_or_lost_claim_cannot_seal_or_replace_another_terminal_receipt() {
    let f = Fixture::new().await;
    let (_, worker) = f.start("goal", "claim-1", "worker-1").await;
    std::fs::write(worker.join("dirty.txt"), "not committed").unwrap();
    assert!(matches!(
        f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
            .await,
        TerminalObservation::Sealed(CandidateResolution::Unverifiable(_))
    ));
    assert!(matches!(
        resolve_candidate(&f.pool, "goal").await.unwrap(),
        LineageResolution::Unverifiable(_)
    ));
    let g = Fixture::new().await;
    let (_, worker) = g.start("goal", "claim-1", "worker-1").await;
    commit(&worker, "candidate.txt");
    sqlx::query("UPDATE cards SET metadata_json='{\"dispatch_claim_id\":\"different\",\"execution_receipt\":{\"session_id\":\"different\"}}' WHERE id='goal'").execute(&g.pool).await.unwrap();
    assert_eq!(
        g.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
            .await,
        TerminalObservation::Stale
    );
    let receipt: String = sqlx::query_scalar("SELECT metadata_json FROM cards WHERE id='goal'")
        .fetch_one(&g.pool)
        .await
        .unwrap();
    assert!(receipt.contains("different"));
    let observations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_candidate_observations")
        .fetch_one(&g.pool)
        .await
        .unwrap();
    assert_eq!(observations, 0);
}

#[tokio::test]
async fn zero_calls_external_unknown_and_pending_known_route_have_distinct_provenance() {
    let f = Fixture::new().await;
    let (_, worker) = f.start("goal", "claim-1", "worker-1").await;
    commit(&worker, "edit.txt");
    f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
        .await;
    let lineage = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    assert!(matches!(
        contributor_evidence(&f.pool, &lineage).await.unwrap(),
        ContributorEvidence::Unknown(_)
    ));
    // Positive coverage uses the production schema and authorization API
    // while the worker is active, never a post-terminal journal insertion.
    let f = Fixture::new().await;
    let (_, worker) = f.start("goal", "claim-1", "worker-1").await;
    let binding = goal_attempts::worker_binding(&f.pool, "worker-1")
        .await
        .unwrap()
        .unwrap();
    crate::native_provider_calls::record(
        &f.pool,
        "physical-call-1",
        &binding,
        "openai",
        "gpt-5.4-mini",
    )
    .await
    .unwrap();
    crate::native_provider_calls::mark_dispatch_possible(&f.pool, "physical-call-1", &binding)
        .await
        .unwrap();
    let attempt = &binding.attempt_id;
    commit(&worker, "edit.txt");
    f.seal("goal", "claim-1", "worker-1", TerminalState::Failed)
        .await;
    let lineage = verified(resolve_candidate(&f.pool, "goal").await.unwrap());
    sqlx::query("INSERT INTO cost_reservations VALUES('worker-1',?,'openai','gpt-5.4-mini','unknown','started_possible','physical-call-1')").bind(attempt).execute(&f.pool).await.unwrap();
    let evidence = contributor_evidence(&f.pool, &lineage).await.unwrap();
    let family =
        crate::cost_router::reviewer_pick::known_physical_family_of("openai", "gpt-5.4-mini")
            .expect("configured canonical physical model is known");
    assert_eq!(
        evidence,
        ContributorEvidence::Known {
            families: std::collections::BTreeSet::from([family]),
            has_unsettled_calls: true,
            invocation_ids: std::collections::BTreeSet::from(["physical-call-1".into()])
        }
    );
    let state: String = sqlx::query_scalar("SELECT state FROM cost_reservations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        state, "unknown",
        "authorship evidence cannot release financial holds"
    );
    sqlx::query("UPDATE goal_attempts SET accounting_authority='unverified' WHERE attempt_id=?")
        .bind(attempt)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        contributor_evidence(&f.pool, &lineage).await.unwrap(),
        ContributorEvidence::Unknown(_)
    ));
}

#[tokio::test]
async fn suspended_old_completion_cannot_review_requeue_or_park_a_redispatched_attempt() {
    use crate::goal_state::GoalAction;
    use crate::goal_transition::{self, TransitionEffects};
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::session::SessionManager::new(temp.path().join("sessions"));
    let pool = manager.pool_clone().await.unwrap();
    let project = crate::projects::PERSONAL_PROJECT_ID;
    crate::cards::seed_goal_columns(&pool, project)
        .await
        .unwrap();
    let column = crate::cards::get_goal_column(&pool, project, "in_progress")
        .await
        .unwrap()
        .unwrap();
    let card=crate::cards::create_card(&pool,crate::cards::CreateCard {
        project_id:project.into(),title:"Completion race".into(),description:None,card_type:Some("goal".into()),column_id:Some(column.id),created_by:None,
        metadata_json:Some(serde_json::json!({"dispatch_claim_id":"old-claim","worker_session_id":"old-worker"})),
    }).await.unwrap();
    sqlx::query("INSERT INTO goal_execution_bindings(goal_id,project_id,authority) VALUES(?,?,'legacy_unverified')").bind(&card.id).bind(project).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO goal_attempts(attempt_id,goal_id,ordinal,claim_id,lifecycle_id,worker_key,worker_session_id,execution_state) VALUES('old-attempt',?,1,'old-claim','test','test-worker','old-worker','failed')")
        .bind(&card.id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO goal_candidate_observations(attempt_id,goal_id,substantive,provenance_state,reason) VALUES('old-attempt',?,0,'unverifiable','fixture')")
        .bind(&card.id).execute(&pool).await.unwrap();
    let expected = terminal_fence(&pool, &card.id, "old-claim", "old-worker")
        .await
        .unwrap();
    let (resume, waiting) = tokio::sync::oneshot::channel();
    let handler_pool = pool.clone();
    let handler_goal = card.id.clone();
    let handler = tokio::spawn(async move {
        waiting.await.unwrap();
        let review = goal_transition::advance_goal_checked(
            &handler_pool,
            &handler_goal,
            GoalAction::Review,
            crate::decisions::ACTOR_SYSTEM,
            None,
            TransitionEffects {
                expected_attempt: Some(expected.clone()),
                ..Default::default()
            },
        )
        .await;
        let requeue = goal_transition::requeue_goal_for_attempt(
            &handler_pool,
            &handler_goal,
            crate::decisions::ACTOR_SYSTEM,
            1,
            "stale",
            Some(&expected),
        )
        .await;
        let park = goal_transition::park_goal_for_attempt(
            &handler_pool,
            &handler_goal,
            crate::decisions::ACTOR_SYSTEM,
            "stale",
            Some(&expected),
        )
        .await;
        (review, requeue, park)
    });
    // The old handler is suspended while a real native stop/requeue/dispatch
    // lifecycle changes the protected claim, rather than a projection-only write.
    goal_transition::park_goal(
        &pool,
        &card.id,
        crate::decisions::ACTOR_JESSE,
        "user interrupted this attempt",
    )
    .await
    .unwrap();
    goal_transition::advance_goal_checked(
        &pool,
        &card.id,
        GoalAction::Ready,
        crate::decisions::ACTOR_JESSE,
        None,
        TransitionEffects::default(),
    )
    .await
    .unwrap();
    goal_transition::advance_goal_checked(
        &pool,
        &card.id,
        GoalAction::Dispatch,
        crate::decisions::ACTOR_SYSTEM,
        None,
        TransitionEffects {
            metadata_patch: serde_json::Map::from_iter([(
                "dispatch_claim_id".into(),
                serde_json::json!("new-claim"),
            )]),
            attempt_claim: Some(ClaimInput {
                claim_id: "new-claim".into(),
                lifecycle_id: "test".into(),
                worker_key: "new-worker".into(),
                execution_root: None,
                input_head: None,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let permit = StartPermit {
        pool: pool.clone(),
        goal_id: card.id.clone(),
        claim_id: "new-claim".into(),
    };
    permit
        .prepare_worker("new-worker", temp.path(), None)
        .await
        .unwrap();
    goal_transition::finalize_dispatch_claim(
        &pool,
        &card.id,
        "new-claim",
        serde_json::Map::from_iter([("worker_session_id".into(), serde_json::json!("new-worker"))]),
    )
    .await
    .unwrap();
    permit.start().await.unwrap();
    let before = crate::cards::get_card(&pool, &card.id)
        .await
        .unwrap()
        .unwrap();
    resume.send(()).unwrap();
    let (review, requeue, park) = handler.await.unwrap();
    assert!(matches!(
        review,
        Err(goal_transition::GuardError::Denied(_))
    ));
    assert!(matches!(
        requeue,
        Err(goal_transition::GuardError::Denied(_))
    ));
    assert!(matches!(park, Err(goal_transition::GuardError::Denied(_))));
    let after = crate::cards::get_card(&pool, &card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.column_id, before.column_id);
    assert_eq!(after.metadata_json, before.metadata_json);

    // A human park WITHOUT another dispatch also revokes callback authority;
    // retaining the exact claim alone cannot preserve the old active column.
    observe_worker_terminal(
        &pool,
        &card.id,
        "new-claim",
        "new-worker",
        TerminalState::Failed,
        "worker ended",
        ReceiptState::Failed,
    )
    .await
    .unwrap();
    let cancelled = terminal_fence(&pool, &card.id, "new-claim", "new-worker")
        .await
        .unwrap();
    goal_transition::park_goal(
        &pool,
        &card.id,
        crate::decisions::ACTOR_JESSE,
        "user cancelled",
    )
    .await
    .unwrap();
    let preserved = crate::cards::get_card(&pool, &card.id)
        .await
        .unwrap()
        .unwrap();
    let decisions_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE goal_id=?")
            .bind(&card.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(goal_transition::advance_goal_checked(
        &pool,
        &card.id,
        GoalAction::Review,
        crate::decisions::ACTOR_SYSTEM,
        None,
        TransitionEffects {
            expected_attempt: Some(cancelled.clone()),
            ..Default::default()
        }
    )
    .await
    .is_err());
    assert!(goal_transition::requeue_goal_for_attempt(
        &pool,
        &card.id,
        crate::decisions::ACTOR_SYSTEM,
        2,
        "stale",
        Some(&cancelled)
    )
    .await
    .is_err());
    assert!(goal_transition::park_goal_for_attempt(
        &pool,
        &card.id,
        crate::decisions::ACTOR_SYSTEM,
        "stale",
        Some(&cancelled)
    )
    .await
    .is_err());
    assert!(goal_transition::exhaust_and_park_for_attempt(
        &pool,
        &card.id,
        &card.title,
        &card.project_id,
        goal_transition::BudgetExhaustion::AttemptCap { spent: 2, cap: 2 },
        Some("stale"),
        None,
        Some(&cancelled)
    )
    .await
    .is_err());
    let decisions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE goal_id=?")
        .bind(&card.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(decisions_before, decisions_after);
    let unchanged = crate::cards::get_card(&pool, &card.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.column_id, preserved.column_id);
    assert_eq!(unchanged.metadata_json, preserved.metadata_json);
}

#[tokio::test]
async fn known_unreported_delegation_is_sticky_and_inactive_worker_is_refused() {
    let fixture = Fixture::new().await;
    let (_permit, worktree) = fixture.start("goal", "claim", "editor").await;
    goal_attempts::mark_unreported_delegation(&fixture.pool, "editor")
        .await
        .unwrap();
    goal_attempts::mark_unreported_delegation(&fixture.pool, "editor")
        .await
        .unwrap();
    let coverage: String = sqlx::query_scalar(
        "SELECT accounting_authority FROM goal_attempts WHERE worker_session_id='editor'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(coverage, "unreported_delegation");
    // The separate native reviewer child has no exact editor binding and
    // neither inherits editor identity nor changes its coverage authority.
    goal_attempts::mark_unreported_delegation(&fixture.pool, "reviewer-child")
        .await
        .unwrap();
    commit(&worktree, "candidate");
    fixture
        .seal("goal", "claim", "editor", TerminalState::Failed)
        .await;
    let LineageResolution::Verified(lineage) =
        resolve_candidate(&fixture.pool, "goal").await.unwrap()
    else {
        panic!("Expected sealed candidate");
    };
    assert!(matches!(
        contributor_evidence(&fixture.pool, &lineage).await.unwrap(),
        ContributorEvidence::Unknown(_)
    ));
    assert!(
        goal_attempts::mark_unreported_delegation(&fixture.pool, "editor")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn wholly_unedited_candidate_has_no_positive_deterministic_authorship_proof() {
    let fixture = Fixture::new().await;
    fixture.start("goal", "claim", "editor").await;
    fixture
        .seal("goal", "claim", "editor", TerminalState::Failed)
        .await;
    let LineageResolution::Verified(lineage) =
        resolve_candidate(&fixture.pool, "goal").await.unwrap()
    else {
        panic!("Expected sealed candidate");
    };
    assert!(lineage.editing_attempts.is_empty());
    assert!(matches!(
        contributor_evidence(&fixture.pool, &lineage).await.unwrap(),
        ContributorEvidence::Unknown(_)
    ));
}

#[tokio::test]
async fn additive_candidate_migration_preserves_legacy_attempts_and_financial_holds() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE goal_attempts(attempt_id TEXT PRIMARY KEY,goal_id TEXT,execution_state TEXT)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO goal_attempts VALUES('legacy','goal','failed')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE cost_reservations(invocation_id TEXT, state TEXT, bound_usd REAL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cost_reservations VALUES('held','unknown',0.2)")
        .execute(&pool)
        .await
        .unwrap();
    apply_schema(&pool).await.unwrap();
    apply_schema(&pool).await.unwrap();
    let row = sqlx::query("SELECT execution_state,baseline_authority,accounting_authority,parent_candidate_attempt_id FROM goal_attempts WHERE attempt_id='legacy'").fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String, _>("execution_state"), "failed");
    assert_eq!(row.get::<String, _>("baseline_authority"), "unverified");
    assert_eq!(row.get::<String, _>("accounting_authority"), "unverified");
    assert!(row
        .get::<Option<String>, _>("parent_candidate_attempt_id")
        .is_none());
    let hold =
        sqlx::query("SELECT state,bound_usd FROM cost_reservations WHERE invocation_id='held'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(hold.get::<String, _>("state"), "unknown");
    assert_eq!(hold.get::<f64, _>("bound_usd"), 0.2);
}

#[tokio::test]
async fn checked_review_and_approval_derive_next_stage_tokens_in_their_transactions() {
    use crate::goal_state::GoalAction;
    use crate::goal_transition::{self, TransitionEffects};
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::session::SessionManager::new(temp.path().join("sessions"));
    let pool = manager.pool_clone().await.unwrap();
    let project = crate::projects::PERSONAL_PROJECT_ID;
    crate::cards::seed_goal_columns(&pool, project)
        .await
        .unwrap();
    let column = crate::cards::get_goal_column(&pool, project, "in_progress")
        .await
        .unwrap()
        .unwrap();
    let card = crate::cards::create_card(
        &pool,
        crate::cards::CreateCard {
            project_id: project.into(),
            title: "Stage ownership".into(),
            description: None,
            card_type: Some("goal".into()),
            column_id: Some(column.id),
            created_by: None,
            metadata_json: Some(serde_json::json!({"dispatch_claim_id":"claim"})),
        },
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO goal_execution_bindings(goal_id,project_id,authority) VALUES(?,?,'legacy_unverified')").bind(&card.id).bind(project).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO goal_attempts(attempt_id,goal_id,ordinal,claim_id,lifecycle_id,worker_key,worker_session_id,execution_state) VALUES('attempt',?,1,'claim','test','worker','editor','failed')").bind(&card.id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO goal_candidate_observations(attempt_id,goal_id,substantive,provenance_state,reason) VALUES('attempt',?,0,'unverifiable','fixture')").bind(&card.id).execute(&pool).await.unwrap();
    // This test proves transition ownership only. Legacy candidate remains
    // unverifiable, so none of these tokens qualify it for automatic landing.
    let started = terminal_fence(&pool, &card.id, "claim", "editor")
        .await
        .unwrap();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let record = goal_transition::advance_goal_checked_in_tx(
        &mut tx,
        &card.id,
        GoalAction::Review,
        crate::decisions::ACTOR_SYSTEM,
        None,
        TransitionEffects {
            expected_attempt: Some(started.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let reviewed = refresh_expected_after_transition_tx(&mut tx, &started, &record)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_ne!(started.column_id, reviewed.column_id);
    let decision = crate::decisions::create_decision(
        &pool,
        crate::decisions::NewDecision {
            kind: "approve_review".into(),
            goal_id: Some(card.id.clone()),
            project_id: Some(project.into()),
            headline: Some("Review fixture".into()),
            detail: Some("Transition ownership fixture".into()),
            payload: serde_json::json!({}),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, proof) = crate::decisions::answer_decision(
        &pool,
        &decision.id,
        &crate::decisions::DecisionAnswer {
            answer: "approve".into(),
            ..Default::default()
        },
        crate::decisions::ACTOR_JESSE,
    )
    .await
    .unwrap();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let record = goal_transition::advance_goal_checked_in_tx(
        &mut tx,
        &card.id,
        GoalAction::Approve,
        crate::decisions::ACTOR_JESSE,
        Some(proof),
        TransitionEffects {
            expected_attempt: Some(reviewed.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let approved = refresh_expected_after_transition_tx(&mut tx, &reviewed, &record)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(approved.state_binding, "complete");
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    check_expected_tx(&mut tx, &card.id, Some(&approved))
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(terminal_fence(&pool, &card.id, "claim", "editor")
        .await
        .is_err());
    assert!(matches!(
        resolve_candidate(&pool, &card.id).await.unwrap(),
        LineageResolution::Unverifiable(_)
    ));
}

// An actual short-lived process uses the production native claim/prepare/start
// gate. Parent tests can then prove owner absence without guessing/backfilling
// the owner PID after death. This child never constructs a provider or model.
#[tokio::test]
async fn native_loss_journal_child_producer() {
    let Ok(database) = std::env::var("PERMAGENT_TEST_NATIVE_LOSS_DB") else {
        return;
    };
    let root = PathBuf::from(std::env::var("PERMAGENT_TEST_NATIVE_LOSS_ROOT").unwrap());
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(database))
        .await
        .unwrap();
    let (head, _) = dispatch_baseline(&pool, "goal", &root)
        .await
        .unwrap()
        .unwrap();
    let head = head.unwrap();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE cards SET metadata_json=json_set(metadata_json,'$.dispatch_claim_id','child-claim') WHERE id='goal'").execute(&mut *tx).await.unwrap();
    goal_attempts::claim_tx(
        &mut tx,
        "goal",
        "project",
        &ClaimInput {
            claim_id: "child-claim".into(),
            lifecycle_id: "child-lifecycle".into(),
            worker_key: "native-test".into(),
            execution_root: Some(root.to_string_lossy().into_owned()),
            input_head: Some(head.clone()),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let worktree = crate::agents::platform_extensions::goal_engine::create_goal_worktree(
        &root,
        &head,
        "child-worker",
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO sessions(id) VALUES('child-worker')")
        .execute(&pool)
        .await
        .unwrap();
    let permit = StartPermit {
        pool: pool.clone(),
        goal_id: "goal".into(),
        claim_id: "child-claim".into(),
    };
    permit
        .prepare_worker("child-worker", &worktree, Some("task"))
        .await
        .unwrap();
    permit
        .mark_provider_accounting("child-worker")
        .await
        .unwrap();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    goal_attempts::finalize_tx(&mut tx, "goal", "child-claim", Some("child-worker"), None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    permit.start().await.unwrap();
    commit(&worktree, "lost-candidate.txt");
}

async fn produce_dead_native_worker(fixture: &Fixture) -> u32 {
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "goal_candidates::tests::native_loss_journal_child_producer",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "PERMAGENT_TEST_NATIVE_LOSS_DB",
            fixture._temp.path().join("lineage.sqlite"),
        )
        .env("PERMAGENT_TEST_NATIVE_LOSS_ROOT", &fixture.root)
        .kill_on_drop(true);
    if let Some(value) = std::env::var_os("DYLD_LIBRARY_PATH") {
        command.env("DYLD_LIBRARY_PATH", value);
    }
    let mut child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(60), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    pid
}

#[tokio::test]
async fn actual_native_owner_producer_and_boot_live_loss_inspection_preserve_task_and_holds() {
    use crate::goal_recovery::{self, NativeInspection};
    let fixture = Fixture::new().await;
    let pid = produce_dead_native_worker(&fixture).await;
    let row=sqlx::query("SELECT owner_process_id,execution_kind,execution_state FROM goal_attempts WHERE worker_session_id='child-worker'").fetch_one(&fixture.pool).await.unwrap();
    assert_eq!(row.get::<i64, _>("owner_process_id"), i64::from(pid));
    assert_eq!(row.get::<String, _>("execution_kind"), "internal");
    assert_eq!(row.get::<String, _>("execution_state"), "started");
    // Actual CLI production fields project_id/repo_root are optional. This
    // missing identity is not used as proof: the native producer's immutable
    // PID plus internal execution-kind establish positive process absence.
    let claimed: String = sqlx::query_scalar(
        "SELECT claimed_at FROM goal_attempts WHERE worker_session_id='child-worker'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    let lease_stamp = crate::harness_lease::lease_timestamp(
        chrono::DateTime::parse_from_rfc3339(&claimed)
            .unwrap()
            .with_timezone(&chrono::Utc)
            - chrono::Duration::seconds(1),
    );
    sqlx::query("INSERT INTO harness_process_leases(lease_id,lifecycle_id,session_id,working_dir,mode,pid,phase,started_at,heartbeat_at) VALUES('ended','child-lifecycle','coordinator',?,'headless',?,'exited',?,?)")
        .bind(fixture.root.to_string_lossy().as_ref()).bind(i64::from(pid)).bind(&lease_stamp).bind(&lease_stamp).execute(&fixture.pool).await.unwrap();
    let original: String = sqlx::query_scalar(
        "SELECT contract_json FROM goal_execution_bindings WHERE goal_id='goal'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO cost_reservations VALUES('held-session',NULL,'openai','gpt-5.4-mini','unknown','started_possible','retained-hold')").execute(&fixture.pool).await.unwrap();
    let hold_before: Vec<(String, String)> =
        sqlx::query_as("SELECT invocation_id,state FROM cost_reservations")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    let NativeInspection::Candidate(boot) =
        goal_recovery::observe_lost_native_worker(&fixture.pool, None, "goal")
            .await
            .unwrap()
    else {
        panic!("Expected actual owner loss observation");
    };
    let NativeInspection::Candidate(live) =
        goal_recovery::inspect_native_goal(&fixture.pool, None, "goal")
            .await
            .unwrap()
    else {
        panic!("Expected same retained candidate");
    };
    assert_eq!(boot, live);
    assert!(matches!(boot, LineageResolution::Verified(_)));
    let state: String = sqlx::query_scalar(
        "SELECT execution_state FROM goal_attempts WHERE worker_session_id='child-worker'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(state, "lost");
    let unchanged: String = sqlx::query_scalar(
        "SELECT contract_json FROM goal_execution_bindings WHERE goal_id='goal'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(original, unchanged);
    let hold_after: Vec<(String, String)> =
        sqlx::query_as("SELECT invocation_id,state FROM cost_reservations")
            .fetch_all(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(hold_before, hold_after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cost_ledger")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM goal_attempt_provider_calls")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn actual_live_foreign_lease_and_human_cancel_refuse_native_loss() {
    use crate::goal_recovery::{self, NativeInspection};
    let fixture = Fixture::new().await;
    produce_dead_native_worker(&fixture).await;
    let mut live = tokio::process::Command::new("/bin/sleep")
        .arg("60")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let now = crate::harness_lease::lease_timestamp(chrono::Utc::now());
    sqlx::query("INSERT INTO harness_process_leases(lease_id,lifecycle_id,session_id,working_dir,mode,pid,phase,started_at,heartbeat_at) VALUES('foreign','child-lifecycle','coordinator',?,'headless',?,'turn',?,?)")
        .bind(fixture.root.to_string_lossy().as_ref()).bind(i64::from(live.id().unwrap())).bind(&now).bind(&now).execute(&fixture.pool).await.unwrap();
    assert!(matches!(
        goal_recovery::observe_lost_native_worker(&fixture.pool, None, "goal")
            .await
            .unwrap(),
        NativeInspection::StillOwned(_)
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM goal_candidate_observations")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    live.kill().await.unwrap();
    live.wait().await.unwrap();
    sqlx::query("DELETE FROM harness_process_leases WHERE lease_id='foreign'")
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert!(matches!(
        goal_recovery::inspect_native_goal(&fixture.pool, None, "goal")
            .await
            .unwrap(),
        NativeInspection::Lost(_)
    ));
    sqlx::query("INSERT INTO board_columns VALUES('cancelled','cancelled')")
        .execute(&fixture.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE cards SET column_id='cancelled' WHERE id='goal'")
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert!(matches!(
        goal_recovery::observe_lost_native_worker(&fixture.pool, None, "goal")
            .await
            .unwrap(),
        NativeInspection::Unverifiable(_)
    ));
    assert!(matches!(
        observe_worker_terminal(
            &fixture.pool,
            "goal",
            "child-claim",
            "child-worker",
            TerminalState::Lost,
            "old loss callback resumed",
            ReceiptState::Failed
        )
        .await
        .unwrap(),
        TerminalObservation::Stale
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM goal_candidate_observations")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn ownership_migration_keeps_legacy_pid_and_execution_kind_unknown() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE goal_attempts(attempt_id TEXT PRIMARY KEY,execution_state TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO goal_attempts VALUES('legacy','started')")
        .execute(&pool)
        .await
        .unwrap();
    crate::goal_recovery::apply_schema(&pool).await.unwrap();
    crate::goal_recovery::apply_schema(&pool).await.unwrap();
    let row=sqlx::query("SELECT owner_process_id,execution_kind,execution_state FROM goal_attempts WHERE attempt_id='legacy'").fetch_one(&pool).await.unwrap();
    assert!(row.get::<Option<i64>, _>("owner_process_id").is_none());
    assert!(row.get::<Option<String>, _>("execution_kind").is_none());
    assert_eq!(row.get::<String, _>("execution_state"), "started");
}
