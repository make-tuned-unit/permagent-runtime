pub mod format;
pub mod graph;
pub mod languages;
pub mod parser;
pub mod repo_map;

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::tool_execution::ToolCallContext;
use anyhow::Result;
use async_trait::async_trait;
use ignore::WalkBuilder;
use indoc::indoc;
use parser::{FileAnalysis, Parser};
use rayon::prelude::*;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "analyze";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeParams {
    /// File or directory path to analyze
    pub path: String,
    /// Symbol name to focus on (triggers call graph mode)
    #[serde(default)]
    pub focus: Option<String>,
    /// Directory recursion depth limit (default 3, 0=unlimited). Also limits focus scan depth.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Call graph traversal depth (default 2, 0=definitions only)
    #[serde(default = "default_follow_depth")]
    pub follow_depth: u32,
    /// Allow large outputs without size warning
    #[serde(default)]
    pub force: bool,
}

fn default_max_depth() -> u32 {
    3
}
fn default_follow_depth() -> u32 {
    2
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MapQueryParams {
    /// Term to find in the project's stored code map: a file/dir/symbol name
    /// or a phrase; matching is case-insensitive and trailing-plural tolerant,
    /// and snake/kebab identifiers match via their parts.
    pub term: String,
    /// Project ID (UUID) or slug. If omitted, the project whose root_path
    /// contains the session's working directory is used.
    #[serde(default)]
    pub project_id_or_slug: Option<String>,
}

pub struct AnalyzeClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl AnalyzeClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(EXTENSION_NAME, "1.0.0").with_title("Analyze"))
            .with_instructions(indoc! {"
            Analyze code structure using tree-sitter AST parsing. Three auto-selected modes:
            - Directory path → structure overview (file tree with function/class counts)
            - File path → semantic details (functions, classes, imports, call counts)
            - Any path + focus parameter → symbol call graph (incoming/outgoing chains)

            For large codebases, delegate analysis to a subagent and retain only the summary.

            map_query answers \"where does X live?\" from the project's STORED code map (indexed
            via POST /api/projects/{id}/index-code) without touching the filesystem — prefer it
            over ls/grep exploration when a code map exists.
        "});

        Ok(Self { info, context })
    }

    fn schema<T: JsonSchema>() -> JsonObject {
        serde_json::to_value(schema_for!(T))
            .expect("schema serialization should succeed")
            .as_object()
            .expect("schema should serialize to an object")
            .clone()
    }

    fn parse_args<T: serde::de::DeserializeOwned>(
        arguments: Option<JsonObject>,
    ) -> Result<T, String> {
        let value = arguments
            .map(Value::Object)
            .ok_or_else(|| "Missing arguments".to_string())?;
        serde_json::from_value(value).map_err(|e| format!("Failed to parse arguments: {e}"))
    }

    fn resolve_path(path: &str, working_dir: Option<&Path>) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else if let Some(cwd) = working_dir {
            cwd.join(p)
        } else {
            p
        }
    }

    fn analyze(&self, params: AnalyzeParams, path: PathBuf) -> CallToolResult {
        if !path.exists() {
            return CallToolResult::error(vec![Content::text(format!(
                "Error: path not found: {}",
                path.display()
            ))
            .with_priority(0.0)]);
        }

        if let Some(ref focus) = params.focus {
            self.focused_mode(
                &path,
                focus,
                params.follow_depth,
                params.max_depth,
                params.force,
            )
        } else if path.is_file() {
            self.semantic_mode(&path, params.force)
        } else {
            self.structure_mode(&path, params.max_depth, params.force)
        }
    }

    pub fn analyze_file(path: &Path) -> Option<FileAnalysis> {
        let source = std::fs::read_to_string(path).ok()?;
        let parser = Parser::new();
        parser.analyze_file(path, &source)
    }

    pub fn collect_files(dir: &Path, max_depth: u32) -> Vec<PathBuf> {
        let mut builder = WalkBuilder::new(dir);
        if max_depth > 0 {
            builder.max_depth(Some(max_depth as usize));
        }
        builder
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
            .map(|e| e.into_path())
            .collect()
    }

    fn structure_mode(&self, dir: &Path, max_depth: u32, force: bool) -> CallToolResult {
        let files = Self::collect_files(dir, max_depth);
        let total_files = files.len();

        let analyses: Vec<FileAnalysis> = files
            .par_iter()
            .filter_map(|f| Self::analyze_file(f))
            .collect();

        let output = format::format_structure(&analyses, dir, max_depth, total_files);
        Self::finish(output, force)
    }

    fn semantic_mode(&self, path: &Path, force: bool) -> CallToolResult {
        match Self::analyze_file(path) {
            Some(analysis) => {
                let root = path.parent().unwrap_or(path);
                let output = format::format_semantic(&analysis, root);
                Self::finish(output, force)
            }
            None => CallToolResult::error(vec![Content::text(format!(
                "Error: could not analyze {} (unsupported language or binary file)",
                path.display()
            ))
            .with_priority(0.0)]),
        }
    }

    fn focused_mode(
        &self,
        path: &Path,
        symbol: &str,
        follow_depth: u32,
        max_depth: u32,
        force: bool,
    ) -> CallToolResult {
        let files = if path.is_file() {
            vec![path.to_path_buf()]
        } else {
            Self::collect_files(path, max_depth)
        };

        let analyses: Vec<FileAnalysis> = files
            .par_iter()
            .filter_map(|f| Self::analyze_file(f))
            .collect();

        let root = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let g = graph::CallGraph::build(&analyses);
        let output = format::format_focused(symbol, &g, follow_depth, analyses.len(), root);
        Self::finish(output, force)
    }

    fn finish(output: String, force: bool) -> CallToolResult {
        match format::check_size(&output, force) {
            Ok(text) => CallToolResult::success(vec![Content::text(text).with_priority(0.0)]),
            Err(warning) => CallToolResult::error(vec![Content::text(warning).with_priority(0.0)]),
        }
    }

    /// The `map_query` tool: slice the project's STORED code map
    /// (`code:{project_id}:map`, written by `POST /api/projects/{id}/index-code`)
    /// around a term. Added after the goals A/B measurement (2026-08-10) showed
    /// that INJECTING a map into worker prompts does not change navigation
    /// behaviour — workers grep anyway. A tool the worker calls at the moment
    /// it wonders "where does X live?" puts the map on the path it actually
    /// takes. The matching + ancestry + budget logic is shared with the
    /// orchestrator's dispatch-time injection ([`super::code_map`]).
    async fn map_query(
        &self,
        params: MapQueryParams,
        working_dir: Option<&Path>,
    ) -> CallToolResult {
        match self.map_query_inner(params, working_dir).await {
            Ok(text) => CallToolResult::success(vec![Content::text(text).with_priority(0.0)]),
            Err(error) => CallToolResult::error(vec![
                Content::text(format!("Error: {error}")).with_priority(0.0)
            ]),
        }
    }

    async fn map_query_inner(
        &self,
        params: MapQueryParams,
        working_dir: Option<&Path>,
    ) -> std::result::Result<String, String> {
        let pool = self
            .context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())?;
        let project = match params
            .project_id_or_slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(id_or_slug) => crate::projects::get_project_by_id_or_slug(&pool, id_or_slug)
                .await?
                .ok_or_else(|| format!("Project '{id_or_slug}' not found"))?,
            None => {
                Self::project_for_working_dir(&pool, working_dir, self.context.session.as_deref())
                    .await?
            }
        };
        let map = match super::get_global_brain() {
            Some(brain) => brain
                .get_memory_by_key(&format!("code:{}:map", project.id))
                .await
                .ok()
                .flatten()
                .map(|m| m.content),
            None => None,
        };
        let label = format!("\"{}\" (slug: {})", project.name, project.slug);
        Ok(super::code_map::render_map_query(
            map.as_deref(),
            &params.term,
            &label,
            &project.id,
        ))
    }

    /// Resolve the session's project by containment, mirroring how sibling
    /// extensions bind a tool call to "the project I'm working in": the project
    /// whose `root_path` contains the call's working directory (falling back to
    /// the session's). Deepest root wins when project roots nest. Explicit,
    /// never silent — no match is an error naming the directory, not a guess.
    async fn project_for_working_dir(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        working_dir: Option<&Path>,
        session: Option<&crate::session::Session>,
    ) -> std::result::Result<crate::projects::Project, String> {
        let dir = working_dir
            .map(Path::to_path_buf)
            .or_else(|| session.map(|s| s.working_dir.clone()))
            .ok_or_else(|| {
                "No project_id_or_slug given and this session has no working directory — \
                 pass the project explicitly."
                    .to_string()
            })?;
        let dir_str = dir.to_string_lossy();
        let mut candidates: Vec<crate::projects::Project> =
            crate::projects::list_projects(pool, None)
                .await?
                .into_iter()
                .filter(|p| {
                    p.root_path.as_deref().is_some_and(|root| {
                        let root = root.trim_end_matches('/');
                        !root.is_empty()
                            && (dir_str == root || dir_str.starts_with(&format!("{root}/")))
                    })
                })
                .collect();
        candidates
            .sort_by_key(|p| std::cmp::Reverse(p.root_path.as_deref().map(str::len).unwrap_or(0)));
        candidates.into_iter().next().ok_or_else(|| {
            format!(
                "No project's root_path contains the working directory {dir_str} — \
                 pass project_id_or_slug explicitly."
            )
        })
    }
}

