use super::{
    findings::{self, FindingTier},
    inventory::{self, ConsumerState, ScanInput},
    report,
};
use std::{collections::BTreeMap, path::Path};

fn fixture(root: &Path) -> ScanInput {
    let input = ScanInput {
        data: root.into(),
        config: root.join("config.yaml"),
        ollama: root.join("ollama/models"),
        voice: root.join("voice"),
        environment: BTreeMap::new(),
    };
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(&input.voice).unwrap();
    std::fs::create_dir_all(
        input
            .ollama
            .join("manifests/registry.ollama.ai/library/qwen2.5"),
    )
    .unwrap();
    std::fs::create_dir_all(input.ollama.join("blobs")).unwrap();
    let digest = "a".repeat(64);
    std::fs::write(
        input.ollama.join("blobs").join(format!("sha256-{digest}")),
        b"local bytes",
    )
    .unwrap();
    std::fs::write(
        input
            .ollama
            .join("manifests/registry.ollama.ai/library/qwen2.5/7b"),
        serde_json::json!({"layers":[{"digest": format!("sha256:{digest}")}]}).to_string(),
    )
    .unwrap();
    let alias = input
        .ollama
        .join("manifests/registry.ollama.ai/library/qwen25-16k");
    std::fs::create_dir_all(&alias).unwrap();
    std::fs::write(
        alias.join("latest"),
        serde_json::json!({"layers":[{"digest":format!("sha256:{digest}")}]}).to_string(),
    )
    .unwrap();
    std::fs::write(root.join("models/fixture-Q4_K_M.gguf"), b"registry bytes").unwrap();
    std::fs::write(root.join("models/registry.json"), serde_json::json!({"models":[{
        "id":"fixture", "repo_id":"fixture", "local_path":root.join("models/fixture-Q4_K_M.gguf"), "quantization":"Q4_K_M"
    }]}).to_string()).unwrap();
    std::fs::write(input.voice.join("kokoro-v1.0.onnx"), b"voice bytes").unwrap();
    std::fs::write(input.voice.join("voices-v1.0.bin"), b"voice bank").unwrap();
    std::fs::write(root.join("models/whisper-base-q8_0.gguf"), b"whisper bytes").unwrap();
    std::fs::write(&input.config, "PERMAGENT_ROLE_EDIT_PROVIDER: ollama\nPERMAGENT_ROLE_EDIT_MODEL: qwen2.5:7b\nPERMAGENT_PACK_LOCAL_PROVIDER: ollama\nPERMAGENT_PACK_LOCAL_MODEL: qwen2.5:7b\nLOCAL_WHISPER_MODEL: base\n").unwrap();
    std::fs::write(
        root.join("librarian_schedule.json"),
        r#"{"model":"qwen2.5:7b"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("harvester.toml"),
        "[models]\ngeneration = \"qwen2.5:7b\"\nembedding = \"qwen3-embedding:0.6b\"\n",
    )
    .unwrap();
    input
}

#[test]
fn all_local_consumers_are_accounted_for() {
    let dir = tempfile::tempdir().unwrap();
    let inv = inventory::scan(&fixture(dir.path()));
    // Expected IDs and paths copied from scan's consumer contracts / backends.
    for id in [
        "registry:fixture",
        "kokoro",
        "kokoro voices",
        "LOCAL_WHISPER_MODEL",
        "PERMAGENT_ROLE_EDIT_MODEL",
        "PERMAGENT_PACK_LOCAL_MODEL",
        "librarian_schedule.model",
        "paused split",
        "harvester.generation",
        "harvester.embedding",
        "reader",
        "initiative",
        "playbook",
        "toolshim",
    ] {
        assert!(inv.consumers.iter().any(|c| c.id == id), "missing {id}");
    }
    assert!(inv
        .consumers
        .iter()
        .any(|c| c.id.starts_with("ollama:") && c.identity.as_deref() == Some("qwen2.5:7b")));
    assert_eq!(
        inv.consumers
            .iter()
            .find(|c| c.id == "paused split")
            .unwrap()
            .state,
        ConsumerState::Paused
    );
    assert!(inv
        .consumers
        .iter()
        .all(|c| c.state != ConsumerState::Active));
    assert!(inv
        .consumers
        .iter()
        .any(|c| c.identity.as_deref() == Some("qwen25-16k:latest")));
    let discoveries = findings::discover(&inv, &findings::snapshot().unwrap(), chrono::Utc::now());
    assert!(discoveries
        .iter()
        .filter(|f| f
            .discovery
            .incumbent_identity
            .starts_with("qwen25-16k:latest"))
        .all(|f| f.tier == FindingTier::UpstreamUnknown));
    assert!(inv
        .artifacts
        .iter()
        .any(|a| a.quantization.as_deref() == Some("Q4_K_M")));
    assert!(!dir.path().join("models/registry.json.lock").exists());
    let second = inventory::scan(&fixture(dir.path()));
    assert_eq!(
        serde_json::to_value(&inv).unwrap(),
        serde_json::to_value(second).unwrap()
    );
}

