//! Decision Inbox end-to-end trace harness (Lane L3 Part B evidence).
//!
//! Ignored by default — run explicitly:
//!
//! ```sh
//! mkdir -p /tmp/di-trace-root/brain
//! cp ~/.permagent/brain/ontology.toml /tmp/di-trace-root/brain/
//! cargo test -p permagent --test decision_inbox_trace -- --ignored --nocapture
//! # optional: DI_TRACE_ROOT overrides the default /tmp/di-trace-root
//! ```
//!
//! Drives the full loop on a THROWAWAY data root (never the live
//! ~/.permagent):
//!
//!   worker blocker → escalate tool payload → SqlDecisionSink → decision row
//!   → answered as jesse through L1's answer path → parked goal resumes
//!   (re-dispatch eligible) → answer ingested into the Brain → recalled via
//!   recall_cascade.
//!
//! Everything is local (SQLite + Spectral). Zero cloud tokens: no provider
//! is constructed anywhere in this harness.

use std::sync::Arc;

use permagent::decision_inbox::{curation, escalate, learn, policy, sink};
use permagent::decisions::{self, DecisionAnswer};
use permagent::projects::PERSONAL_PROJECT_ID;

fn banner(step: &str) {
    println!("\n=== {} ===", step);
}

async fn state_of(pool: &permagent::sqlx::Pool<permagent::sqlx::Sqlite>, card_id: &str) -> String {
    let card = permagent::cards::get_card(pool, card_id)
        .await
        .unwrap()
        .expect("card gone");
    let col = permagent::cards::get_column(pool, &card.column_id)
        .await
        .unwrap()
        .expect("column gone");
    col.state_binding.unwrap_or_default()
}

