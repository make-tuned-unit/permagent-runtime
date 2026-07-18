//! Write jail (C3): file writes/edits OUTSIDE the session's working directory
//! require user confirmation — routed through the same Decision-Inbox seam as
//! every other tool approval (#760), so a jailed write is an answerable card,
//! never a silent refusal or a hang.
//!
//! Confirmation, NOT a hard wall: the coding harness legitimately writes
//! absolute paths (its own worktrees, temp scratch), so the jail ALLOWS
//! without asking:
//!   - anything under the session's working directory (relative paths resolve
//!     there — the same `resolve_path` the developer tools use);
//!   - temp directories (`std::env::temp_dir()`, `/tmp`, `/var/folders`);
//!   - the goal-engine worktree namespace (any path containing a
//!     `.permagent-goal-worktrees` component — worker sessions also run WITH
//!     their worktree as working dir, so this mainly covers the supervising
//!     session reaching into a worker's tree);
//!   - paths the user names in config (`SECURITY_WRITE_JAIL_ALLOW`: a YAML
//!     list or a comma-separated string of prefixes).
//!
//! `SECURITY_WRITE_JAIL_ENABLED: false` turns the jail off entirely (explicit
//! config over hidden defaults). If the session's working directory cannot be
//! resolved, the jail FAILS OPEN (allows, with a warning log): a jail that
//! cannot see its own boundary must not take the agent down with it.

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::config::{Config, GooseMode};
use crate::conversation::message::{Message, ToolRequest};
use crate::session::SessionManager;
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};

pub const WRITE_JAIL_INSPECTOR_NAME: &str = "write_jail";

/// Directory component that marks the goal-engine worktree namespace (see
/// `goal_engine::GOAL_WORKTREES_DIR`).
const GOAL_WORKTREES_COMPONENT: &str = ".permagent-goal-worktrees";

pub struct WriteJailInspector {
    session_manager: Option<Arc<SessionManager>>,
}

impl WriteJailInspector {
    pub fn new(session_manager: Option<Arc<SessionManager>>) -> Self {
        Self { session_manager }
    }

    async fn session_working_dir(&self, session_id: &str) -> Option<PathBuf> {
        let manager = self.session_manager.as_ref()?;
        match manager.get_session(session_id, false).await {
            Ok(session) => Some(session.working_dir),
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "write jail could not resolve the session working dir; failing open"
                );
                None
            }
        }
    }
}

fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "write" | "edit" | "text_editor" | "str_replace_editor"
    ) || name.ends_with("__write")
        || name.ends_with("__edit")
        || name.ends_with("__text_editor")
}

fn write_path(request: &ToolRequest) -> Option<String> {
    let tool_call = request.tool_call.as_ref().ok()?;
    if !is_write_tool(tool_call.name.as_ref()) {
        return None;
    }
    let args = tool_call.arguments.as_ref()?;
    args.get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Resolve `.`/`..` lexically, without touching the filesystem — so a
/// `../../..` escape is judged by where it LANDS, and a non-existent target
/// (the common case for a new file) can still be normalized.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonical form for containment checks: normalize lexically, then resolve
/// symlinks through the nearest EXISTING ancestor and re-append the rest.
/// Handles the macOS `/var` → `/private/var` symlink (temp dirs, tempdir-rooted
/// sessions) without requiring the target file to exist yet.
fn canonical_for_containment(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    let mut existing = normalized.as_path();
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if existing.exists() {
            let mut out =
                std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
            for part in remainder.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                remainder.push(name.to_os_string());
                existing = parent;
            }
            _ => return normalized,
        }
    }
}

fn has_goal_worktree_component(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == GOAL_WORKTREES_COMPONENT)
}

/// Built-in always-allowed roots: the temp directories agents and the coding
/// harness use as scratch space.
fn builtin_allowed_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        std::env::temp_dir(),
        canonical_for_containment(&std::env::temp_dir()),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/var/folders"),
        PathBuf::from("/private/var/folders"),
    ];
    roots.dedup();
    roots
}

