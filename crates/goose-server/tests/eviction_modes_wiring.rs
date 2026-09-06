//! Integration-wiring proofs for the re-enable-gate epic part B (eviction
//! ordering + effective-mode visibility), through the real handlers against an
//! AppState rooted at a throwaway PERMAGENT_PATH_ROOT.
//!
//! Covers:
//!   wire 1 — POST /sessions/{id}/reply with an ACTIVE bus request 400s
//!            BEFORE the provider/model staleness sync can evict the cached
//!            agent (pre-fix: the parked/running turn's agent was removed
//!            first, orphaning the turn);
//!   wire 2 — with the bus free but a live orchestrator-style registered
//!            cancel token, the staleness sync is DEFERRED whole: no metadata
//!            write, no eviction (a partial sync would make the session look
//!            non-stale next turn and strand the live agent on the old
//!            provider forever);
//!   wire 3 — GET /config reports `effective_goose_mode` resolved with env
//!            precedence, diverging from the env-blind YAML map — the signal
//!            Settings uses to warn that a GOOSE_MODE env override makes the
//!            YAML selection inert.
//!
//! Runs as its own integration-test binary (own process): the AgentManager
//! singleton and process env (PERMAGENT_PATH_ROOT, GOOSE_PROVIDER,
//! GOOSE_MODE) are process-wide, so this is the single test in the binary
//! (same pattern as decision_wiring.rs).

use axum::extract::{Path, State};
use axum::Json;

use permagent::config::GooseMode;
use permagent::conversation::message::Message;
use permagent::execution::manager::AgentManager;
use permagent::session::SessionType;
use permagent_daemon::routes::config_management::read_all_config;
use permagent_daemon::routes::session_events::{session_reply, SessionReplyRequest};

#[tokio::test(flavor = "multi_thread")]
async fn eviction_ordering_and_effective_mode_wiring() {
    // Throwaway data root for the whole process (single test in this binary).
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("PERMAGENT_PATH_ROOT", tmp.path());

    let state = permagent_daemon::state::AppState::new(true).await.unwrap();
    let manager = AgentManager::instance().await.unwrap();

    let session = state
        .session_manager()
        .create_session(
            tmp.path().to_path_buf(),
            "eviction-guard".into(),
            SessionType::User,
            GooseMode::default(),
        )
        .await
        .unwrap();

    // Prime the cached agent — the one a parked/running turn would live on.
    manager
        .get_or_create_agent(session.id.clone())
        .await
        .unwrap();
    assert!(manager.has_session(&session.id).await);

    // Make the session look provider-stale: a fresh session has no
    // provider_name, so any configured global provider diverges from it.
    // (get_goose_provider resolves env-first, same as the daemon.)
    std::env::set_var("GOOSE_PROVIDER", "openai");

    let reply_req = |request_id: String| SessionReplyRequest {
        request_id,
        user_message: Message::user().with_text("hello"),
        override_conversation: None,
        app_context: None,
        attachment_ids: Vec::new(),
    };

    // ── Wire 1: active bus request → 400 BEFORE any eviction ──
    let bus = state.get_or_create_event_bus(&session.id).await;
    let parked_request_id = uuid::Uuid::new_v4().to_string();
    let _parked_token = bus
        .try_register_request(parked_request_id.clone())
        .await
        .expect("bus starts free");

    let err = session_reply(
        State(state.clone()),
        Path(session.id.clone()),
        Json(reply_req(uuid::Uuid::new_v4().to_string())),
    )
    .await
    .expect_err("second request while one is active must be rejected");
    assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
    assert!(
        err.message.contains("already has an active request"),
        "unexpected 400 message: {}",
        err.message
    );
    assert!(
        manager.has_session(&session.id).await,
        "the parked turn's agent must SURVIVE a rejected staleness-sync attempt"
    );
    let session_after = state
        .session_manager()
        .get_session(&session.id, false)
        .await
        .unwrap();
    assert_eq!(
        session_after.provider_name, None,
        "staleness sync must be fully deferred while a request is active (no metadata write)"
    );

    // Free the bus slot for wire 2.
    bus.cleanup_request(&parked_request_id).await;
    assert!(bus.active_request_ids().await.is_empty());

    // ── Wire 2: bus free, but an orchestrator-style turn is live ──
    manager
        .try_register_cancel_token(&session.id, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    let accepted_request_id = uuid::Uuid::new_v4().to_string();
    let resp = session_reply(
        State(state.clone()),
        Path(session.id.clone()),
        Json(reply_req(accepted_request_id.clone())),
    )
    .await
    .expect("reply is accepted; only the staleness sync is deferred");
    assert_eq!(resp.0.request_id, accepted_request_id);

    assert!(
        manager.has_session(&session.id).await,
        "a live registered turn must not be evicted by the staleness sync"
    );
    let session_after = state
        .session_manager()
        .get_session(&session.id, false)
        .await
        .unwrap();
    assert_eq!(
        session_after.provider_name, None,
        "sync must be deferred WHOLE while a non-bus turn is live — a partial \
         metadata write would strand the live agent on the old provider forever"
    );

    // Settle the wire-2 reply task before mutating process env again
    // (set_var while concurrent non-Rust code reads the env is the classic
    // hazard): cancel via the bus and wait — bounded, best-effort — for the
    // request slot to free.
    for id in bus.active_request_ids().await {
        bus.cancel_request(&id).await;
    }
    for _ in 0..50 {
        if bus.active_request_ids().await.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    manager.unregister_cancel_token(&session.id).await;
    std::env::remove_var("GOOSE_PROVIDER");

    // ── Wire 3: effective_goose_mode resolves env over YAML ──
    std::env::set_var("GOOSE_MODE", "approve");
    let resp = read_all_config().await.unwrap();
    assert_eq!(
        resp.0.effective_goose_mode, "approve",
        "effective mode must honor the env var"
    );
    // The env-blind YAML map is what the Settings buttons control; in this
    // throwaway root it has no GOOSE_MODE at all — exactly the divergence the
    // panel now warns about instead of silently highlighting 'Automatic'.
    let yaml_mode = resp.0.config.get("GOOSE_MODE").cloned();
    assert_ne!(
        yaml_mode,
        Some(serde_json::Value::String("approve".into())),
        "YAML map must not reflect the env override"
    );

    std::env::remove_var("GOOSE_MODE");
    let resp = read_all_config().await.unwrap();
    assert_eq!(
        resp.0.effective_goose_mode, "auto",
        "with no env override and no YAML value, effective mode is the default"
    );
}