#[tokio::test]
#[ignore = "end-to-end trace against a throwaway DI_TRACE_ROOT; run explicitly"]
async fn decision_inbox_end_to_end_trace() {
    // ── Safety: throwaway root only ──
    let root = std::env::var("DI_TRACE_ROOT").unwrap_or_else(|_| "/tmp/di-trace-root".to_string());
    let live = dirs::home_dir().unwrap_or_default().join(".permagent");
    assert_ne!(
        std::path::Path::new(&root),
        live.as_path(),
        "refusing to run against the live data root"
    );
    std::env::set_var("PERMAGENT_PATH_ROOT", &root);
    println!("Data root (throwaway): {}", root);

    // ── Open the spectral DB + ensure schema ──
    let db_path = permagent::config::paths::Paths::spectral_db();
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let pool = permagent::sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
        .await
        .unwrap();
    permagent::session::spectral_schema::init_spectral_db(&pool)
        .await
        .unwrap();
    println!("Spectral DB: {}", db_path.display());

    // ── Mount the Brain (fresh stores in the throwaway root) ──
    let brain_dir = permagent::config::paths::Paths::brain_dir();
    let ontology = permagent::config::paths::Paths::brain_ontology();
    assert!(
        ontology.exists(),
        "brain/ontology.toml missing in throwaway root (copy it from the live root)"
    );
    let brain = tokio::task::spawn_blocking(move || {
        spectral::Brain::builder()
            .data_dir(&brain_dir)
            .ontology_path(&ontology)
            .device_id(spectral::DeviceId::from_descriptor("di-trace"))
            .build()
            .map(permagent::brain_handle::SafeBrain::new)
    })
    .await
    .unwrap()
    .expect("brain failed to mount");
    println!("Brain mounted");

    // ── 1. A goal is in progress; its worker hits a blocker ──
    banner("1. worker blocker (goal in_progress)");
    permagent::cards::seed_goal_columns(&pool, PERSONAL_PROJECT_ID)
        .await
        .unwrap();
    let col = permagent::cards::get_goal_column(&pool, PERSONAL_PROJECT_ID, "in_progress")
        .await
        .unwrap()
        .expect("no in_progress column");
    let goal = permagent::cards::create_card(
        &pool,
        permagent::cards::CreateCard {
            project_id: PERSONAL_PROJECT_ID.to_string(),
            title: "Add OAuth login to the web app".to_string(),
            description: Some("Support a third-party login provider".to_string()),
            card_type: Some("goal".to_string()),
            column_id: Some(col.id.clone()),
            created_by: None,
            metadata_json: Some(serde_json::json!({
                "goal_state": "in_progress",
                "attempt_count": 1,
            })),
        },
    )
    .await
    .unwrap();
    println!("goal card: {} ({})", goal.id, goal.title);
    println!("worker blocker: cannot pick an OAuth provider without a product decision");

    // ── 2. escalate tool call → decision row via SqlDecisionSink ──
    banner("2. escalate tool call -> decision row");
    escalate::install_decision_sink(Arc::new(sink::SqlDecisionSink::new(pool.clone())))
        .map_err(|_| ())
        .expect("sink already installed");
    let raw = serde_json::json!({
        "kind": "decision",
        "specific_ask": "Choose which login provider the new sign-in should support first",
        "why_blocked": "The OAuth callback, scopes, and app registration differ per provider; \
                        implementation diverges from here and the worker cannot proceed.",
        "evidence_refs": [format!("card:{}", goal.id), "session:sess-di-trace"],
        "options": [
            {"id": "github", "label": "GitHub", "consequence": "Developers sign in fastest; consumer users need accounts"},
            {"id": "google", "label": "Google", "consequence": "Widest consumer coverage; stricter app verification"}
        ],
        "resume": "auto"
    });
    let draft = escalate::draft_from_payload(
        raw,
        &escalate::DraftSource {
            session_id: Some("sess-di-trace".to_string()),
            worker_roster_name: Some("codex".to_string()),
        },
    );
    let recorded = escalate::global_decision_sink()
        .record(draft.clone())
        .await
        .unwrap();
    println!(
        "tool result -> {}",
        escalate::tool_result_text(&draft, &recorded.decision_id)
    );

    // Park the goal while it waits (the worker cannot continue).
    permagent::goal_transition::park_goal(
        &pool,
        &goal.id,
        decisions::ACTOR_SYSTEM,
        "blocked on login-provider decision",
    )
    .await
    .unwrap();

    // Curation pass: feed the rank column.
    let ranked = curation::rerank_open_decisions(&pool).await.unwrap();
    println!("curation: re-ranked {} open decision(s)", ranked);

    let decision = decisions::get_decision(&pool, &recorded.decision_id)
        .await
        .unwrap()
        .expect("decision row missing");
    println!(
        "decision row:\n{}",
        serde_json::to_string_pretty(&decision).unwrap()
    );
    assert_eq!(decision.kind, "choice");
    assert_eq!(decision.status, "open");
    assert_eq!(decision.goal_id.as_deref(), Some(goal.id.as_str()));
    assert!(decision.rank.is_some(), "curation must have set a rank");
    println!("goal state now: {}", state_of(&pool, &goal.id).await);
    assert_eq!(state_of(&pool, &goal.id).await, "triage");

    // ── 3. Jesse answers through L1's answer path ──
    banner("3. answered as jesse (L1 answer path)");
    let (answered, proof) = decisions::answer_decision(
        &pool,
        &decision.id,
        &DecisionAnswer {
            answer: "choice".to_string(),
            choice_id: Some("github".to_string()),
            note: Some("Start with GitHub — our first users are developers.".to_string()),
            ..Default::default()
        },
        decisions::ACTOR_JESSE,
    )
    .await
    .unwrap();
    println!(
        "answered row:\n{}",
        serde_json::to_string_pretty(&answered).unwrap()
    );
    assert_eq!(answered.status, "answered");
    assert_eq!(answered.acted_by.as_deref(), Some(decisions::ACTOR_JESSE));

    // ── 4. resume:auto — the parked goal becomes re-dispatch eligible ──
    banner("4. resume:auto (parked goal -> Ready)");
    let effect = policy::resume_answered_decision(&pool, &answered, proof)
        .await
        .unwrap();
    println!("effect: {}", effect.as_deref().unwrap_or("(none)"));
    assert!(effect.is_some(), "parked goal must resume");
    println!("goal state now: {}", state_of(&pool, &goal.id).await);
    assert_eq!(state_of(&pool, &goal.id).await, "ready");

    // ── 5. Learn: ingest into the Brain ──
    banner("5. learn ingestion (SafeBrain remember_with)");
    let result = learn::ingest_answered_decision(&pool, &brain, &answered)
        .await
        .unwrap();
    println!("remember result: {:?}", result);
    assert!(result.is_some(), "jesse-answered decision must be ingested");

    // ── 6. Recall via recall_cascade ──
    banner("6. recall_cascade (focus_wing=personal, decision: prefix)");
    let hits = learn::recall_decisions(
        &brain,
        "Which login provider should new sign-in work use?",
        "personal",
    )
    .await
    .unwrap();
    assert!(!hits.is_empty(), "decision memory was NOT recalled");
    for h in &hits {
        println!("[{:.3}] {} -> {}", h.signal_score, h.key, h.content);
    }
    let expected_key = learn::decision_memory_key("personal", &decision.id);
    assert!(
        hits.iter().any(|h| h.key == expected_key),
        "expected key {} in recall hits",
        expected_key
    );
    println!(
        "\ncontext block:\n{}",
        learn::format_decision_context_block(&hits).unwrap_or_default()
    );

    // Audit chain stays intact across the whole loop.
    let report = decisions::verify_audit_chain(&pool).await.unwrap();
    assert!(report.intact, "audit chain broken: {}", report.detail);
    println!("\naudit chain: {}", report.detail);

    banner("trace complete — zero cloud tokens (no provider constructed)");

    // The Brain owns an internal runtime; drop it off the async executor.
    tokio::task::spawn_blocking(move || drop(brain))
        .await
        .unwrap();
}