/// User-configured allowlist: `SECURITY_WRITE_JAIL_ALLOW` as a YAML/JSON list
/// of paths, or a single comma-separated string.
fn configured_allowed_roots() -> Vec<PathBuf> {
    let config = Config::global();
    if let Ok(paths) = config.get_param::<Vec<String>>("SECURITY_WRITE_JAIL_ALLOW") {
        return paths.into_iter().map(PathBuf::from).collect();
    }
    if let Ok(joined) = config.get_param::<String>("SECURITY_WRITE_JAIL_ALLOW") {
        return joined
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    Vec::new()
}

/// The jail decision for one write target. `raw_path` is the tool argument
/// verbatim; relative paths resolve against `working_dir` exactly as the
/// developer tools resolve them at execution time.
fn jail_verdict(raw_path: &str, working_dir: &Path) -> JailVerdict {
    let resolved = crate::agents::platform_extensions::developer::edit::resolve_path(
        raw_path,
        Some(working_dir),
    );
    let candidate = canonical_for_containment(&resolved);
    let root = canonical_for_containment(working_dir);

    if candidate.starts_with(&root) {
        return JailVerdict::Inside;
    }
    if has_goal_worktree_component(&candidate) {
        return JailVerdict::Allowlisted("goal worktree namespace");
    }
    for allowed in builtin_allowed_roots() {
        if candidate.starts_with(&allowed) {
            return JailVerdict::Allowlisted("temp directory");
        }
    }
    for allowed in configured_allowed_roots() {
        if candidate.starts_with(canonical_for_containment(&allowed)) {
            return JailVerdict::Allowlisted("configured allowlist");
        }
    }
    JailVerdict::Outside {
        resolved: candidate,
    }
}

enum JailVerdict {
    Inside,
    Allowlisted(&'static str),
    Outside { resolved: PathBuf },
}

#[async_trait]
impl ToolInspector for WriteJailInspector {
    fn name(&self) -> &'static str {
        WRITE_JAIL_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn is_enabled(&self) -> bool {
        Config::global()
            .get_param::<bool>("SECURITY_WRITE_JAIL_ENABLED")
            .unwrap_or(true)
    }

    async fn inspect(
        &self,
        session_id: &str,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        _goose_mode: GooseMode,
    ) -> Result<Vec<InspectionResult>> {
        // Resolve the session's working dir once per batch — and only when the
        // batch actually contains a write, so read-only turns cost nothing.
        if !tool_requests.iter().any(|r| write_path(r).is_some()) {
            return Ok(Vec::new());
        }
        let Some(working_dir) = self.session_working_dir(session_id).await else {
            // Fail open: no boundary to enforce (see module docs).
            return Ok(Vec::new());
        };

        let mut results = Vec::new();
        for request in tool_requests {
            let Some(raw_path) = write_path(request) else {
                continue;
            };
            match jail_verdict(&raw_path, &working_dir) {
                JailVerdict::Inside => {}
                JailVerdict::Allowlisted(kind) => {
                    tracing::debug!(
                        tool_request_id = %request.id,
                        path = %raw_path,
                        allowlist = kind,
                        "write jail: allowlisted write outside the working dir"
                    );
                }
                JailVerdict::Outside { resolved } => {
                    tracing::info!(
                        monotonic_counter.goose.write_jail_confirmations = 1,
                        tool_request_id = %request.id,
                        path = %resolved.display(),
                        working_dir = %working_dir.display(),
                        "write jail: write outside the session working dir requires approval"
                    );
                    results.push(InspectionResult {
                        tool_request_id: request.id.clone(),
                        action: InspectionAction::RequireApproval(Some(format!(
                            "🔒 Write outside the working directory\n\n\
                             This session works in {} but the agent wants to write to {}. \
                             Approve if you asked for this; deny otherwise. \
                             (Standing exceptions can be added via SECURITY_WRITE_JAIL_ALLOW.)",
                            working_dir.display(),
                            resolved.display()
                        ))),
                        reason: format!(
                            "Write target {} is outside the session working directory {}",
                            resolved.display(),
                            working_dir.display()
                        ),
                        confidence: 1.0,
                        inspector_name: self.name().to_string(),
                        finding_id: None,
                    });
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionType;
    use rmcp::model::CallToolRequestParams;
    use rmcp::object;

    fn write_request(id: &str, tool: &str, path: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams::new(tool.to_string())
                .with_arguments(object!({"path": path, "content": "x"}))),
            metadata: None,
            tool_meta: None,
        }
    }

    async fn jail_with_session(tmp: &tempfile::TempDir) -> (WriteJailInspector, String, PathBuf) {
        let work = tmp.path().join("workspace");
        std::fs::create_dir_all(&work).unwrap();
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let session = session_manager
            .create_session(
                work.clone(),
                "write-jail test".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await
            .expect("create session");
        (
            WriteJailInspector::new(Some(session_manager)),
            session.id,
            work,
        )
    }

    async fn verdicts(
        jail: &WriteJailInspector,
        session_id: &str,
        requests: &[ToolRequest],
    ) -> Vec<InspectionResult> {
        jail.inspect(session_id, requests, &[], GooseMode::Auto)
            .await
            .expect("inspect")
    }

    #[tokio::test]
    async fn writes_inside_the_working_dir_pass_silently() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, work) = jail_with_session(&tmp).await;

        let requests = vec![
            write_request("rel", "write", "src/main.rs"),
            write_request("rel-dot", "edit", "./notes/a.md"),
            write_request(
                "abs",
                "write",
                work.join("deep/nested/file.rs").to_str().unwrap(),
            ),
        ];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert!(
            results.is_empty(),
            "in-jail writes must not require approval: {:?}",
            results
        );
    }

    #[tokio::test]
    async fn absolute_write_outside_requires_approval_with_answerable_message() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, work) = jail_with_session(&tmp).await;

        let requests = vec![write_request(
            "outside",
            "write",
            "/definitely/not/allowed/c3.txt",
        )];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert_eq!(results.len(), 1);
        let InspectionAction::RequireApproval(Some(message)) = &results[0].action else {
            panic!(
                "outside write must require approval: {:?}",
                results[0].action
            );
        };
        assert!(
            message.contains("/definitely/not/allowed/c3.txt"),
            "{message}"
        );
        assert!(
            message.contains(&work.display().to_string())
                || message.contains(&canonical_for_containment(&work).display().to_string()),
            "message must name the working dir: {message}"
        );
        assert_eq!(results[0].inspector_name, WRITE_JAIL_INSPECTOR_NAME);
    }

    #[tokio::test]
    async fn dot_dot_escape_is_judged_by_where_it_lands() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        // Climb far above the working dir and back down into a foreign root.
        let requests = vec![write_request(
            "escape",
            "edit",
            "../../../../../../../../definitely/not/allowed/esc.txt",
        )];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert_eq!(results.len(), 1, "a ..-escape must require approval");
        assert!(matches!(
            results[0].action,
            InspectionAction::RequireApproval(Some(_))
        ));
    }

    #[tokio::test]
    async fn temp_dirs_and_goal_worktrees_are_allowlisted() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        let temp_target = std::env::temp_dir().join("c3-jail-scratch.txt");
        let requests = vec![
            write_request("temp", "write", temp_target.to_str().unwrap()),
            write_request(
                "worktree",
                "write",
                "/repos/.permagent-goal-worktrees/run-42/src/lib.rs",
            ),
        ];
        let results = verdicts(&jail, &session_id, &requests).await;
        assert!(
            results.is_empty(),
            "temp + worktree writes are the coding harness's legitimate paths: {:?}",
            results
        );
    }

    #[tokio::test]
    async fn non_write_tools_and_missing_paths_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        let shell = ToolRequest {
            id: "shell".to_string(),
            tool_call: Ok(CallToolRequestParams::new("shell")
                .with_arguments(object!({"command": "echo hi > /definitely/not/allowed/x"}))),
            metadata: None,
            tool_meta: None,
        };
        let write_without_path = ToolRequest {
            id: "no-path".to_string(),
            tool_call: Ok(
                CallToolRequestParams::new("write").with_arguments(object!({"content": "orphan"}))
            ),
            metadata: None,
            tool_meta: None,
        };
        let results = verdicts(&jail, &session_id, &[shell, write_without_path]).await;
        assert!(results.is_empty(), "{:?}", results);
    }

    #[tokio::test]
    async fn unknown_session_fails_open() {
        let tmp = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(tmp.path().join("sessions")));
        let jail = WriteJailInspector::new(Some(session_manager));

        let requests = vec![write_request("x", "write", "/definitely/not/allowed/y.txt")];
        let results = verdicts(&jail, "no-such-session", &requests).await;
        assert!(
            results.is_empty(),
            "a jail without a boundary must fail open, not strand the agent"
        );
    }

    /// `env_lock` guards the SECURITY_WRITE_JAIL_ALLOW env config (restored on
    /// drop, panic-safe, serialized against other env-locked tests).
    #[tokio::test]
    async fn config_allowlist_extends_the_jail_in_both_formats() {
        let _env =
            env_lock::lock_env([("SECURITY_WRITE_JAIL_ALLOW", Some(r#"["/opt/c3-allowed"]"#))]);
        let tmp = tempfile::tempdir().unwrap();
        let (jail, session_id, _work) = jail_with_session(&tmp).await;

        // JSON/YAML list form.
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("a", "write", "/opt/c3-allowed/deploy/x.txt")],
        )
        .await;
        assert!(results.is_empty(), "list-form allowlist: {:?}", results);

        // Comma-separated string form (mid-test flip of the guard-listed var).
        std::env::set_var("SECURITY_WRITE_JAIL_ALLOW", "/opt/c3-a, /opt/c3-b");
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("b", "edit", "/opt/c3-b/y.txt")],
        )
        .await;
        assert!(results.is_empty(), "string-form allowlist: {:?}", results);

        // A path outside the allowlist still parks.
        let results = verdicts(
            &jail,
            &session_id,
            &[write_request("c", "write", "/opt/c3-elsewhere/z.txt")],
        )
        .await;
        assert_eq!(results.len(), 1, "non-allowlisted must still confirm");
    }

    /// `env_lock` guards the SECURITY_WRITE_JAIL_ENABLED env config.
    #[tokio::test]
    async fn jail_defaults_on_and_config_disables() {
        let _env = env_lock::lock_env([("SECURITY_WRITE_JAIL_ENABLED", None::<&str>)]);
        let jail = WriteJailInspector::new(None);
        assert!(jail.is_enabled(), "the jail must default ON");

        std::env::set_var("SECURITY_WRITE_JAIL_ENABLED", "false");
        assert!(!jail.is_enabled(), "explicit config must disable the jail");
    }

    #[test]
    fn lexical_normalize_resolves_dots_without_fs() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            lexical_normalize(Path::new("/a/../../etc/passwd")),
            PathBuf::from("/etc/passwd"),
            ".. above root clamps at root"
        );
    }

    #[test]
    fn write_tool_names_match_flat_and_prefixed() {
        for name in [
            "write",
            "edit",
            "developer__write",
            "x__edit",
            "text_editor",
        ] {
            assert!(is_write_tool(name), "{name}");
        }
        for name in ["shell", "search", "read_webpage", "rewrite", "credit"] {
            assert!(!is_write_tool(name), "{name}");
        }
    }
}