/// A persisted **code map**: the rendered directory/symbol overview plus the
/// count of files that parsed into it — what a caller reports as "indexed N
/// files".
pub struct CodeMap {
    /// Rendered map text — byte-identical to the `analyze` tool's directory
    /// (structure) mode for the same root and depth.
    pub text: String,
    /// Files that parsed into the map (matches the map header's own count).
    pub files: usize,
}

/// Build a project **code map** for durable persistence: the same
/// directory-structure overview the `analyze` tool renders for a directory (a
/// file tree with per-file LOC / function / class digests), returned as a plain
/// `String` instead of streamed to a transcript.
///
/// Reuses the tool's own collect → parallel-parse → format pipeline (identical
/// to the private `structure_mode`) so the stored map is byte-identical to what
/// the agent sees interactively. `max_depth` follows the tool's semantics
/// (0 = unlimited); [`ignore::WalkBuilder`] honors `.gitignore` / hidden-file
/// rules, so build artifacts (`node_modules`, `target`, `.git`, …) are excluded
/// without extra config. The tool's transcript size guard
/// ([`format::check_size`]) is intentionally *not* applied — a persisted memory
/// is not a transcript, and the caller owns any truncation policy.
pub fn build_code_map(root: &Path, max_depth: u32) -> CodeMap {
    let files = AnalyzeClient::collect_files(root, max_depth);
    let total_files = files.len();
    let analyses: Vec<FileAnalysis> = files
        .par_iter()
        .filter_map(|f| AnalyzeClient::analyze_file(f))
        .collect();
    let text = format::format_structure(&analyses, root, max_depth, total_files);
    CodeMap {
        files: analyses.len(),
        text,
    }
}

