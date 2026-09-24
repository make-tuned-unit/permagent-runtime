//! K1b batch admission tests. Real SQLite store, real reservation arithmetic,
//! real decisions + audit rows; nothing here is a scripted double.
use super::*;
use crate::config::GooseMode;
use crate::cost_router::budget::{BudgetCeilings, BudgetConfig, BudgetScope};
use crate::session::{SessionManager, SessionType};
use std::sync::Arc;
use tempfile::TempDir;

const LEASE_HOURS: i64 = 2;

/// Task gate/hard as given; the session scope is wide so the task scope is
/// the one under test unless a test says otherwise.
fn config(task_gate: f64, task_hard: f64) -> BudgetConfig {
    BudgetConfig {
        task: BudgetCeilings {
            soft: task_gate,
            gate: task_gate,
            hard: task_hard,
        },
        session: BudgetCeilings {
            soft: 100.0,
            gate: 100.0,
            hard: 200.0,
        },
    }
}

struct Fixture {
    dir: TempDir,
    sm: Arc<SessionManager>,
    pool: Pool<Sqlite>,
    owner: String,
    task: String,
}

/// A coordinator with a Council-owned child that holds the stable task, the
/// same shape `council::preparation` builds.
async fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let sm = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    let parent = sm
        .create_session(
            dir.path().to_path_buf(),
            "coordinator".into(),
            SessionType::User,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    let owner = sm
        .create_session_with_parent(
            Some(&parent.id),
            dir.path().to_path_buf(),
            "Council convene".into(),
            SessionType::SubAgent,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    let task = sm.begin_budget_task(&owner.id).await.unwrap();
    let pool = sm.pool_clone().await.unwrap();
    Fixture {
        dir,
        sm,
        pool,
        owner: owner.id,
        task,
    }
}

impl Fixture {
    fn request(
        &self,
        request_id: &str,
        plan_hash: &str,
        paid: usize,
        free: usize,
    ) -> BatchAdmissionRequest {
        let mut slots: Vec<BatchSlot> = (0..paid)
            .map(|i| BatchSlot {
                slot_id: format!("Round1:{i}"),
                invocation_id: format!("{request_id}-invocation-{i}"),
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                bound_usd: Some(0.25),
            })
            .collect();
        slots.extend((0..free).map(|i| BatchSlot {
            slot_id: format!("Free:{i}"),
            invocation_id: format!("{request_id}-free-{i}"),
            provider: "ollama".into(),
            model: Some("qwen2.5:7b".into()),
            bound_usd: None,
        }));
        BatchAdmissionRequest {
            request_id: request_id.into(),
            plan_hash: plan_hash.into(),
            owner_session_id: self.owner.clone(),
            slots,
            lease_until: (Utc::now() + chrono::Duration::hours(LEASE_HOURS)).to_rfc3339(),
            gate_approval: None,
        }
    }

    async fn admit(
        &self,
        request: &BatchAdmissionRequest,
        cfg: &BudgetConfig,
    ) -> Result<BatchAdmissionOutcome, BatchAdmissionError> {
        self.sm.reserve_invocation_batch(request, cfg).await
    }

    async fn child(&self, parent: &str) -> String {
        self.sm
            .create_session_with_parent(
                Some(parent),
                self.dir.path().to_path_buf(),
                "Council slot".into(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await
            .unwrap()
            .id
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }

    async fn reservations(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM cost_reservations").await
    }

    async fn consumed(&self) -> i64 {
        self.count(
            "SELECT COUNT(*) FROM cost_reservation_batches WHERE approval_decision_id IS NOT NULL",
        )
        .await
    }

    async fn state(&self, reservation_id: &str) -> (String, String) {
        sqlx::query_as("SELECT state, session_id FROM cost_reservations WHERE reservation_id = ?")
            .bind(reservation_id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// The card `council::preparation` raises on NeedsGate, built with the
    /// same helpers and the same proposal marker.
    async fn council_card(&self, request: &BatchAdmissionRequest, bound: f64) -> String {
        let mut card = crate::cost_router::budget::with_requesting_session_id(
            crate::cost_router::budget::reservation_gate_decision_request(
                BudgetScope::Task,
                0.0,
                0.0,
                bound,
                1.0,
                0.0,
                None,
                None,
            ),
            &request.owner_session_id,
        );
        card.payload["options"] = serde_json::json!([
            {"id": "approve", "label": "Approve bounded Council convene"},
            {"id": "cancel", "label": "Cancel this occurrence"}
        ]);
        card.payload["default"] = "cancel".into();
        card.payload["proposal"] =
            council_proposal_marker(&request.request_id, &request.plan_hash).into();
        crate::decisions::create_decision(&self.pool, card)
            .await
            .unwrap()
            .id
    }

    /// Answer as the authenticated operator (HTTP route shape).
    async fn approve_as_operator(&self, decision_id: &str) {
        crate::decisions::answer_decision_with_principal(
            &self.pool,
            decision_id,
            &crate::decisions::DecisionAnswer {
                answer: "choice".into(),
                note: None,
                choice_id: Some("approve".into()),
                input_text: None,
            },
            crate::decisions::ACTOR_JESSE,
            "master",
        )
        .await
        .unwrap();
    }
}

fn with_approval(mut request: BatchAdmissionRequest, decision_id: &str) -> BatchAdmissionRequest {
    request.gate_approval = Some(BatchGateApproval {
        decision_id: decision_id.into(),
        plan_hash: request.plan_hash.clone(),
    });
    request
}

#[tokio::test]
async fn batch_admission_is_idempotent_on_request_id_and_plan() {
    let f = fixture().await;
    let cfg = config(10.0, 20.0);
    let request = f.request("req-1", "hash-a", 4, 1);
    let admitted = f.admit(&request, &cfg).await.unwrap();
    assert!(matches!(
        admitted,
        BatchAdmissionOutcome::Admitted { paid_slots: 4, reserved_usd, .. } if (reserved_usd - 1.0).abs() < 1e-12
    ));
    assert_eq!(
        f.reservations().await,
        4,
        "one ordinary reservation per PAID slot"
    );
    assert_eq!(
        f.count("SELECT COUNT(*) FROM cost_reservation_batch_members")
            .await,
        5,
        "free slots are recorded in the manifest"
    );
    assert_eq!(
        f.count("SELECT COUNT(*) FROM cost_reservations WHERE state = 'pending' AND task_id IS NOT NULL")
            .await,
        4
    );

    assert!(matches!(
        f.admit(&request, &cfg).await.unwrap(),
        BatchAdmissionOutcome::AlreadyAdmitted { .. }
    ));
    let mut changed_hash = request.clone();
    changed_hash.plan_hash = "hash-b".into();
    assert!(matches!(
        f.admit(&changed_hash, &cfg).await.unwrap(),
        BatchAdmissionOutcome::PlanChanged { admitted_plan_hash, .. } if admitted_plan_hash == "hash-a"
    ));
    // Same claimed hash, different slots: the server digest catches it.
    let mut changed_slots = request.clone();
    changed_slots.slots[0].bound_usd = Some(0.5);
    assert!(matches!(
        f.admit(&changed_slots, &cfg).await.unwrap(),
        BatchAdmissionOutcome::PlanChanged { .. }
    ));
    assert_eq!(f.reservations().await, 4, "replays reserve nothing");
}

/// Ruling acceptance: eleven individually affordable calls whose SUM crosses
/// the gate need one decision and write NOTHING.
#[tokio::test]
async fn affordable_slots_whose_sum_crosses_the_gate_need_gate_and_write_nothing() {
    let f = fixture().await;
    let cfg = config(1.0, 10.0);
    let request = f.request("req-gate", "hash", 11, 0);
    match f.admit(&request, &cfg).await.unwrap() {
        BatchAdmissionOutcome::NeedsGate {
            scope,
            spent_usd,
            held_usd,
            requested_usd,
            ceiling_usd,
        } => {
            assert_eq!(scope, BudgetScope::Task);
            assert_eq!((spent_usd, held_usd, ceiling_usd), (0.0, 0.0, 1.0));
            assert!(
                (requested_usd - 2.75).abs() < 1e-12,
                "the SUM, not one slot"
            );
        }
        other => panic!("expected NeedsGate, got {other:?}"),
    }
    assert_eq!(f.reservations().await, 0);
    assert_eq!(
        f.count("SELECT COUNT(*) FROM cost_reservation_batches")
            .await,
        0
    );
    assert_eq!(
        f.count("SELECT COUNT(*) FROM cost_reservation_batch_members")
            .await,
        0
    );
}

#[tokio::test]
async fn hard_ceiling_refuses_even_with_a_valid_operator_approval() {
    let f = fixture().await;
    let cfg = config(1.0, 2.0);
    let request = f.request("req-hard", "hash", 11, 0);
    let card = f.council_card(&request, 2.75).await;
    f.approve_as_operator(&card).await;
    let outcome = f.admit(&with_approval(request, &card), &cfg).await.unwrap();
    assert!(
        matches!(outcome, BatchAdmissionOutcome::Refused { ceiling_usd, .. } if ceiling_usd == 2.0),
        "{outcome:?}"
    );
    assert_eq!(f.reservations().await, 0);
    assert_eq!(
        f.consumed().await,
        0,
        "a refused admission leaves the approval unconsumed"
    );
}

#[tokio::test]
async fn operator_approval_admits_once_and_replay_is_refused() {
    let f = fixture().await;
    let cfg = config(1.0, 10.0);
    let request = f.request("req-approved", "hash", 11, 1);
    assert!(matches!(
        f.admit(&request, &cfg).await.unwrap(),
        BatchAdmissionOutcome::NeedsGate { .. }
    ));
    let card = f.council_card(&request, 2.75).await;
    f.approve_as_operator(&card).await;
    let approved = with_approval(request.clone(), &card);
    assert!(matches!(
        f.admit(&approved, &cfg).await.unwrap(),
        BatchAdmissionOutcome::Admitted { paid_slots: 11, .. }
    ));
    assert_eq!(f.reservations().await, 11);
    assert_eq!(f.consumed().await, 1);
    // Same request replayed: idempotent, nothing new.
    assert!(matches!(
        f.admit(&approved, &cfg).await.unwrap(),
        BatchAdmissionOutcome::AlreadyAdmitted { .. }
    ));
    // The same approval presented for another request: refused, typed.
    let mut other = with_approval(f.request("req-other", "hash", 11, 0), &card);
    other.gate_approval.as_mut().unwrap().plan_hash = "hash".into();
    match f.admit(&other, &cfg).await {
        Err(BatchAdmissionError::ApprovalConsumed {
            decision_id,
            batch_id,
        }) => {
            assert_eq!(decision_id, card);
            assert_eq!(batch_id, "batch:req-approved");
        }
        other => panic!("expected ApprovalConsumed, got {other:?}"),
    }
    assert_eq!(f.reservations().await, 11, "replay reserved nothing");
}

#[tokio::test]
async fn headless_policy_and_unauthenticated_approvals_are_rejected() {
    let f = fixture().await;
    let cfg = config(1.0, 10.0);
    let request = f.request("req-headless", "hash", 11, 0);
    let card = f.council_card(&request, 2.75).await;
    // The headless policy's own answer shape (auto_approve_reservation_gate
    // cannot answer this card — no to_completion option — so write it as that
    // path would have).
    sqlx::query(
        "UPDATE decisions SET status = 'answered', answer = 'choice', answer_choice_id = 'approve',
         acted_by = ?, resolved_at = 'now' WHERE id = ?",
    )
    .bind(crate::decisions::ACTOR_HEADLESS_POLICY)
    .bind(&card)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.admit(&with_approval(request.clone(), &card), &cfg).await,
        Err(BatchAdmissionError::ApprovalUnauthorized { .. })
    ));
    assert_eq!(f.reservations().await, 0);
    assert_eq!(f.consumed().await, 0);

    // ACTOR_JESSE without an authenticated principal is not enough either.
    let request = f.request("req-noprincipal", "hash", 11, 0);
    let card = f.council_card(&request, 2.75).await;
    crate::decisions::answer_decision(
        &f.pool,
        &card,
        &crate::decisions::DecisionAnswer {
            answer: "choice".into(),
            note: None,
            choice_id: Some("approve".into()),
            input_text: None,
        },
        crate::decisions::ACTOR_JESSE,
    )
    .await
    .unwrap();
    assert!(matches!(
        f.admit(&with_approval(request, &card), &cfg).await,
        Err(BatchAdmissionError::ApprovalUnauthorized { .. })
    ));
    assert_eq!(f.reservations().await, 0);
    assert_eq!(f.consumed().await, 0);
}

#[tokio::test]
async fn mismatched_plan_approval_is_rejected_and_stays_unconsumed() {
    let f = fixture().await;
    let cfg = config(1.0, 10.0);
    let approved_plan = f.request("req-mismatch", "hash-approved", 11, 0);
    let card = f.council_card(&approved_plan, 2.75).await;
    f.approve_as_operator(&card).await;

    // The reference itself names another plan.
    let mut wrong_ref = with_approval(approved_plan.clone(), &card);
    wrong_ref.gate_approval.as_mut().unwrap().plan_hash = "hash-other".into();
    assert!(matches!(
        f.admit(&wrong_ref, &cfg).await,
        Err(BatchAdmissionError::ApprovalInvalid { .. })
    ));
    // The request's plan differs from the card's (claimed hash agrees with the
    // request, but the card was approved for another plan).
    let mut other_plan = approved_plan.clone();
    other_plan.plan_hash = "hash-other".into();
    assert!(matches!(
        f.admit(&with_approval(other_plan, &card), &cfg).await,
        Err(BatchAdmissionError::ApprovalInvalid { .. })
    ));
    // Same hash, but the slots now sum to a different bound than approved.
    let mut bigger = approved_plan.clone();
    bigger.slots[0].bound_usd = Some(0.5);
    assert!(matches!(
        f.admit(&with_approval(bigger, &card), &cfg).await,
        Err(BatchAdmissionError::ApprovalInvalid { .. })
    ));
    assert_eq!(f.reservations().await, 0);
    assert_eq!(f.consumed().await, 0);
    // Still unconsumed: the matching plan is admitted with it.
    assert!(matches!(
        f.admit(&with_approval(approved_plan, &card), &cfg)
            .await
            .unwrap(),
        BatchAdmissionOutcome::Admitted { .. }
    ));
    assert_eq!(f.consumed().await, 1);
}

#[tokio::test]
async fn claim_transfers_the_existing_reservation_and_creates_none() {
    let f = fixture().await;
    let cfg = config(10.0, 20.0);
    let request = f.request("req-claim", "hash", 3, 1);
    f.admit(&request, &cfg).await.unwrap();
    let batch = "batch:req-claim";
    let before = f.reservations().await;
    let child = f.child(&f.owner).await;
    let reservation_id = match f
        .sm
        .claim_batch_invocation(batch, "Round1:0", &child)
        .await
        .unwrap()
    {
        BatchClaimOutcome::Claimed {
            reservation_id,
            bound_usd,
        } => {
            assert_eq!(bound_usd, 0.25);
            reservation_id
        }
        other => panic!("expected Claimed, got {other:?}"),
    };
    assert_eq!(reservation_id, "invocation:req-claim-invocation-0");
    assert_eq!(
        f.reservations().await,
        before,
        "a claim never creates a reservation"
    );
    assert_eq!(
        f.state(&reservation_id).await,
        ("pending".to_string(), child.clone()),
        "attribution moves to the child that will make the call"
    );

    let second = f.child(&f.owner).await;
    assert_eq!(
        f.sm.claim_batch_invocation(batch, "Round1:0", &second)
            .await
            .unwrap(),
        BatchClaimOutcome::AlreadyClaimed {
            reservation_id: reservation_id.clone(),
            claimed_session_id: child.clone(),
        }
    );
    assert!(matches!(
        f.sm.claim_batch_invocation(batch, "Free:0", &second)
            .await
            .unwrap(),
        BatchClaimOutcome::NotEligible { .. }
    ));
    assert_eq!(
        f.sm.claim_batch_invocation(batch, "Round9:9", &second)
            .await
            .unwrap(),
        BatchClaimOutcome::UnknownSlot
    );
    // A session outside the owner's subtree cannot take a slot.
    let stranger =
        f.sm.create_session(
            f.dir.path().to_path_buf(),
            "stranger".into(),
            SessionType::User,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    f.sm.begin_budget_task(&stranger.id).await.unwrap();
    assert!(matches!(
        f.sm.claim_batch_invocation(batch, "Round1:1", &stranger.id)
            .await
            .unwrap(),
        BatchClaimOutcome::NotEligible { .. }
    ));
    assert_eq!(f.reservations().await, before);
}

#[tokio::test]
async fn close_releases_only_unclaimed_retains_claimed_and_is_idempotent() {
    let f = fixture().await;
    let cfg = config(10.0, 20.0);
    f.admit(&f.request("req-close", "hash", 4, 1), &cfg)
        .await
        .unwrap();
    let batch = "batch:req-close";
    let child = f.child(&f.owner).await;
    let BatchClaimOutcome::Claimed { reservation_id, .. } =
        f.sm.claim_batch_invocation(batch, "Round1:0", &child)
            .await
            .unwrap()
    else {
        panic!("claim failed");
    };
    let first = f.sm.close_reservation_batch(batch).await.unwrap();
    assert_eq!(first.released_slots, 3);
    assert_eq!(first.retained_slots, 1);
    assert!((first.released_usd - 0.75).abs() < 1e-12);
    assert_eq!(
        f.state(&reservation_id).await.0,
        "pending",
        "claimed stays funded"
    );
    assert_eq!(
        f.count("SELECT COUNT(*) FROM cost_reservations WHERE state = 'released'")
            .await,
        3
    );

    let second = f.sm.close_reservation_batch(batch).await.unwrap();
    assert_eq!(
        (
            second.released_slots,
            second.released_usd,
            second.retained_slots
        ),
        (0, 0.0, 1),
        "released counts only NEWLY released slots"
    );
    assert_eq!(
        f.sm.claim_batch_invocation(batch, "Round1:1", &child)
            .await
            .unwrap(),
        BatchClaimOutcome::BatchClosed
    );
    assert!(matches!(
        f.sm.close_reservation_batch("batch:nope").await,
        Err(BatchAdmissionError::UnknownBatch(_))
    ));
}

#[tokio::test]
async fn expiry_stops_new_claims_and_never_refunds_claimed() {
    let f = fixture().await;
    let cfg = config(10.0, 20.0);
    f.admit(&f.request("req-expire", "hash", 3, 0), &cfg)
        .await
        .unwrap();
    let batch = "batch:req-expire";
    let child = f.child(&f.owner).await;
    let BatchClaimOutcome::Claimed { reservation_id, .. } =
        f.sm.claim_batch_invocation(batch, "Round1:0", &child)
            .await
            .unwrap()
    else {
        panic!("claim failed");
    };
    let summary = f.sm.expire_reservation_batch(batch).await.unwrap();
    assert_eq!((summary.released_slots, summary.retained_slots), (2, 1));
    assert_eq!(f.state(&reservation_id).await.0, "pending");
    assert_eq!(
        f.sm.claim_batch_invocation(batch, "Round1:1", &child)
            .await
            .unwrap(),
        BatchClaimOutcome::BatchExpired
    );
    assert_eq!(
        f.sm.expire_reservation_batch(batch)
            .await
            .unwrap()
            .released_slots,
        0
    );
}

#[tokio::test]
async fn startup_reconciliation_expires_a_lapsed_batch() {
    let f = fixture().await;
    let cfg = config(10.0, 20.0);
    f.admit(&f.request("req-lapsed", "hash", 3, 0), &cfg)
        .await
        .unwrap();
    let child = f.child(&f.owner).await;
    f.sm.claim_batch_invocation("batch:req-lapsed", "Round1:0", &child)
        .await
        .unwrap();
    // Simulate the claim deadline passing while the daemon was down. The
    // plan-immutability trigger guards lease_until, so lift it for the test.
    sqlx::query("DROP TRIGGER trg_cost_reservation_batch_plan_immutable")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE cost_reservation_batches SET lease_until = '2000-01-01T00:00:00+00:00'
         WHERE batch_id = 'batch:req-lapsed'",
    )
    .execute(&f.pool)
    .await
    .unwrap();

    // Restart: a fresh manager on the same store runs the boot pass.
    let restarted = SessionManager::new(f.dir.path().to_path_buf());
    let pool = restarted.pool_clone().await.unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT status FROM cost_reservation_batches WHERE batch_id = 'batch:req-lapsed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "expired");
    let released: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cost_reservations WHERE state = 'released'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(released, 2, "only the unclaimed slots are released");
    let retained: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cost_reservations WHERE state IN ('pending', 'unknown')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retained, 1, "the claimed slot stays counted");
    // A second pass is a no-op.
    assert_eq!(
        restarted.reconcile_expired_batches().await.unwrap(),
        BatchExpirySweep::default()
    );
}

/// Liability = settled + pending + unknown: each paid slot counts exactly
/// once, before and after its claim moves it to the child. The projection
/// reads the task scope from the canonical ROOT's task, so this test makes the
/// batch owner a root (the nested shape is covered by the session scope, and
/// by the direct task-scope sum below, which is the reservation path's filter).
#[tokio::test]
async fn budget_projection_counts_each_paid_slot_exactly_once() {
    let mut f = fixture().await;
    let root =
        f.sm.create_session(
            f.dir.path().to_path_buf(),
            "Council weekly root".into(),
            SessionType::Scheduled,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    f.task = f.sm.begin_budget_task(&root.id).await.unwrap();
    f.owner = root.id;
    let cfg = config(10.0, 20.0);
    f.admit(&f.request("req-liability", "hash", 4, 2), &cfg)
        .await
        .unwrap();
    let held = |p: &crate::session::BudgetProjection| {
        (
            p.task.held_usd.unwrap_or(-1.0),
            p.session.held_usd.unwrap_or(-1.0),
        )
    };
    let task_sum = |pool: Pool<Sqlite>, task: String| async move {
        sqlx::query_scalar::<_, f64>(
            "SELECT CAST(COALESCE(SUM(amount_usd), 0.0) AS REAL) FROM cost_reservations
             WHERE task_id = ? AND state IN ('pending', 'unknown')",
        )
        .bind(task)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let admitted = f.sm.budget_projection(&f.owner, cfg).await.unwrap();
    assert_eq!(held(&admitted), (1.0, 1.0));
    assert_eq!(task_sum(f.pool.clone(), f.task.clone()).await, 1.0);

    let child = f.child(&f.owner).await;
    f.sm.claim_batch_invocation("batch:req-liability", "Round1:0", &child)
        .await
        .unwrap();
    let claimed = f.sm.budget_projection(&f.owner, cfg).await.unwrap();
    assert_eq!(
        held(&claimed),
        (1.0, 1.0),
        "a claim moves money, never adds it"
    );
    assert_eq!(task_sum(f.pool.clone(), f.task.clone()).await, 1.0);

    f.sm.close_reservation_batch("batch:req-liability")
        .await
        .unwrap();
    let closed = f.sm.budget_projection(&f.owner, cfg).await.unwrap();
    assert_eq!(
        held(&closed),
        (0.25, 0.25),
        "only the claimed slot stays held"
    );
    assert_eq!(task_sum(f.pool.clone(), f.task.clone()).await, 0.25);
}
