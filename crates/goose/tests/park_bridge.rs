//! Provider-side permission parks ↔ Decision Inbox: in-memory round trips.
//!
//! Providers with `PermissionRouting::ActionRequired` (claude-code, ACP) park
//! mid-stream on an internal oneshot after yielding an ActionRequired tool
//! confirmation. These tests drive a real `Agent::reply` turn against a fake
//! provider that parks exactly that way and prove, fully in memory:
//!
//!   1. daemon population: the park FILES a `tool_approval` decision row
//!      before the client ever sees the action_required event, and answering
//!      from the row's own payload delivers through
//!      `Agent::handle_confirmation` into the provider's oneshot — the parked
//!      stream resumes and the turn completes (the #760 round-trip pattern,
//!      extended to provider parks).
//!   2. headless population (scheduled jobs): the park is auto-DENIED
//!      immediately — the turn completes, the provider sees DenyOnce, and no
//!      decision row is ever filed.
//!
//! This is a SEPARATE test binary on purpose: it calls
//! `decisions::mark_process_serves_inbox()`, which is process-wide and
//! irreversible. The `permagent` lib unit tests model the CLI process and
//! must never set that flag; each integration file gets its own process, so
//! both populations stay deterministic.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use futures::StreamExt;
use permagent::agents::{Agent, AgentEvent, AgentRunnerConfig, GoosePlatform, SessionConfig};
use permagent::config::permission::PermissionManager;
use permagent::config::GooseMode;
use permagent::conversation::message::{Message, MessageContent};
use permagent::decisions;
use permagent::model::ModelConfig;
use permagent::permission::permission_confirmation::PrincipalType;
use permagent::permission::{Permission, PermissionConfirmation};
use permagent::providers::base::{
    MessageStream, PermissionRouting, Provider, ProviderUsage, Usage,
};
use permagent::providers::errors::ProviderError;
use permagent::session::{Session, SessionManager, SessionType};
use tokio::sync::{oneshot, Mutex};

const REQUEST_ID: &str = "prov-park-req-1";
const TOOL_NAME: &str = "Write";
const FINAL_TEXT: &str = "turn finished after the park resolved";

/// A provider that parks exactly the way claude-code does: it yields an
/// ActionRequired tool confirmation, then awaits an internal oneshot that only
/// `handle_permission_confirmation` can complete, then finishes the turn.
struct ParkingProvider {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<PermissionConfirmation>>>>,
    /// The confirmation the parked stream actually received (delivery proof).
    received: Arc<Mutex<Option<PermissionConfirmation>>>,
}

impl ParkingProvider {
    fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            received: Arc::new(Mutex::new(None)),
        }
    }
}

impl std::fmt::Debug for ParkingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParkingProvider").finish()
    }
}

#[async_trait::async_trait]
impl Provider for ParkingProvider {
    fn get_name(&self) -> &str {
        "test-parking"
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("test").unwrap()
    }

    fn permission_routing(&self) -> PermissionRouting {
        PermissionRouting::ActionRequired
    }

    async fn handle_permission_confirmation(
        &self,
        request_id: &str,
        confirmation: &PermissionConfirmation,
    ) -> bool {
        let mut pending = self.pending.lock().await;
        if let Some(tx) = pending.remove(request_id) {
            let _ = tx.send(confirmation.clone());
            return true;
        }
        false
    }

    async fn stream(
        &self,
        _model_config: &ModelConfig,
        _session_id: &str,
        _system: &str,
        _messages: &[Message],
        _tools: &[rmcp::model::Tool],
    ) -> Result<MessageStream, ProviderError> {
        let pending = Arc::clone(&self.pending);
        let received = Arc::clone(&self.received);
        Ok(Box::pin(try_stream! {
            // Register the waiter BEFORE yielding — identical ordering to
            // claude_code.rs, so an answer arriving right after the yield
            // always finds the oneshot.
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(REQUEST_ID.to_string(), tx);

            let mut args = rmcp::model::JsonObject::new();
            args.insert("path".to_string(), serde_json::json!("/tmp/out.txt"));
            let action_msg = Message::assistant().with_action_required(
                REQUEST_ID.to_string(),
                TOOL_NAME.to_string(),
                args,
                None,
            );
            yield (Some(action_msg), None);

            // The park.
            let confirmation = rx.await.unwrap_or(PermissionConfirmation {
                principal_type: PrincipalType::Tool,
                permission: Permission::Cancel,
            });
            pending.lock().await.remove(REQUEST_ID);
            *received.lock().await = Some(confirmation);

            yield (Some(Message::assistant().with_text(FINAL_TEXT)), None);
            yield (
                None,
                Some(ProviderUsage::new("test".to_string(), Usage::default())),
            );
        }))
    }
}

/// Isolated agent + session + pool on a tempdir-backed SessionManager.
async fn bridge_test_agent(tmp: &tempfile::TempDir) -> (Agent, Session) {
    let session_manager = Arc::new(SessionManager::new(tmp.path().to_path_buf()));
    let agent = Agent::with_config(AgentRunnerConfig::new(
        Arc::clone(&session_manager),
        PermissionManager::instance(),
        None,
        GooseMode::Approve,
        true,
        GoosePlatform::GooseCli,
    ));
    let session = session_manager
        .create_session(
            tmp.path().to_path_buf(),
            "park-bridge round trip".to_string(),
            SessionType::User,
            GooseMode::Approve,
        )
        .await
        .expect("create session");
    (agent, session)
}