/// Edit-as-training end-to-end: a draft-carrying decision → approve-with-edits
/// (answer='edit') → BOTH acceptance and correction ingested → the correction
/// recalls at draft time, isolated by key prefix from the acceptance memory.
///
/// Same throwaway-root safety and zero-cloud-token guarantees as the trace
/// above (no provider is ever constructed).
#[tokio::test]
#[ignore = "end-to-end edit-as-training trace against a throwaway DI_TRACE_ROOT; run explicitly"]
async fn edit_as_training_end_to_end_trace() {
    // ── Safety: throwaway root only ──
    let root = std::env::var("DI_TRACE_ROOT").unwrap_or_else(|_| "/tmp/di-trace-root".to_string());
    let live = dirs::home_dir().unwrap_or_default().join(".permagent");
    assert_ne!(
        std::path::Path::new(&root),
        live.as_path(),
        "refusing to run against the live data root"
    );
    std::env::set_var("PERMAGENT_PATH_ROOT", &root);
    println!("Data root (throwaway): {}", root);

    // ── Open the spectral DB + ensure schema ──
    let db_path = permagent::config::paths::Paths::spectral_db();
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let pool = permagent::sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
        .await
        .unwrap();
    permagent::session::spectral_schema::init_spectral_db(&pool)
        .await
        .unwrap();

    // ── Mount the Brain (fresh stores in the throwaway root) ──
    let brain_dir = permagent::config::paths::Paths::brain_dir();
    let ontology = permagent::config::paths::Paths::brain_ontology();
    assert!(
        ontology.exists(),
        "brain/ontology.toml missing in throwaway root (copy it from the live root)"
    );
    let brain = tokio::task::spawn_blocking(move || {
        spectral::Brain::builder()
            .data_dir(&brain_dir)
            .ontology_path(&ontology)
            .device_id(spectral::DeviceId::from_descriptor("di-edit-trace"))
            .build()
            .map(permagent::brain_handle::SafeBrain::new)
    })
    .await
    .unwrap()
    .expect("brain failed to mount");
    println!("Brain mounted");

    // ── 1. A draft-carrying proposal lands on the inbox (payload.draft) ──
    banner("1. draft-carrying automation_proposal");
    let original_draft = "Save `git status && git pull` as a recipe you can run on demand.";
    let d = decisions::create_decision(
        &pool,
        decisions::NewDecision {
            kind: "automation_proposal".to_string(),
            // project_id None → slug resolves deterministically to "personal".
            headline: Some("Automate your morning git sync?".to_string()),
            detail: Some(original_draft.to_string()),
            payload: serde_json::json!({
                "normalized_command": "git status && git pull",
                "occurrence_count": 3,
                "exemplars": ["git status && git pull"],
                "draft": original_draft,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        d.kind, "automation_proposal",
        "draft must not malform the row"
    );
    assert_eq!(d.payload["draft"].as_str(), Some(original_draft));

    // ── 2. Jesse revises the draft, then accepts (answer='edit') ──
    banner("2. approve-with-edits (answer='edit')");
    let revised = "Save `git status && git pull --rebase` as a recipe; I always rebase.";
    let (answered, _proof) = decisions::answer_decision(
        &pool,
        &d.id,
        &DecisionAnswer {
            answer: "edit".to_string(),
            input_text: Some(revised.to_string()),
            ..Default::default()
        },
        decisions::ACTOR_JESSE,
    )
    .await
    .unwrap();
    assert_eq!(answered.answer.as_deref(), Some("edit"));
    assert_eq!(answered.answer_input.as_deref(), Some(revised));

    // ── 3. Learn: an edit is BOTH an acceptance AND a correction ──
    banner("3. ingest acceptance + correction");
    let accept = learn::ingest_answered_decision(&pool, &brain, &answered)
        .await
        .unwrap();
    assert!(accept.is_some(), "an edit is also an acceptance");
    let correction = learn::ingest_edited_decision(&pool, &brain, &answered)
        .await
        .unwrap();
    assert!(
        correction.is_some(),
        "the draft→revision delta becomes a correction memory"
    );

    // ── 4. Recall corrections at draft time; prove prefix isolation ──
    banner("4. recall_corrections (correction: prefix)");
    let corr_hits = learn::recall_corrections(&brain, "automate git pull recipe", "personal")
        .await
        .unwrap();
    for h in &corr_hits {
        println!("[corr {:.3}] {} -> {}", h.signal_score, h.key, h.content);
    }
    assert!(!corr_hits.is_empty(), "correction memory was NOT recalled");
    let corr_key = learn::correction_memory_key("personal", &d.id);
    let dec_key = learn::decision_memory_key("personal", &d.id);
    assert!(
        corr_hits.iter().any(|h| h.key == corr_key),
        "expected correction key {} in hits",
        corr_key
    );
    assert!(
        corr_hits.iter().all(|h| h.key != dec_key),
        "decision: memory leaked into correction recall"
    );

    // ── 5. The reverse isolation: decision recall excludes the correction ──
    banner("5. recall_decisions excludes corrections");
    let dec_hits = learn::recall_decisions(&brain, "automate git pull recipe", "personal")
        .await
        .unwrap();
    assert!(
        dec_hits.iter().all(|h| h.key != corr_key),
        "correction: memory leaked into decision recall"
    );

    // ── 6. The injected draft-time block quotes the revision (data, not cmd) ──
    let block = learn::format_correction_context_block(&corr_hits).unwrap_or_default();
    println!("\ncorrection context block:\n{}", block);
    assert!(block.contains("Jesse revised it to:"));

    let report = decisions::verify_audit_chain(&pool).await.unwrap();
    assert!(report.intact, "audit chain broken: {}", report.detail);

    banner("edit-as-training trace complete — zero cloud tokens (no provider constructed)");
    tokio::task::spawn_blocking(move || drop(brain))
        .await
        .unwrap();
}
