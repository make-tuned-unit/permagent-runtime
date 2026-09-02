use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

use crate::config::GooseMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::permission::permission_inspector::PermissionInspector;
use crate::permission::permission_judge::PermissionCheckResult;

/// The one string to grep for when asking "was a safety inspector switched
/// off while this ran". Emitted on every affected call and echoed by
/// `permagent doctor`, so a disable cannot be silent (D34).
pub const SAFETY_DISABLE_MARKER: &str = "SAFETY_INSPECTOR_DISABLED";

/// A break-glass switch: something that is ON by default and can be turned OFF
/// out of band. `Config::get_param` reads the uppercased env var before the
/// config file, so every one of these is settable as a process env var with no
/// file trace — which is exactly why they have to be loud.
///
/// The dev machine needs these escape hatches, so nothing here removes them.
/// They are made impossible to use quietly instead.
pub struct SafetySwitch {
    /// The inspector (or check) this turns off, by its `ToolInspector::name()`
    /// where one exists.
    pub inspector: &'static str,
    /// The config key / env var that carries the disable.
    pub switch: &'static str,
    /// What is lost while it is set.
    pub loses: &'static str,
}

/// Every break-glass disable in the safety core, in one list so the runtime
/// WARN and the operator-facing `permagent doctor` check cannot drift.
pub const SAFETY_SWITCHES: &[SafetySwitch] = &[
    SafetySwitch {
        inspector: "write_jail",
        switch: "SECURITY_WRITE_JAIL_ENABLED",
        loses: "writes outside the session working directory are no longer flagged for approval",
    },
    SafetySwitch {
        inspector: "security",
        switch: "SECURITY_PROMPT_ENABLED",
        loses: "prompt-injection scanning of shell and write/edit calls does not run at all",
    },
    SafetySwitch {
        inspector: "security",
        switch: "SECURITY_PROMPT_LOG_ONLY",
        loses: "prompt-injection findings are logged but never block — the inspector still \
                reports itself as enabled",
    },
    SafetySwitch {
        inspector: "tool_argument_validation",
        switch: "GOOSE_TOOL_ARG_VALIDATION",
        loses: "tool arguments are dispatched without checking them against the tool's own \
                declared schema",
    },
];

/// Look a break-glass switch up by the inspector it disables.
pub fn safety_switch_for(inspector: &str) -> Option<&'static SafetySwitch> {
    SAFETY_SWITCHES.iter().find(|s| s.inspector == inspector)
}

/// One break-glass switch found active right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSafetyDisable {
    pub inspector: &'static str,
    pub switch: &'static str,
    pub loses: &'static str,
    /// The value that turned it off, for the operator to recognise.
    pub value: String,
}

/// Which break-glass switches are set in THIS process (env first, then the
/// config file — the same precedence the inspectors themselves read).
///
/// `permagent doctor` runs in its own process, so it sees the config file and
/// its own environment, not the daemon's. Say so wherever this is rendered.
pub fn active_safety_disables() -> Vec<ActiveSafetyDisable> {
    use crate::config::Config;
    let config = Config::global();
    let mut out = Vec::new();

    let write_jail_on = config
        .get_param::<bool>("SECURITY_WRITE_JAIL_ENABLED")
        .unwrap_or(true);
    if !write_jail_on {
        out.push(active("SECURITY_WRITE_JAIL_ENABLED", "false"));
    }

    if !crate::security::prompt_detection_enabled(
        config.get_param::<bool>("SECURITY_PROMPT_ENABLED").ok(),
    ) {
        out.push(active("SECURITY_PROMPT_ENABLED", "false"));
    }
    if crate::security::prompt_detection_log_only(
        config.get_param::<bool>("SECURITY_PROMPT_LOG_ONLY").ok(),
    ) {
        out.push(active("SECURITY_PROMPT_LOG_ONLY", "true"));
    }

    if crate::agents::schema_validation::ToolArgValidationMode::from_config()
        == crate::agents::schema_validation::ToolArgValidationMode::Off
    {
        out.push(active("GOOSE_TOOL_ARG_VALIDATION", "off"));
    }

    out
}