/// Self-knowledge descriptor for the **codebase index** surface (#471): a
/// project's code can be parsed into a durable, project-scoped code map in the
/// Brain, then recalled and described like its documents. Co-located with
/// [`build_code_map`] (the pass that produces the persisted map); aggregated by
/// `crate::agents::self_knowledge::SURFACE_DESCRIPTORS`. Static — the capability
/// is described without claiming a live per-project index status.
pub const CODEBASE_INDEX_FEATURE: crate::agents::self_knowledge::FeatureDescriptor =
    crate::agents::self_knowledge::FeatureDescriptor {
        id: "codebase",
        display_name: "Codebase Index",
        category: crate::agents::self_knowledge::FeatureCategory::Surface,
        what_it_does:
            "A project's codebase can be indexed into your Brain — the analyze extension's tree-sitter pass renders a durable code map of its directory structure and per-file symbols, stored and scoped to that project exactly as dropped documents and written notes are",
        why_it_matters:
            "It makes a codebase a first-class thing you remember and recall — not a transcript you parse once and forget; once a project is indexed you can recall how its code is shaped and what its symbols are without re-reading every file, and the Librarian describes the map just as it describes documents",
        state_source: crate::agents::self_knowledge::StateSource::Static,
        // Onboarding lesson (the standing rule: every user-facing capability carries
        // teaching steps, not just a descriptor). A Static surface that writes to
        // the Brain confirms by the MemoryRecallable proxy — the sanctioned
        // read-back when there is no live status to poll (mirrors the Reader).
        teaching: &[
            crate::agents::self_knowledge::TeachingStep {
                title: "Open a project's codebase",
                body: "Take them to Projects, open a project that has a code folder, and point out the Codebase panel on its Overview — where its code can be indexed into your Brain the way its documents and notes already are.",
                open_surface: Some(crate::agents::self_knowledge::SurfaceRef {
                    tab: "Projects",
                    section: None,
                }),
                confirm: None,
            },
            crate::agents::self_knowledge::TeachingStep {
                title: "Index it, then ask about the code",
                body: "Offer to index the project's code for them, then prove it landed: have them ask you about the codebase — its shape, where something lives, what its main pieces are — and answer from the code map you just stored, not by re-reading files.",
                open_surface: None,
                confirm: Some(crate::agents::self_knowledge::ConfirmCheck::MemoryRecallable(
                    "this project's code structure and symbols — the code map you just indexed",
                )),
            },
        ],
    };