#[test]
fn sovereign_discovery_transport_source_secondary_tripwire() {
    // Secondary tripwire only. The behavioral acceptance test lives beside
    // execute_job in scheduler.rs and observes the runtime audit/cost sinks.
    for source in [
        include_str!("inventory.rs"),
        include_str!("findings.rs"),
        include_str!("report.rs"),
    ] {
        for forbidden in [
            "reqwest",
            "hyper::",
            "TcpStream",
            "TcpSocket",
            "ureq",
            "Command::",
            "create_provider",
            "new_scheduled_job_agent",
        ] {
            assert!(
                !source.contains(forbidden),
                "unexpected capability: {forbidden}"
            );
        }
    }
}

#[test]
fn changed_bytes_are_not_better_models() {
    let dir = tempfile::tempdir().unwrap();
    let mut inv = inventory::scan(&fixture(dir.path()));
    let known = findings::snapshot().unwrap();
    for digest in ["sha256:changed", "sha256:unknown"] {
        for a in &mut inv.artifacts {
            a.digest = Some(digest.into());
        }
        for now in ["2026-09-22T00:00:00Z", "2027-01-01T00:00:00Z"] {
            let now = chrono::DateTime::parse_from_rfc3339(now)
                .unwrap()
                .with_timezone(&chrono::Utc);
            let result = findings::discover(&inv, &known, now);
            assert!(result.iter().all(|f| !matches!(
                f.tier,
                FindingTier::ValidatedSwitchRecommendation | FindingTier::UpdateAvailable
            )));
            if now.year() == 2027 {
                assert!(result
                    .iter()
                    .all(|f| f.tier != FindingTier::CandidateWorthEvaluating));
            }
        }
    }
}
use chrono::Datelike;

#[tokio::test]
async fn one_finding_has_one_decision() {
    let dir = tempfile::tempdir().unwrap();
    let inv = inventory::scan(&fixture(dir.path()));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::session::spectral_schema::apply_model_review_schema(&pool)
        .await
        .unwrap();
    let first = report::build(inv.clone(), chrono::Utc::now()).unwrap();
    let second = report::build(inv, chrono::Utc::now()).unwrap();
    report::persist_report(&pool, &first).await.unwrap();
    report::persist_report(&pool, &second).await.unwrap();
    assert_eq!(
        report::ledger(&pool).await.unwrap().len(),
        first.findings.len()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_review_reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    // Empty reports must advance latest while keeping historical ledger rows.
    let mut empty = report::build(Default::default(), chrono::Utc::now()).unwrap();
    empty.findings.clear();
    report::persist_report(&pool, &empty).await.unwrap();
    assert!(report::latest(&pool)
        .await
        .unwrap()
        .unwrap()
        .findings
        .is_empty());
    sqlx::query("CREATE TRIGGER fail_review BEFORE INSERT ON model_review_findings BEGIN SELECT RAISE(ABORT, 'fixture failure'); END")
        .execute(&pool).await.unwrap();
    let mut failure = first;
    failure.id = uuid::Uuid::new_v4().to_string();
    assert!(report::persist_report(&pool, &failure).await.is_err());
    assert_eq!(report::latest(&pool).await.unwrap().unwrap().id, empty.id);
}

#[tokio::test]
async fn model_review_fresh_and_upgrade_paths_install_the_same_tables() {
    let dir = tempfile::tempdir().unwrap();
    let manager = crate::session::SessionManager::new(dir.path().to_path_buf());
    let pool = manager.pool_clone().await.unwrap();
    let schema: Vec<(String, String)> = sqlx::query_as("SELECT name, sql FROM sqlite_master WHERE type='table' AND name LIKE 'model_review_%' ORDER BY name")
        .fetch_all(&pool).await.unwrap();
    // reports + findings (v59) and finding_states (v62, W1b).
    assert_eq!(schema.len(), 3);
    sqlx::query("DROP TABLE model_review_finding_states")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE model_review_findings")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE model_review_reports")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM schema_version")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO schema_version(version) VALUES (58)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    drop(manager);
    let reopened = crate::session::SessionManager::new(dir.path().to_path_buf());
    let pool = reopened.pool_clone().await.unwrap();
    let upgraded: Vec<(String, String)> = sqlx::query_as("SELECT name, sql FROM sqlite_master WHERE type='table' AND name LIKE 'model_review_%' ORDER BY name")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(schema, upgraded);
    let version: i32 = sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    // The full ladder runs on reopen, so later migrations stamp past 59.
    assert!(version >= 59, "schema version {version} is below v59");
}