fn active(switch: &'static str, value: &str) -> ActiveSafetyDisable {
    let s = SAFETY_SWITCHES
        .iter()
        .find(|s| s.switch == switch)
        .expect("every emitted disable is declared in SAFETY_SWITCHES");
    ActiveSafetyDisable {
        inspector: s.inspector,
        switch: s.switch,
        loses: s.loses,
        value: value.to_string(),
    }
}

/// Result of inspecting a tool call
#[derive(Debug, Clone)]
pub struct InspectionResult {
    pub tool_request_id: String,
    pub action: InspectionAction,
    pub reason: String,
    pub confidence: f32,
    pub inspector_name: String,
    pub finding_id: Option<String>,
}

/// Action to take based on inspection result
#[derive(Debug, Clone, PartialEq)]
pub enum InspectionAction {
    /// Allow the tool to execute without user intervention
    Allow,
    /// Deny the tool execution completely
    Deny,
    /// Require user approval before execution (with optional warning message)
    RequireApproval(Option<String>),
}

/// Trait for all tool inspectors
#[async_trait]
pub trait ToolInspector: Send + Sync {
    /// Name of this inspector (for logging/debugging)
    fn name(&self) -> &'static str;

    /// Inspect tool requests and return results
    async fn inspect(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>>;

    /// Whether this inspector is enabled
    fn is_enabled(&self) -> bool {
        true
    }

    /// Allow downcasting to concrete types
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Manages all tool inspectors and coordinates their results
pub struct ToolInspectionManager {
    inspectors: Vec<Box<dyn ToolInspector>>,
}

impl ToolInspectionManager {
    pub fn new() -> Self {
        Self {
            inspectors: Vec::new(),
        }
    }

    /// Add an inspector to the manager
    /// Inspectors run in the order they are added
    pub fn add_inspector(&mut self, inspector: Box<dyn ToolInspector>) {
        self.inspectors.push(inspector);
    }

    /// Run all inspectors on the tool requests
    pub async fn inspect_tools(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        messages: &[Message],
        goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>> {
        let mut all_results = Vec::new();

        for inspector in &self.inspectors {
            if !inspector.is_enabled() {
                // A skipped safety inspector used to be an unlogged `continue`
                // — the disable was invisible at runtime and `inspector_names()`
                // still reported it as registered. Every call it does not
                // inspect now says so at WARN, carrying the marker (D34).
                match safety_switch_for(inspector.name()) {
                    Some(switch) => tracing::warn!(
                        marker = SAFETY_DISABLE_MARKER,
                        inspector_name = inspector.name(),
                        switch = switch.switch,
                        loses = switch.loses,
                        tool_count = tool_requests.len(),
                        session_id,
                        "safety inspector disabled — tool calls are running past it"
                    ),
                    // No break-glass switch: an opt-in inspector (the adversary
                    // reviewer) that was simply never configured. Nothing was
                    // turned off, so this is not an alarm.
                    None => tracing::debug!(
                        inspector_name = inspector.name(),
                        "inspector not active (not configured)"
                    ),
                }
                continue;
            }

            tracing::debug!(
                inspector_name = inspector.name(),
                tool_count = tool_requests.len(),
                "Running tool inspector"
            );

            match inspector
                .inspect(session_id, tool_requests, messages, goose_mode)
                .await
            {
                Ok(results) => {
                    tracing::debug!(
                        inspector_name = inspector.name(),
                        result_count = results.len(),
                        "Tool inspector completed"
                    );
                    all_results.extend(results);
                }
                Err(e) => {
                    tracing::error!(
                        inspector_name = inspector.name(),
                        error = %e,
                        "Tool inspector failed"
                    );
                    // Continue with other inspectors even if one fails
                }
            }
        }

        Ok(all_results)
    }

    /// Get list of registered inspector names
    pub fn inspector_names(&self) -> Vec<&'static str> {
        self.inspectors.iter().map(|i| i.name()).collect()
    }

    fn get_permission_inspector(&self) -> Option<&PermissionInspector> {
        self.inspectors
            .iter()
            .find(|i| i.name() == "permission")
            .and_then(|i| i.as_any().downcast_ref::<PermissionInspector>())
    }

    pub fn apply_tool_annotations(&self, tools: &[rmcp::model::Tool]) {
        if let Some(inspector) = self.get_permission_inspector() {
            inspector.apply_tool_annotations(tools);
        }
    }

    pub async fn update_permission_manager(
        &self,
        tool_name: &str,
        permission_level: crate::config::permission::PermissionLevel,
    ) {
        if let Some(inspector) = self.get_permission_inspector() {
            inspector
                .permission_manager
                .update_user_permission(tool_name, permission_level);
        }
    }

    pub fn process_inspection_results_with_permission_inspector(
        &self,
        remaining_requests: &[ToolRequest],
        inspection_results: &[InspectionResult],
    ) -> Option<PermissionCheckResult> {
        self.get_permission_inspector().map(|inspector| {
            inspector.process_inspection_results(remaining_requests, inspection_results)
        })
    }
}

impl Default for ToolInspectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply inspection results to permission check results
/// This is the generic permission-mixing logic that works for all inspector types
pub fn apply_inspection_results_to_permissions(
    mut permission_result: PermissionCheckResult,
    inspection_results: &[InspectionResult],
) -> PermissionCheckResult {
    if inspection_results.is_empty() {
        return permission_result;
    }

    // Create a map of tool requests by ID for easy lookup
    let mut all_requests: HashMap<String, ToolRequest> = HashMap::new();

    // Collect all tool requests
    for req in &permission_result.approved {
        all_requests.insert(req.id.clone(), req.clone());
    }
    for req in &permission_result.needs_approval {
        all_requests.insert(req.id.clone(), req.clone());
    }
    for req in &permission_result.denied {
        all_requests.insert(req.id.clone(), req.clone());
    }

    // Process inspection results
    for result in inspection_results {
        let request_id = &result.tool_request_id;

        tracing::info!(
            inspector_name = result.inspector_name,
            tool_request_id = %request_id,
            action = ?result.action,
            confidence = result.confidence,
            reason = %result.reason,
            finding_id = ?result.finding_id,
            "Applying inspection result"
        );

        match result.action {
            InspectionAction::Deny => {
                // Remove from approved and needs_approval, add to denied
                permission_result
                    .approved
                    .retain(|req| req.id != *request_id);
                permission_result
                    .needs_approval
                    .retain(|req| req.id != *request_id);

                if let Some(request) = all_requests.get(request_id) {
                    if !permission_result
                        .denied
                        .iter()
                        .any(|req| req.id == *request_id)
                    {
                        permission_result.denied.push(request.clone());
                    }
                }
            }
            InspectionAction::RequireApproval(_) => {
                // Remove from approved, add to needs_approval if not already there
                permission_result
                    .approved
                    .retain(|req| req.id != *request_id);

                if let Some(request) = all_requests.get(request_id) {
                    if !permission_result
                        .needs_approval
                        .iter()
                        .any(|req| req.id == *request_id)
                    {
                        permission_result.needs_approval.push(request.clone());
                    }
                }
            }
            InspectionAction::Allow => {
                // This inspector allows it, but don't override other inspectors' decisions
                // If it's already denied or needs approval, leave it that way
            }
        }
    }

    permission_result
}

pub fn get_security_finding_id_from_results(
    tool_request_id: &str,
    inspection_results: &[InspectionResult],
) -> Option<String> {
    inspection_results
        .iter()
        .find(|result| {
            result.tool_request_id == tool_request_id && result.inspector_name == "security"
        })
        .and_then(|result| result.finding_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::ToolRequest;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;

    #[test]
    fn test_apply_inspection_results() {
        let tool_request = ToolRequest {
            id: "req_1".to_string(),
            tool_call: Ok(CallToolRequestParams::new("test_tool").with_arguments(object!({}))),
            metadata: None,
            tool_meta: None,
        };

        let permission_result = PermissionCheckResult {
            approved: vec![tool_request.clone()],
            needs_approval: vec![],
            denied: vec![],
        };

        let inspection_results = vec![InspectionResult {
            tool_request_id: "req_1".to_string(),
            action: InspectionAction::Deny,
            reason: "Test denial".to_string(),
            confidence: 0.9,
            inspector_name: "test_inspector".to_string(),
            finding_id: Some("TEST-001".to_string()),
        }];

        let updated_result =
            apply_inspection_results_to_permissions(permission_result, &inspection_results);

        assert_eq!(updated_result.approved.len(), 0);
        assert_eq!(updated_result.denied.len(), 1);
        assert_eq!(updated_result.denied[0].id, "req_1");
    }

    // ── D34: a disable cannot be silent ──

    /// An inspector that reports itself off, standing in for a write jail or
    /// security scanner whose break-glass switch is set.
    struct DisabledInspector(&'static str);

    #[async_trait]
    impl ToolInspector for DisabledInspector {
        fn name(&self) -> &'static str {
            self.0
        }
        fn is_enabled(&self) -> bool {
            false
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        async fn inspect(
            &self,
            _session_id: &str,
            _tool_requests: &[ToolRequest],
            _messages: &[Message],
            _goose_mode: GooseMode,
        ) -> Result<Vec<InspectionResult>> {
            panic!("a disabled inspector must never be asked to inspect");
        }
    }

    /// Capture everything a block logs, so "it warns" is asserted rather than
    /// asserted-about.
    fn captured_logs(f: impl FnOnce()) -> String {
        use std::sync::{Arc, Mutex};
        #[derive(Clone)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let sink = Sink(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || sink.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buf.lock().unwrap().clone();
        String::from_utf8_lossy(&bytes).to_string()
    }

    #[test]
    fn a_skipped_safety_inspector_warns_with_the_marker_on_every_call() {
        let mut manager = ToolInspectionManager::new();
        manager.add_inspector(Box::new(DisabledInspector("write_jail")));
        let requests = vec![ToolRequest {
            id: "req_1".to_string(),
            tool_call: Ok(CallToolRequestParams::new("shell").with_arguments(object!({}))),
            metadata: None,
            tool_meta: None,
        }];

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let logs = captured_logs(|| {
            for _ in 0..2 {
                rt.block_on(manager.inspect_tools("sess-1", &requests, &[], GooseMode::Auto))
                    .unwrap();
            }
        });

        assert_eq!(
            logs.matches(SAFETY_DISABLE_MARKER).count(),
            2,
            "every affected call must carry the marker, not just the first: {logs}"
        );
        assert!(logs.contains("WARN"), "must be WARN, not debug: {logs}");
        assert!(
            logs.contains("SECURITY_WRITE_JAIL_ENABLED"),
            "the WARN must name the switch that turned it off: {logs}"
        );
    }

    #[test]
    fn an_unconfigured_optional_inspector_is_not_an_alarm() {
        // The adversary reviewer ships off (no adversary.md). Nothing was
        // switched off, so it must not raise the break-glass marker.
        let mut manager = ToolInspectionManager::new();
        manager.add_inspector(Box::new(DisabledInspector("adversary")));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let logs = captured_logs(|| {
            rt.block_on(manager.inspect_tools("sess-1", &[], &[], GooseMode::Auto))
                .unwrap();
        });
        assert!(
            !logs.contains(SAFETY_DISABLE_MARKER),
            "an opt-in inspector that was never configured is not a disable: {logs}"
        );
        assert!(safety_switch_for("adversary").is_none());
    }

    #[test]
    fn every_break_glass_switch_names_a_real_inspector_and_what_it_costs() {
        assert!(!SAFETY_SWITCHES.is_empty());
        for s in SAFETY_SWITCHES {
            assert!(!s.inspector.is_empty());
            assert_eq!(
                s.switch,
                s.switch.to_uppercase(),
                "Config::get_param reads the UPPERCASED env var first"
            );
            assert!(!s.loses.is_empty(), "{} must say what is lost", s.switch);
            assert!(safety_switch_for(s.inspector).is_some());
        }
    }

    #[test]
    fn an_active_break_glass_switch_is_reported_to_doctor() {
        let _env = env_lock::lock_env([("SECURITY_WRITE_JAIL_ENABLED", Some("false"))]);
        let active = active_safety_disables();
        let jail = active
            .iter()
            .find(|d| d.switch == "SECURITY_WRITE_JAIL_ENABLED")
            .expect("a disabled write jail must surface: {active:?}");
        assert_eq!(jail.inspector, "write_jail");
        assert_eq!(jail.value, "false");
        assert!(!jail.loses.is_empty());
    }

    #[test]
    fn tool_argument_validation_off_is_reported_to_doctor() {
        let _env = env_lock::lock_env([("GOOSE_TOOL_ARG_VALIDATION", Some("off"))]);
        assert!(
            active_safety_disables()
                .iter()
                .any(|d| d.switch == "GOOSE_TOOL_ARG_VALIDATION"),
            "an off argument validator is a disabled safety inspector"
        );
    }
}