impl AnalyzeClient {
    /// The full, static tool inventory. Extracted from `list_tools` so the
    /// self-knowledge completeness guard derives its inventory from the REAL
    /// list — add a tool here and CI fails until the registry `description`
    /// names it.
    pub(crate) fn get_tools() -> Vec<Tool> {
        vec![
            Tool::new(
                "analyze".to_string(),
                "Analyze code structure in 3 modes: 1) Directory overview - file tree with LOC/function/class counts to max_depth. 2) File details - functions, classes, imports. 3) Symbol focus - call graphs across directory to max_depth (requires file or directory path, case-sensitive). Typical flow: directory → files → symbols. Functions called >3x show •N.".to_string(),
                Self::schema::<AnalyzeParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Analyze".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "map_query".to_string(),
                "Find where something lives using the project's stored code map (indexed by POST /api/projects/{id}/index-code) — no filesystem access. Returns only the map lines matching the term (case-insensitive, tolerates a trailing plural) together with their ancestor directories, so every hit is a navigable path. Prefer this over ls/grep exploration when the project is indexed; if the project argument is omitted, the session's working directory selects it.".to_string(),
                Self::schema::<MapQueryParams>(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Map Query".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
        ]
    }
}

#[async_trait]
impl McpClientTrait for AnalyzeClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let working_dir = ctx.working_dir.as_deref();
        match name {
            "analyze" => match Self::parse_args::<AnalyzeParams>(arguments) {
                Ok(params) => {
                    let path = Self::resolve_path(&params.path, working_dir);
                    Ok(self.analyze(params, path))
                }
                Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {error}"
                ))
                .with_priority(0.0)])),
            },
            "map_query" => match Self::parse_args::<MapQueryParams>(arguments) {
                Ok(params) => Ok(self.map_query(params, working_dir).await),
                Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Error: {error}"
                ))
                .with_priority(0.0)])),
            },
            _ => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: Unknown tool: {name}"
            ))
            .with_priority(0.0)])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionManager;
    use rmcp::model::RawContent;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn ctx() -> PlatformExtensionContext {
        PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(SessionManager::new(std::env::temp_dir())),
            session: None,
        }
    }

    fn text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(t) => &t.text,
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn structure_mode() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("lib.rs"),
            "use std::io;\nfn read() {}\nfn write() {}\nstruct Buffer;\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("app.py"),
            "import os\nclass App:\n    pass\ndef main():\n    pass\ndef run():\n    pass\n",
        )
        .unwrap();

        let client = AnalyzeClient::new(ctx()).unwrap();
        let result = client.analyze(
            AnalyzeParams {
                path: tmp.path().to_str().unwrap().into(),
                focus: None,
                max_depth: 3,
                follow_depth: 2,
                force: false,
            },
            tmp.path().to_path_buf(),
        );
        let out = text(&result);

        assert!(out.contains("2 files"));
        assert!(out.contains("F"));
        assert!(out.contains("lib.rs"));
        assert!(out.contains("app.py"));
        assert!(out.contains("rust"));
        assert!(out.contains("python"));
    }

    #[tokio::test]
    async fn semantic_mode() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("demo.rs");
        fs::write(
            &file,
            r#"
use std::collections::HashMap;
use std::io;

struct Config;

fn validate(x: i32) -> bool { x > 0 }
fn process() {
    validate(1);
    validate(2);
    validate(3);
    validate(4);
    helper();
}
fn helper() { validate(0); }
"#,
        )
        .unwrap();

        let client = AnalyzeClient::new(ctx()).unwrap();
        let result = client.analyze(
            AnalyzeParams {
                path: file.to_str().unwrap().into(),
                focus: None,
                max_depth: 3,
                follow_depth: 2,
                force: false,
            },
            file.clone(),
        );
        let out = text(&result);

        // Functions listed with signatures and line numbers
        assert!(out.contains("F:"));
        assert!(out.contains("validate("));
        assert!(out.contains("process:"));
        assert!(out.contains("helper"));
        // Struct
        assert!(out.contains("C:"));
        assert!(out.contains("Config:"));
        // Imports
        assert!(out.contains("I:"));
        assert!(out.contains("std::collections::HashMap"));
        // validate called 5 times (>3) → •5
        assert!(out.contains("validate(") && out.contains("•5"));
    }

    #[tokio::test]
    async fn focused_mode() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("a.rs"), "fn process() { validate(1); }\n").unwrap();
        fs::write(tmp.path().join("b.rs"), "fn validate() { process(); }\n").unwrap();

        let client = AnalyzeClient::new(ctx()).unwrap();
        let result = client.analyze(
            AnalyzeParams {
                path: tmp.path().to_str().unwrap().into(),
                focus: Some("process".into()),
                max_depth: 3,
                follow_depth: 2,
                force: false,
            },
            tmp.path().to_path_buf(),
        );
        let out = text(&result);

        assert!(out.contains("FOCUS: process"));
        assert!(out.contains("DEF"));
        assert!(out.contains("IN") || out.contains("OUT"));
        assert!(out.contains("files analyzed"));
    }

    #[tokio::test]
    async fn error_and_edge() {
        let client = AnalyzeClient::new(ctx()).unwrap();

        // Nonexistent path
        let result = client.analyze(
            AnalyzeParams {
                path: "/no/such/path".into(),
                focus: None,
                max_depth: 3,
                follow_depth: 2,
                force: false,
            },
            PathBuf::from("/no/such/path"),
        );
        assert_eq!(result.is_error, Some(true));
        assert!(text(&result).contains("path not found"));

        // Empty directory → 0 files
        let tmp = tempdir().unwrap();
        let result = client.analyze(
            AnalyzeParams {
                path: tmp.path().to_str().unwrap().into(),
                focus: None,
                max_depth: 3,
                follow_depth: 2,
                force: false,
            },
            tmp.path().to_path_buf(),
        );
        assert!(text(&result).contains("0 files"));

        // Size guard
        let big = "x".repeat(60_000);
        assert!(format::check_size(&big, false).is_err());
        assert!(format::check_size(&big, true).is_ok());
    }

    #[test]
    fn build_code_map_reuses_structure_pass() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("lib.rs"),
            "use std::io;\nfn read() {}\nfn write() {}\nstruct Buffer;\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("app.py"),
            "import os\nclass App:\n    pass\ndef main():\n    pass\n",
        )
        .unwrap();

        let map = build_code_map(tmp.path(), 0);
        // Both supported files parsed into the map.
        assert_eq!(map.files, 2);
        // The persisted text is the same structure overview the tool renders to
        // a transcript — so a stored code map reads identically to a live one.
        assert!(map.text.contains("2 files"));
        assert!(map.text.contains("lib.rs"));
        assert!(map.text.contains("app.py"));
    }

    /// The tool must actually ship: listed with its documented parameters, so
    /// the self-knowledge completeness guard holds it to the naming contract.
    #[test]
    fn map_query_tool_is_listed_with_its_params() {
        let tools = AnalyzeClient::get_tools();
        let map_query = tools
            .iter()
            .find(|t| t.name == "map_query")
            .expect("map_query must be in the analyze tool inventory");
        let props = map_query
            .input_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("map_query schema has properties");
        assert!(props.contains_key("term"));
        assert!(props.contains_key("project_id_or_slug"));
    }

    /// `project_id_or_slug` is optional on the wire: a bare `{term}` call is
    /// valid and resolves the project from the session's working directory.
    #[test]
    fn map_query_params_parse_with_term_only() {
        let params: MapQueryParams = AnalyzeClient::parse_args(Some(
            serde_json::json!({"term": "receipts"})
                .as_object()
                .unwrap()
                .clone(),
        ))
        .unwrap();
        assert_eq!(params.term, "receipts");
        assert!(params.project_id_or_slug.is_none());
    }
}
