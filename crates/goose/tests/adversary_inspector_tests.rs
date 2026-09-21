use permagent::config::GooseMode;
use permagent::conversation::message::{Message, MessageContent, ToolRequest};
use permagent::security::adversary_inspector::AdversaryInspector;
use permagent::tool_inspection::ToolInspector;
use rmcp::model::CallToolRequestParams;
use rmcp::object;
use std::sync::Arc;
use tokio::sync::Mutex;

fn make_request(
    id: &str,
    tool: &str,
    args: serde_json::Map<String, serde_json::Value>,
) -> ToolRequest {
    ToolRequest {
        id: id.into(),
        tool_call: Ok(CallToolRequestParams::new(tool.to_string()).with_arguments(args)),
        metadata: None,
        tool_meta: None,
    }
}

fn write_adversary_md(dir: &std::path::Path, content: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("adversary.md"), content).unwrap();
}

#[tokio::test]
async fn test_adversary_disabled_without_config_file() {
    let tmp = tempfile::tempdir().unwrap();

    let provider = Arc::new(Mutex::new(None));
    let inspector = AdversaryInspector::with_config_dir(provider, tmp.path().to_path_buf());

    assert_eq!(inspector.name(), "adversary");
    assert!(!inspector.is_enabled());

    let results = inspector
        .inspect(
            "test-session",
            &[make_request(
                "r1",
                "shell",
                object!({"command": "rm -rf /"}),
            )],
            &[],
            GooseMode::SmartApprove,
        )
        .await
        .unwrap();

    assert!(results.is_empty());
}

#[tokio::test]
async fn test_adversary_enabled_default_tools() {
    let tmp = tempfile::tempdir().unwrap();
    write_adversary_md(tmp.path(), "BLOCK everything for testing");

    let provider = Arc::new(Mutex::new(None));
    let inspector = AdversaryInspector::with_config_dir(provider, tmp.path().to_path_buf());

    assert!(inspector.is_enabled());

    let messages = vec![Message::new(
        rmcp::model::Role::User,
        chrono::Utc::now().timestamp(),
        vec![MessageContent::text("build the project")],
    )];

    // shell is reviewed by default — no provider means fail-closed (RequireApproval)
    let results = inspector
        .inspect(
            "test-session",
            &[make_request(
                "r1",
                "shell",
                object!({"command": "cargo build"}),
            )],
            &messages,
            GooseMode::SmartApprove,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0].action,
        permagent::tool_inspection::InspectionAction::RequireApproval(Some(_))
    ));

    assert!(results[0].reason.contains("No provider available"));

    // write is NOT reviewed by default — skipped entirely
    let results = inspector
        .inspect(
            "test-session",
            &[make_request(
                "r1",
                "write",
                object!({"path": "foo.txt", "content": "hi"}),
            )],
            &messages,
            GooseMode::SmartApprove,
        )
        .await
        .unwrap();

    assert!(results.is_empty());
}

#[tokio::test]
async fn test_adversary_custom_tool_filter() {
    let tmp = tempfile::tempdir().unwrap();
    write_adversary_md(
        tmp.path(),
        "tools: shell, computercontroller__automation_script\n---\nBLOCK bad stuff",
    );

    let provider = Arc::new(Mutex::new(None));
    let inspector = AdversaryInspector::with_config_dir(provider, tmp.path().to_path_buf());

    assert!(inspector.is_enabled());

    let messages = vec![Message::new(
        rmcp::model::Role::User,
        chrono::Utc::now().timestamp(),
        vec![MessageContent::text("do something")],
    )];

    // shell — reviewed
    let results = inspector
        .inspect(
            "test",
            &[make_request("r1", "shell", object!({"command": "ls"}))],
            &messages,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // automation_script — reviewed
    let results = inspector
        .inspect(
            "test",
            &[make_request(
                "r2",
                "computercontroller__automation_script",
                object!({"script": "echo hi", "language": "shell"}),
            )],
            &messages,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // write — NOT reviewed
    let results = inspector
        .inspect(
            "test",
            &[make_request(
                "r3",
                "write",
                object!({"path": "x.txt", "content": "y"}),
            )],
            &messages,
            GooseMode::Auto,
        )
        .await
        .unwrap();
    assert!(results.is_empty());
}

// Follows MockAuthErrorProvider in tests/agent.rs; no external provider is called.
struct AdversaryErrorProvider {
    called: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl permagent::providers::base::Provider for AdversaryErrorProvider {
    fn get_name(&self) -> &str {
        "adversary-error-test"
    }

    fn get_model_config(&self) -> permagent::model::ModelConfig {
        permagent::model::ModelConfig::new("mock-model").unwrap()
    }

    fn cost_tier(&self) -> permagent::session::CostTier {
        permagent::session::CostTier::LocalFree
    }

    async fn stream(
        &self,
        _model_config: &permagent::model::ModelConfig,
        _session_id: &str,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[rmcp::model::Tool],
    ) -> Result<
        permagent::providers::base::MessageStream,
        permagent::providers::errors::ProviderError,
    > {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        Err(permagent::providers::errors::ProviderError::Authentication(
            "adversary test authentication failure".to_string(),
        ))
    }
}

#[tokio::test]
async fn test_adversary_provider_error_requires_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = Arc::new(permagent::session::SessionManager::new(
        tmp.path().to_path_buf(),
    ));
    let session = manager
        .create_session(
            tmp.path().to_path_buf(),
            "adversary-provider-error".to_string(),
            permagent::session::SessionType::Hidden,
            GooseMode::SmartApprove,
        )
        .await
        .unwrap();
    write_adversary_md(tmp.path(), "BLOCK unsafe commands");
    let mock = Arc::new(AdversaryErrorProvider {
        called: std::sync::atomic::AtomicBool::new(false),
    });
    let provider: permagent::agents::types::SharedProvider =
        Arc::new(Mutex::new(Some(mock.clone())));
    let inspector = AdversaryInspector::with_config_dir(provider, tmp.path().to_path_buf())
        .with_session_manager(manager);
    let results = inspector
        .inspect(
            &session.id,
            &[make_request("r1", "shell", object!({"command": "ls"}))],
            &[],
            GooseMode::SmartApprove,
        )
        .await
        .unwrap();

    assert!(mock.called.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(results.len(), 1);
    assert!(matches!(
        &results[0].action,
        permagent::tool_inspection::InspectionAction::RequireApproval(Some(reason))
            if reason == &results[0].reason
    ));
    assert!(results[0]
        .reason
        .contains("adversary test authentication failure"));
    assert!(results[0].reason.contains("fail-closed"));
    assert_eq!(results[0].confidence, 0.0);
}