fn session_config(session: &Session) -> SessionConfig {
    SessionConfig {
        id: session.id.clone(),
        schedule_id: None,
        max_turns: None,
        retry_config: None,
    }
}

fn is_action_required(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Message(m) if m
            .content
            .iter()
            .any(|c| matches!(c, MessageContent::ActionRequired(_)))
    )
}

fn is_final_text(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::Message(m) if m.as_concat_text().contains(FINAL_TEXT)
    )
}

/// Daemon population: provider park → decision filed → answer FROM THE ROW's
/// payload → delivered into the provider oneshot → parked turn resumes and
/// completes. Also proves #766 honesty for provider routing: the first
/// delivery reports true, a second delivery of the same id reports false.
#[tokio::test(flavor = "multi_thread")]
async fn provider_park_files_decision_and_inbox_answer_unparks_provider() {
    decisions::mark_process_serves_inbox();

    let tmp = tempfile::tempdir().unwrap();
    let (agent, session) = bridge_test_agent(&tmp).await;
    let pool = agent
        .config
        .session_manager
        .pool_clone()
        .await
        .expect("pool");

    let provider = Arc::new(ParkingProvider::new());
    agent
        .update_provider(provider.clone(), &session.id)
        .await
        .expect("install provider");

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        let mut stream = agent
            .reply(
                Message::user().with_text("write the file please"),
                session_config(&session),
                None,
            )
            .await
            .expect("reply stream");

        // Drive until the park surfaces to the client.
        let mut saw_action_required = false;
        while let Some(event) = stream.next().await {
            if is_action_required(&event.expect("agent event")) {
                saw_action_required = true;
                break;
            }
        }
        assert!(
            saw_action_required,
            "provider park must surface action_required"
        );

        // The decision row must ALREADY exist (filed before the event was
        // yielded), and its payload must carry the routing keys the answer
        // path needs.
        let row = decisions::find_open_tool_approval_by_request_id(&pool, REQUEST_ID)
            .await
            .expect("decision lookup")
            .expect("provider park must file a tool_approval decision");
        assert_eq!(row.kind, "tool_approval");
        let payload: decisions::ToolApprovalPayload =
            serde_json::from_value(row.payload.clone()).expect("typed payload");
        assert_eq!(payload.request_id, REQUEST_ID);
        assert_eq!(payload.session_id, session.id);
        assert_eq!(payload.tool_name, TOOL_NAME);

        // Answer using only what the row carries — exactly what the daemon's
        // deliver_tool_confirmation does after `get_agent(payload.session_id)`.
        let delivered = agent
            .handle_confirmation(
                payload.request_id.clone(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: Permission::AllowOnce,
                },
            )
            .await;
        assert!(
            delivered,
            "the provider must consume the confirmation (honest delivery = true)"
        );

        // The parked provider stream resumes and the turn completes.
        let mut saw_final = false;
        while let Some(event) = stream.next().await {
            if is_final_text(&event.expect("agent event")) {
                saw_final = true;
            }
        }
        assert!(saw_final, "the unparked turn must run to completion");

        // Honesty: nobody is waiting anymore — a second delivery reports so.
        let redelivered = agent
            .handle_confirmation(
                REQUEST_ID.to_string(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission: Permission::AllowOnce,
                },
            )
            .await;
        assert!(
            !redelivered,
            "no waiter left — redelivery must report false"
        );
    })
    .await;
    outcome.expect("round trip must not hang");

    let received = provider.received.lock().await.clone();
    assert_eq!(
        received.map(|c| c.permission),
        Some(Permission::AllowOnce),
        "the provider oneshot must have received the inbox answer"
    );
}

/// Headless population (scheduled jobs): the provider park is auto-denied
/// immediately — no filing even though this process DOES serve the inbox, no
/// hang, and the provider sees DenyOnce.
#[tokio::test(flavor = "multi_thread")]
async fn headless_provider_park_auto_denies_completes_and_files_nothing() {
    decisions::mark_process_serves_inbox();

    let tmp = tempfile::tempdir().unwrap();
    let (agent, session) = bridge_test_agent(&tmp).await;
    agent.set_headless(true);
    let pool = agent
        .config
        .session_manager
        .pool_clone()
        .await
        .expect("pool");

    let provider = Arc::new(ParkingProvider::new());
    agent
        .update_provider(provider.clone(), &session.id)
        .await
        .expect("install provider");

    let outcome = tokio::time::timeout(Duration::from_secs(60), async {
        let mut stream = agent
            .reply(
                Message::user().with_text("write the file please"),
                session_config(&session),
                None,
            )
            .await
            .expect("reply stream");

        // Drive the WHOLE turn without ever answering anything: the headless
        // bridge must unpark the provider on its own.
        let mut saw_final = false;
        while let Some(event) = stream.next().await {
            if is_final_text(&event.expect("agent event")) {
                saw_final = true;
            }
        }
        assert!(
            saw_final,
            "headless turn must complete without an external answer"
        );
    })
    .await;
    outcome.expect("headless turn must never park/hang");

    let received = provider.received.lock().await.clone();
    assert_eq!(
        received.map(|c| c.permission),
        Some(Permission::DenyOnce),
        "headless bridge must deny the provider park"
    );
    assert!(
        decisions::find_open_tool_approval_by_request_id(&pool, REQUEST_ID)
            .await
            .expect("decision lookup")
            .is_none(),
        "headless agents must never file a tool_approval decision"
    );
}