/// W1b: review state lives beside the ledger and survives the weekly rescan;
/// deferrals lapse back to open; reopen keeps a decision link; a bad request
/// writes nothing; unknown keys are reported, not fatal.
#[tokio::test]
async fn ledger_state_survives_rescans_and_actions_are_atomic() {
    use super::ledger::{self, ActionError, FindingAction, FindingState};
    let dir = tempfile::tempdir().unwrap();
    let inv = inventory::scan(&fixture(dir.path()));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::session::spectral_schema::apply_model_review_schema(&pool)
        .await
        .unwrap();
    crate::session::spectral_schema::apply_model_review_states_schema(&pool)
        .await
        .unwrap();
    let now = chrono::Utc::now();
    let first = report::build(inv.clone(), now).unwrap();
    assert!(
        first.findings.len() >= 2,
        "fixture must yield at least two findings, got {}",
        first.findings.len()
    );
    report::persist_report(&pool, &first).await.unwrap();

    let entries = ledger::entries(&pool, now).await.unwrap();
    assert_eq!(entries.len(), first.findings.len());
    assert!(entries.iter().all(|e| e.state == FindingState::Open));
    let (a, b) = (entries[0].key.clone(), entries[1].key.clone());

    let out = ledger::apply_action(
        &pool,
        &[a.clone(), "[\"nope\"]".into()],
        FindingAction::Dismiss,
        None,
        now,
    )
    .await
    .unwrap();
    assert_eq!(out.updated, vec![a.clone()]);
    assert_eq!(out.unknown, vec!["[\"nope\"]".to_string()]);

    let until = (now + chrono::Duration::days(7)).to_rfc3339();
    ledger::apply_action(&pool, &[b.clone()], FindingAction::Defer, Some(&until), now)
        .await
        .unwrap();

    // The weekly rescan re-persists every finding; state must not reset.
    let second = report::build(inv, now).unwrap();
    report::persist_report(&pool, &second).await.unwrap();
    let state_of = |entries: &[ledger::LedgerEntry], k: &str| {
        entries.iter().find(|e| e.key == k).unwrap().state
    };
    let after = ledger::entries(&pool, now).await.unwrap();
    assert_eq!(state_of(&after, &a), FindingState::Dismissed);
    assert_eq!(state_of(&after, &b), FindingState::Deferred);
    assert_eq!(
        after.last().unwrap().state,
        FindingState::Dismissed,
        "dismissed sorts last"
    );

    // A lapsed deferral reads open without anyone touching it.
    let later = ledger::entries(&pool, now + chrono::Duration::days(8))
        .await
        .unwrap();
    assert_eq!(state_of(&later, &b), FindingState::Open);

    // Reopen keeps a decision link (set by W1c) and clears the state.
    sqlx::query(
        "UPDATE model_review_finding_states SET decision_id = 'dec_1' WHERE finding_key = ?",
    )
    .bind(&a)
    .execute(&pool)
    .await
    .unwrap();
    ledger::apply_action(&pool, &[a.clone()], FindingAction::Reopen, None, now)
        .await
        .unwrap();
    let reopened = ledger::entries(&pool, now).await.unwrap();
    let entry = reopened.iter().find(|e| e.key == a).unwrap();
    assert_eq!(entry.state, FindingState::Open);
    assert_eq!(entry.decision_id.as_deref(), Some("dec_1"));

    // Malformed requests write nothing.
    for (keys, action, until) in [
        (vec![], FindingAction::Dismiss, None),
        (vec![b.clone()], FindingAction::Defer, None),
        (
            vec![b.clone()],
            FindingAction::Defer,
            Some("tomorrow".to_string()),
        ),
        (
            vec![b.clone()],
            FindingAction::Defer,
            Some((now - chrono::Duration::hours(1)).to_rfc3339()),
        ),
        (
            vec![b.clone()],
            FindingAction::Defer,
            Some((now + chrono::Duration::days(181)).to_rfc3339()),
        ),
    ] {
        let err = ledger::apply_action(&pool, &keys, action, until.as_deref(), now)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Invalid(_)), "{err}");
    }
    let too_many: Vec<String> = (0..=ledger::MAX_KEYS_PER_ACTION)
        .map(|i| i.to_string())
        .collect();
    assert!(matches!(
        ledger::apply_action(&pool, &too_many, FindingAction::Dismiss, None, now).await,
        Err(ActionError::Invalid(_))
    ));
    let unchanged = ledger::entries(&pool, now).await.unwrap();
    assert_eq!(state_of(&unchanged, &b), FindingState::Deferred);
}
