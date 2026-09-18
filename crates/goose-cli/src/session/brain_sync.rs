//! Best-effort bridge between the standalone coding harness and the daemon's Brain.
//!
//! WHY THIS EXISTS. Chat and the harness share one Brain, but not one process.
//! The daemon owns the live `SafeBrain` handle and sets `GLOBAL_BRAIN` at boot;
//! `permagent run` is a second binary that never populates that singleton, so
//! the `search_memory` tool structurally cannot appear in its tool list and no
//! ambient recall runs before its turns. Until now the harness was write-only
//! toward the daemon — a spend ping per turn, one distilled summary at exit —
//! and read nothing back. It could not remember yesterday.
//!
//! The CLI deliberately talks to the owner over loopback instead of opening the
//! same databases in a second process: two writers of one Spectral brain is a
//! corruption story, and the daemon already exposes both halves behind the same
//! bearer token `spend_announce.rs` proves works today.
//!
//! SILENT BY CONSTRUCTION, and BOUNDED. Both operations swallow every failure —
//! no token, no daemon, refused, timed out, renamed field. A bare `permagent
//! run` in a terminal with no daemon running is a completely normal thing to
//! do, and memory availability must never turn into harness availability. The
//! read sits in the critical path before the model call, so it is capped at
//! [`BRIDGE_TIMEOUT`]; the write is detached and costs the turn nothing at all.
//!
//! SCOPED TO THIS REPO (Lane R61a). The user's ruling, 2026-09-16: "The harness
//! should only ever have access to memories about the project repo it is
//! actively in unless the user tells them to look at a different project repo
//! or memories to learn or identify patterns. giving access to memories should
//! be a decision the user approves in terminal like how Codex/Claude Code do."
//! So every recall carries a [`MemoryScope`] — this session's working directory,
//! which the daemon resolves to the project that owns it — and the daemon cuts
//! everything else, general chat memory included. Anything wider is a
//! `request_memory_access` call the USER answers at the terminal
//! ([`decide_memory_access`]); the model can only ask. A session with no
//! terminal refuses the request and stays repo-scoped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The whole budget the recall may spend, connect-to-parse.
///
/// This one is in the critical path: it runs before `agent.reply`, so it is the
/// only latency this module can add to a turn. 750ms is the ceiling a user
/// would pay on a turn where the daemon is wedged — not the common case, which
/// is a loopback query answering in single-digit milliseconds or failing to
/// connect instantly.
const BRIDGE_TIMEOUT: Duration = Duration::from_millis(750);

/// How much recalled text may reach the prompt.
const MAX_RECALL_CHARS: usize = 2_400;

/// How much of the user's turn is used as the recall query.
///
/// A pasted stack trace or file is a legitimate user turn and a terrible query
/// string: unbounded here it would be URL-encoded into a multi-kilobyte GET
/// that the daemon may refuse outright, turning a long paste into the one turn
/// with no memory. The opening of a message carries the intent.
const MAX_QUERY_CHARS: usize = 512;

/// Hits below this are noise. Matched to the parked design and to the score
/// floor Chat's own recall applies before injecting anything.
const MIN_SCORE: f64 = 0.7;

/// The system-prompt extra this block is installed under.
///
/// Deliberately the SAME key Chat's `brain_ops::inject_recall` uses. It is
/// registered in `prompt_manager::VOLATILE_EXTRA_KEYS`, which is what keeps a
/// per-turn block behind the prompt-cache breakpoint (in `# Turn-specific
/// Instructions`) instead of invalidating the cached prefix on every turn.
/// Renaming it here would silently move this block into the cached half and
/// cost a full prompt re-read per turn.
pub const RECALL_PROMPT_KEY: &str = "memory_recall";

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchResult {
    preview: String,
    score: f64,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

/// Recall response plus the redacted, machine-readable packet accounting that
/// was actually available at this bridge seam. The packet carries counts and
/// provenance, never recalled text.
struct RecallResult {
    block: String,
    spectral_memories: Vec<permagent::context_packet::SpectralMemoryRecord>,
}

/// Context returned by the bridge for one turn. The records are the same
/// filtered Spectral hits used to build `block`; callers attach them to the
/// Agent's typed prompt-extra metadata instead of recovering them from prose.
pub(crate) struct RecallContext {
    pub block: String,
    pub spectral_memories: Vec<permagent::context_packet::SpectralMemoryRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnRequest<'a> {
    session_id: &'a str,
    turn_idx: usize,
    user_text: &'a str,
    assistant_text: &'a str,
    working_dir: Option<&'a str>,
    repo_root: Option<&'a str>,
}

/// Which memories this harness session's recall may read (Lane R61a).
///
/// Session state, never config: it is built from the session's working
/// directory when the session starts, widened only by a grant the user approved
/// at the terminal, and gone when the process exits. Nothing here persists —
/// not even an "allow for this session" — because the ruling makes memory
/// access a per-decision user approval, and the tool-approval prompt's
/// "Always Allow" convention (which does persist) is deliberately not offered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MemoryScope {
    /// The directory the session works in, canonicalized so it compares
    /// against stored (canonical) project roots. `None` means the repo is
    /// unknown, and a scope with nothing else in it recalls NOTHING.
    working_dir: Option<String>,
    /// The main checkout, when `working_dir` is inside a linked git worktree.
    repo_root: Option<String>,
    /// Other projects' wing slugs the user approved for this session.
    granted_projects: BTreeSet<String>,
    /// The user approved all memories (every project and general chat memory)
    /// for this session.
    all_granted: bool,
}

impl MemoryScope {
    /// The scope of a session working in `dir`: that repo only.
    pub(crate) fn for_working_dir(dir: &Path) -> Self {
        let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let repo_root = main_checkout_root(&canonical)
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .filter(|root| *root != canonical);
        Self {
            working_dir: Some(canonical.to_string_lossy().into_owned()),
            repo_root: repo_root.map(|root| root.to_string_lossy().into_owned()),
            ..Self::default()
        }
    }

    /// A scope holding exactly one grant and not this repo: what an approved
    /// request searches, so its answer shows what the grant added.
    fn for_grant(target: &ResolvedMemoryTarget) -> Self {
        let mut scope = Self::default();
        scope.grant(target);
        scope
    }

    pub(crate) fn working_dir(&self) -> Option<&str> {
        self.working_dir.as_deref()
    }

    pub(crate) fn repo_root(&self) -> Option<&str> {
        self.repo_root.as_deref()
    }

    /// Nothing to search: no repo and no grant. Recall is skipped without a
    /// network attempt — fail closed, never open.
    fn is_empty(&self) -> bool {
        self.working_dir.is_none()
            && self.repo_root.is_none()
            && self.granted_projects.is_empty()
            && !self.all_granted
    }

    fn covers(&self, target: &ResolvedMemoryTarget) -> bool {
        match target {
            _ if self.all_granted => true,
            ResolvedMemoryTarget::All => false,
            ResolvedMemoryTarget::Project { slug, .. } => self.granted_projects.contains(slug),
        }
    }

    fn grant(&mut self, target: &ResolvedMemoryTarget) {
        match target {
            ResolvedMemoryTarget::All => self.all_granted = true,
            ResolvedMemoryTarget::Project { slug, .. } => {
                self.granted_projects.insert(slug.clone());
            }
        }
    }

    /// Whether any grant widened this scope beyond the repo.
    fn is_widened(&self) -> bool {
        self.all_granted || !self.granted_projects.is_empty()
    }
}

/// The main checkout of the git repository `dir` is in.
///
/// Walks up to the nearest `.git`. A directory means an ordinary checkout,
/// which is its own main checkout. A FILE means a linked worktree or a
/// submodule: `gitdir: <main>/.git/worktrees/<name>` resolves to `<main>`, and
/// anything else (a submodule's `.git/modules/<name>`) keeps the directory the
/// file sits in. Read straight from disk rather than by spawning `git`, so it
/// costs nothing when the session starts and cannot hang it.
fn main_checkout_root(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors() {
        let dot_git = ancestor.join(".git");
        if dot_git.is_dir() {
            return Some(ancestor.to_path_buf());
        }
        if !dot_git.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let gitdir = text
            .lines()
            .find_map(|line| line.trim().strip_prefix("gitdir:"))
            .map(str::trim)?;
        let gitdir = ancestor.join(gitdir);
        let worktrees = gitdir.parent();
        let common = worktrees
            .filter(|dir| dir.file_name().is_some_and(|name| name == "worktrees"))
            .and_then(Path::parent)
            .filter(|dir| dir.file_name().is_some_and(|name| name == ".git"));
        return Some(
            common
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| ancestor.to_path_buf()),
        );
    }
    None
}

/// The `/api/brain/search` URL for `query` inside `scope`, or `None` when the
/// scope holds nothing (or the query is empty).
///
/// `source=memory`: chat transcript rows carry no project attribution, and the
/// daemon drops them from any scoped search anyway.
fn search_url(base: &str, query: &str, scope: &MemoryScope, limit: usize) -> Option<url::Url> {
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    if query.is_empty() || scope.is_empty() {
        return None;
    }
    let mut url = url::Url::parse(&format!("{base}/api/brain/search")).ok()?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("q", &query)
            .append_pair("source", "memory")
            .append_pair("limit", &limit.to_string());
        if let Some(dir) = scope.working_dir.as_deref() {
            pairs.append_pair("scope_cwd", dir);
        }
        if let Some(root) = scope.repo_root.as_deref() {
            pairs.append_pair("scope_repo_root", root);
        }
        if !scope.granted_projects.is_empty() {
            let slugs: Vec<&str> = scope.granted_projects.iter().map(String::as_str).collect();
            pairs.append_pair("scope_projects", &slugs.join(","));
        }
        if scope.all_granted {
            pairs.append_pair("scope_all", "true");
        }
    }
    Some(url)
}

/// A resolved, authenticated way to reach the daemon.
///
/// Split out of the two public entry points so both halves of the bridge can be
/// exercised against a stub server in tests without ever reading — or needing —
/// the real `~/.permagent/secrets/daemon_token.json`.
struct Endpoint {
    client: reqwest::Client,
    base: String,
    token: String,
}

/// Resolve the live daemon, or fail fast.
///
/// Missing token file is the no-daemon-has-ever-run case and returns here
/// without a network attempt, so the common "just running the CLI" path pays
/// nothing at all.
fn endpoint() -> anyhow::Result<Endpoint> {
    let token = crate::commands::daemon::load_daemon_token()?;
    let port = crate::commands::daemon::read_daemon_port();
    Ok(Endpoint {
        client: http_client()?,
        base: format!("http://127.0.0.1:{port}"),
        token,
    })
}

/// The one place the budget is applied.
///
/// Factored out so the tests that assert the bound build their stub endpoint
/// through the SAME constructor. A test that set its own timeout would prove
/// only that reqwest honours a timeout, and would stay green if this module
/// stopped setting one.
fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(BRIDGE_TIMEOUT).build()
}

/// What this turn should install under [`RECALL_PROMPT_KEY`], if anything.
///
/// Three-valued on purpose, and the third value is the one that matters:
/// - `Some(block)` — hits worth injecting.
/// - `Some(String::new())` — no hits, but the PREVIOUS turn injected some.
///   The extras map is keyed and persists across turns, so leaving turn 1's
///   memories in place while the user asks about something else on turn 9 is
///   exactly the contamination this bridge must not introduce. Overwriting with
///   an empty string clears it.
/// - `None` — no hits and nothing stale to clear. Write nothing, so a session
///   that never recalls anything never grows an empty header.
pub(crate) async fn recall_context(query: &str, scope: &MemoryScope) -> Option<RecallContext> {
    if scope.is_empty() {
        return None;
    }
    let Ok(ep) = endpoint() else {
        return None;
    };
    recall_at_with_packet(&ep, query, scope)
        .await
        .map(|result| RecallContext {
            block: result.block,
            spectral_memories: result.spectral_memories,
        })
}

#[cfg(test)]
async fn recall_block_at(
    ep: &Endpoint,
    query: &str,
    scope: &MemoryScope,
    previously_installed: bool,
) -> Option<String> {
    match (
        recall_at_with_packet(ep, query, scope).await,
        previously_installed,
    ) {
        (Some(result), _) => Some(result.block),
        (None, true) => Some(String::new()),
        (None, false) => None,
    }
}

#[cfg(test)]
async fn recall_at(ep: &Endpoint, query: &str, scope: &MemoryScope) -> Option<String> {
    recall_at_with_packet(ep, query, scope)
        .await
        .map(|result| result.block)
}

async fn search_at(
    ep: &Endpoint,
    query: &str,
    scope: &MemoryScope,
    limit: usize,
) -> Option<Vec<SearchResult>> {
    let url = search_url(&ep.base, query, scope, limit)?;
    let response = ep
        .client
        .get(url)
        .bearer_auth(&ep.token)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<SearchResponse>()
        .await
        .ok()?;
    Some(response.results)
}

async fn recall_at_with_packet(
    ep: &Endpoint,
    query: &str,
    scope: &MemoryScope,
) -> Option<RecallResult> {
    let results = search_at(ep, query, scope, 3).await?;
    let header = if scope.is_widened() {
        RECALL_HEADER_WIDENED
    } else {
        RECALL_HEADER
    };
    let (block, spectral_records) = render_recall_context_with_header(results, header)?;
    Some(RecallResult {
        block,
        spectral_memories: spectral_records,
    })
}

/// The recall block's header for a repo-scoped session.
const RECALL_HEADER: &str = "Shared Brain context from prior work in this repo (historical hints; verify before relying on them):\n";

/// The header once the user approved wider access for this session.
const RECALL_HEADER_WIDENED: &str = "Shared Brain context from prior work in this repo and the memory access the user approved (historical hints; verify before relying on them):\n";

/// Frame the hits as BACKGROUND, never as instructions.
///
/// A memory is a record of something that was true once, written by a different
/// session with different facts in front of it. Dropped into a prompt unlabelled
/// it reads as a directive from the user — which is how a note about last
/// month's schema turns into this turn's refactor. The header says three things
/// on purpose: where it came from, that it is historical, and that it must be
/// verified before it is relied on.
///
/// Lane R61a: the header also says WHICH memories — this repo's, or this repo's
/// plus what the user approved — so the model never mistakes a scoped recall
/// for the whole Brain.
#[cfg(test)]
fn render_recall_context(
    results: Vec<SearchResult>,
) -> Option<(String, Vec<permagent::context_packet::SpectralMemoryRecord>)> {
    render_recall_context_with_header(results, RECALL_HEADER)
}

fn render_recall_context_with_header(
    results: Vec<SearchResult>,
    header: &str,
) -> Option<(String, Vec<permagent::context_packet::SpectralMemoryRecord>)> {
    let mut out = String::from(header);
    let mut records = Vec::new();
    let mut count = 0;
    let mut seen_keys = std::collections::HashSet::new();
    for result in results
        .into_iter()
        .filter(|hit| hit.score >= MIN_SCORE)
        // The endpoint searches both chat and durable memory. Chat rows have
        // no packet slot, so exclude them rather than installing unaccounted
        // prompt text.
        .filter(|hit| {
            hit.source
                .as_deref()
                .is_none_or(|source| source == "memory")
        })
        .enumerate()
        .filter_map(|(index, mut result)| {
            if result.preview.trim().is_empty() {
                return None;
            }
            // Spectral's stable key is preferred. Test/legacy responses can
            // omit both key and id; position-scoping those records preserves
            // every rendered contribution instead of collapsing them all to
            // one `unknown` packet record.
            let key = result
                .key
                .clone()
                .or_else(|| result.id.clone())
                .unwrap_or_else(|| format!("anonymous:{index}"));
            if !seen_keys.insert(key.clone()) {
                return None;
            }
            result.key = Some(key);
            Some(result)
        })
        .take(3)
    {
        let preview = result.preview.trim();
        count += 1;
        let line = format!("- {preview}\n");
        let remaining = MAX_RECALL_CHARS.saturating_sub(out.chars().count());
        if remaining == 0 {
            break;
        }
        let rendered_line: String = line.chars().take(remaining).collect();
        let rendered = if records.is_empty() {
            format!("{header}{rendered_line}")
        } else {
            rendered_line.clone()
        };
        out.push_str(&rendered_line);
        let provenance = result
            .id
            .as_deref()
            .map(|id| format!("spectral_search_result:{id}"))
            .unwrap_or_else(|| "spectral_search".to_string());
        let key = result.key.expect("normalized recall key");
        records.push(permagent::context_packet::SpectralMemoryRecord {
            key,
            text: rendered,
            provenance: vec![provenance],
        });
        if rendered_line.chars().count() < line.chars().count() {
            break;
        }
    }
    (count > 0).then_some((out, records))
}

#[cfg(test)]
fn render_recall(results: Vec<SearchResult>) -> Option<String> {
    render_recall_context(results).map(|(block, _)| block)
}

/// Persist a completed Harness turn through the daemon that owns Brain.
///
/// Returns immediately: the request runs on a detached task, so a wedged or
/// absent daemon costs the turn nothing observable. Failures are intentionally
/// silent and never affect task completion.
pub fn persist_turn(
    session_id: String,
    turn_idx: usize,
    user_text: String,
    assistant_text: String,
    working_dir: Option<String>,
    repo_root: Option<String>,
) {
    // Nothing to remember, and the route rejects it anyway (400) — skip the
    // round trip rather than spend a task on a guaranteed refusal.
    if session_id.trim().is_empty()
        || user_text.trim().is_empty()
        || assistant_text.trim().is_empty()
    {
        return;
    }
    let Ok(ep) = endpoint() else {
        return;
    };
    spawn_persist(
        ep,
        session_id,
        turn_idx,
        user_text,
        assistant_text,
        working_dir,
        repo_root,
    );
}

fn spawn_persist(
    ep: Endpoint,
    session_id: String,
    turn_idx: usize,
    user_text: String,
    assistant_text: String,
    working_dir: Option<String>,
    repo_root: Option<String>,
) {
    tokio::spawn(async move {
        let _ = ep
            .client
            .post(format!("{}/api/coding-sessions/turn", ep.base))
            .bearer_auth(&ep.token)
            .json(&TurnRequest {
                session_id: &session_id,
                turn_idx,
                user_text: &user_text,
                assistant_text: &assistant_text,
                working_dir: working_dir.as_deref(),
                repo_root: repo_root.as_deref(),
            })
            .send()
            .await;
    });
}

// ── Memory access beyond this repo (Lane R61a) ──────────────────────────

/// The harness tool through which the model may ASK for wider memory access.
///
/// A frontend tool: the agent hands the call to this CLI and blocks until the
/// CLI answers, so the answer is whatever the user chose at the terminal —
/// nothing the model can supply in its arguments changes that.
pub(crate) const MEMORY_ACCESS_TOOL_NAME: &str = "request_memory_access";

/// The one frontend extension the coding harness registers.
pub(crate) const MEMORY_ACCESS_EXTENSION_NAME: &str = "memory_access";

/// Returned when no one can approve. Worded as a direction, so a headless run
/// continues with its own repo instead of retrying.
pub(crate) const MEMORY_ACCESS_HEADLESS_REFUSAL: &str = "Refused: memory access beyond this repo needs the user's approval in an interactive terminal, and this session has none. Continue with this repo's context.";

/// How many memories an approved request returns.
const GRANTED_SEARCH_LIMIT: usize = 5;

/// The tool definition. The model gets a scope, a reason and a query — and
/// nothing that could grant anything.
pub(crate) fn memory_access_tool() -> rmcp::model::Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "scope": {
                "type": "string",
                "enum": ["project", "all"],
                "description": "\"project\": one other registered project's memories. \"all\": every project and general chat memory."
            },
            "project": {
                "type": "string",
                "description": "Required when scope is \"project\": the project's slug, or an absolute path inside its repo."
            },
            "reason": {
                "type": "string",
                "description": "Why this task needs those memories, shown to the user verbatim."
            },
            "query": {
                "type": "string",
                "description": "What to look for in the granted memories."
            }
        },
        "required": ["scope", "reason", "query"]
    });
    let schema = match schema {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    rmcp::model::Tool::new(
        MEMORY_ACCESS_TOOL_NAME,
        "Ask the user for access to memories outside this repo. Your recalled memory is scoped to \
         the repo this session works in. Call this only when the user's request actually needs \
         another project's patterns or their general memory. The user approves or denies it in \
         the terminal; a session with no terminal refuses it. If approved, the matching memories \
         are returned.",
        schema,
    )
}

/// The frontend-extension instructions installed with the tool.
pub(crate) const MEMORY_ACCESS_INSTRUCTIONS: &str = "`request_memory_access` is answered by the user in the terminal, not by a tool server. Recall is scoped to this repo; use it only when the request needs another project's memories.";

/// What the model asked for, before it is resolved against the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryAccessTarget {
    /// A project slug or a path inside a project's repo, as the model wrote it.
    Project(String),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryAccessRequest {
    pub target: MemoryAccessTarget,
    pub reason: String,
    pub query: String,
}

/// A request's target, resolved to what the user is actually granting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedMemoryTarget {
    Project {
        slug: String,
        name: String,
        root_path: Option<String>,
    },
    All,
}

impl ResolvedMemoryTarget {
    /// How the target is named to the user and in the tool result.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::All => "all memories (every project and general chat memory)".to_string(),
            Self::Project {
                slug,
                name,
                root_path: Some(root),
            } => format!("project \"{name}\" ({slug}, {root})"),
            Self::Project { slug, name, .. } => format!("project \"{name}\" ({slug})"),
        }
    }
}

/// Read the tool call's arguments. Every failure is a message the model can act on.
pub(crate) fn parse_memory_access_request(
    arguments: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<MemoryAccessRequest, String> {
    let text = |key: &str| {
        arguments
            .and_then(|args| args.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let target = match text("scope").as_deref() {
        Some("all") => MemoryAccessTarget::All,
        Some("project") => MemoryAccessTarget::Project(text("project").ok_or_else(|| {
            "`project` is required when scope is \"project\" (a slug or a path inside its repo)"
                .to_string()
        })?),
        _ => return Err("`scope` must be \"project\" or \"all\"".to_string()),
    };
    let reason = text("reason").ok_or_else(|| "`reason` is required".to_string())?;
    let query = text("query").ok_or_else(|| "`query` is required".to_string())?;
    Ok(MemoryAccessRequest {
        target,
        reason,
        query,
    })
}

/// Resolve a requested project against the session database's registry — the
/// same registry the daemon scopes by. A path resolves to the project owning it.
pub(crate) async fn resolve_memory_target(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    target: &MemoryAccessTarget,
) -> Result<ResolvedMemoryTarget, String> {
    let requested = match target {
        MemoryAccessTarget::All => return Ok(ResolvedMemoryTarget::All),
        MemoryAccessTarget::Project(requested) => requested.as_str(),
    };
    let slug = if requested.starts_with('/') || requested.starts_with('~') {
        let expanded = match requested.strip_prefix('~') {
            Some(rest) => dirs::home_dir()
                .map(|home| format!("{}{rest}", home.display()))
                .unwrap_or_else(|| requested.to_string()),
            None => requested.to_string(),
        };
        permagent::session_wing::resolve_project_wing_for_paths(pool, &[expanded.as_str()])
            .await
            .ok_or_else(|| format!("No registered project owns the path {requested}."))?
    } else {
        requested.to_string()
    };
    if slug == permagent::session_wing::PERSONAL_SLUG {
        return Err(
            "`personal` is not a project scope. Ask for scope \"all\" if general memory is needed."
                .to_string(),
        );
    }
    match permagent::projects::get_project_by_id_or_slug(pool, &slug).await {
        Ok(Some(project)) if project.slug != permagent::session_wing::PERSONAL_SLUG => {
            Ok(ResolvedMemoryTarget::Project {
                slug: project.slug,
                name: project.name,
                root_path: project.root_path,
            })
        }
        Ok(_) => Err(format!(
            "No registered project matches \"{requested}\". Access is granted per registered project."
        )),
        Err(error) => {
            tracing::warn!(%error, "memory access: project registry read failed");
            Err("The project registry could not be read, so no access was requested.".to_string())
        }
    }
}

/// The user's answer at the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryGrantAnswer {
    AllowOnce,
    AllowForSession,
    Deny,
}

/// What the terminal prompt shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryAccessPrompt {
    pub target_label: String,
    pub reason: String,
}

/// The outcome of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryAccessDecision {
    /// No terminal to ask. Scope unchanged.
    Refused,
    /// The target is this session's own repo, which recall already covers.
    AlreadyInScope,
    /// The user said no. Scope unchanged.
    Denied,
    /// Search `search_scope` and return what it finds. `for_session` says
    /// whether the session's scope was widened too.
    Granted {
        search_scope: MemoryScope,
        for_session: bool,
    },
}

impl MemoryAccessDecision {
    /// A stable label for the diagnostic log line.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Refused => "refused_no_terminal",
            Self::AlreadyInScope => "already_in_scope",
            Self::Denied => "denied",
            Self::Granted {
                for_session: true, ..
            } => "granted_for_session",
            Self::Granted { .. } => "granted_once",
        }
    }
}

/// Decide one request. This is the whole policy, kept free of the terminal and
/// the network so each path is testable:
///
/// * `can_prompt == false` (no TTY, `permagent run` headless, an eval): refused
///   without asking anyone, scope unchanged;
/// * the session's own repo: already in scope, nobody is asked;
/// * an existing session grant covers it: granted again without asking;
/// * otherwise `prompt` asks the user. Allow once searches the target this one
///   time; allow for this session also widens `scope`; deny changes nothing.
pub(crate) fn decide_memory_access(
    scope: &mut MemoryScope,
    target: &ResolvedMemoryTarget,
    current_repo_wing: Option<&str>,
    reason: &str,
    can_prompt: bool,
    prompt: impl FnOnce(&MemoryAccessPrompt) -> MemoryGrantAnswer,
) -> MemoryAccessDecision {
    if let (ResolvedMemoryTarget::Project { slug, .. }, Some(current)) = (target, current_repo_wing)
    {
        if slug == current {
            return MemoryAccessDecision::AlreadyInScope;
        }
    }
    if scope.covers(target) {
        return MemoryAccessDecision::Granted {
            search_scope: MemoryScope::for_grant(target),
            for_session: true,
        };
    }
    if !can_prompt {
        return MemoryAccessDecision::Refused;
    }
    let answer = prompt(&MemoryAccessPrompt {
        target_label: target.label(),
        reason: reason.to_string(),
    });
    match answer {
        MemoryGrantAnswer::Deny => MemoryAccessDecision::Denied,
        MemoryGrantAnswer::AllowOnce => MemoryAccessDecision::Granted {
            search_scope: MemoryScope::for_grant(target),
            for_session: false,
        },
        MemoryGrantAnswer::AllowForSession => {
            scope.grant(target);
            MemoryAccessDecision::Granted {
                search_scope: MemoryScope::for_grant(target),
                for_session: true,
            }
        }
    }
}

/// Search a granted scope and render the tool result's memories. `None` when
/// the daemon is unreachable or nothing matched.
pub(crate) async fn search_granted(
    search_scope: &MemoryScope,
    query: &str,
    target: &ResolvedMemoryTarget,
) -> Option<String> {
    let ep = endpoint().ok()?;
    let results = search_at(&ep, query, search_scope, GRANTED_SEARCH_LIMIT).await?;
    let header = format!(
        "Memories from {} (approved by the user; historical hints; verify before relying on them):\n",
        target.label()
    );
    render_recall_context_with_header(results, &header).map(|(block, _)| block)
}

/// The tool result text for a decided request.
pub(crate) fn memory_access_result_text(
    decision: &MemoryAccessDecision,
    target: &ResolvedMemoryTarget,
    memories: Option<&str>,
) -> String {
    match decision {
        MemoryAccessDecision::Refused => MEMORY_ACCESS_HEADLESS_REFUSAL.to_string(),
        MemoryAccessDecision::AlreadyInScope => format!(
            "{} is the repo this session works in; its memories are already in your recall. No approval was needed.",
            target.label()
        ),
        MemoryAccessDecision::Denied => format!(
            "The user denied access to {}. Memory stays scoped to this repo; continue with this repo's context.",
            target.label()
        ),
        MemoryAccessDecision::Granted { for_session, .. } => {
            let grant = if *for_session {
                format!(
                    "The user approved access to {} for the rest of this session; later turns' recall includes it.",
                    target.label()
                )
            } else {
                format!(
                    "The user approved access to {} for this request only.",
                    target.label()
                )
            };
            match memories {
                Some(block) => format!("{grant}\n\n{block}"),
                None => format!("{grant} No matching memories were found (or the memory service is not reachable from this session)."),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A harness session working in a repo that does not exist on this
    /// machine: canonicalization and the `.git` walk both find nothing, so the
    /// scope is exactly the path given.
    fn repo_scope() -> MemoryScope {
        MemoryScope::for_working_dir(Path::new("/work/acme-api"))
    }

    fn ep_for(base: String) -> Endpoint {
        Endpoint {
            // Production's own constructor, on purpose — see `http_client`.
            client: http_client().unwrap(),
            base,
            token: "fixture-token-not-the-real-one".to_string(),
        }
    }

    #[test]
    fn recall_is_bounded_filtered_and_labelled_historical() {
        let text = render_recall(vec![
            SearchResult {
                preview: "current project".into(),
                score: 0.9,
                source: None,
                id: None,
                key: None,
            },
            SearchResult {
                preview: "too weak".into(),
                score: 0.2,
                source: None,
                id: None,
                key: None,
            },
        ])
        .unwrap();
        assert!(text.contains("historical hints"));
        assert!(text.contains("current project"));
        assert!(!text.contains("too weak"));
        assert!(text.chars().count() <= MAX_RECALL_CHARS);
    }

    #[test]
    fn empty_recall_is_not_injected() {
        assert!(render_recall(Vec::new()).is_none());
    }

    /// (a) When the daemon answers, the daemon's memories are what reaches the
    /// outgoing context — under the key Chat uses, carrying its own "historical
    /// hints" frame, and only after presenting the bearer token.
    #[tokio::test]
    async fn a_live_daemons_memories_reach_the_outgoing_context() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .and(query_param("source", "memory"))
            .and(query_param("scope_cwd", "/work/acme-api"))
            .and(header(
                "authorization",
                "Bearer fixture-token-not-the-real-one",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "preview": "The picker scanner runs on port 8080", "score": 0.91 },
                    { "preview": "unrelated noise", "score": 0.10 },
                ]
            })))
            .mount(&server)
            .await;

        let block = recall_block_at(
            &ep_for(server.uri()),
            "where does the scanner run?",
            &repo_scope(),
            false,
        )
        .await
        .expect("a live daemon with a strong hit must produce a block");
        assert!(block.contains("The picker scanner runs on port 8080"));
        assert!(
            block.contains("historical hints; verify before relying on them"),
            "recalled memories must be framed as background, never as instructions: {block}"
        );
        assert!(
            !block.contains("unrelated noise"),
            "a sub-threshold hit must not be injected: {block}"
        );
        assert_eq!(
            RECALL_PROMPT_KEY, "memory_recall",
            "the block must ride the same volatile key Chat uses, or it lands \
             inside the cached prompt prefix and costs a re-read every turn"
        );
    }

    #[tokio::test]
    async fn live_spectral_recall_emits_packet_counts_and_provenance() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"source": "memory", "id": "spectral:7", "key": "mem-7", "preview": "stable fact", "score": 0.95},
                    {"source": "memory", "id": "spectral:7", "key": "mem-7", "preview": "duplicate", "score": 0.94},
                    {"source": "chat", "id": "chat:9", "preview": "chat context", "score": 0.91}
                ]
            })))
            .mount(&server)
            .await;

        let result = recall_at_with_packet(&ep_for(server.uri()), "stable fact", &repo_scope())
            .await
            .expect("strong recall should produce a result");
        let packet = permagent::context_packet::assemble_owned(
            None,
            None,
            None,
            &result.spectral_memories,
            None,
            &[],
        );
        let spectral = packet
            .parts
            .iter()
            .find(|part| part.source == permagent::context_packet::ContextSource::SpectralMemory)
            .expect("fixed Spectral packet slot");
        let expected_block = render_recall(vec![SearchResult {
            preview: "stable fact".into(),
            score: 0.95,
            source: Some("memory".into()),
            id: Some("spectral:7".into()),
            key: Some("mem-7".into()),
        }])
        .unwrap();
        assert_eq!(spectral.characters, Some(expected_block.chars().count()));
        assert_eq!(
            packet.retrieval_provenance,
            vec!["spectral_search_result:spectral:7"]
        );
        assert_eq!(
            packet.estimated_total_tokens,
            Some(expected_block.chars().count().saturating_add(3) / 4)
        );
        assert!(packet
            .parts
            .iter()
            .filter(|part| part.source != permagent::context_packet::ContextSource::SpectralMemory)
            .all(|part| part.estimated_tokens.is_none()));
    }

    #[test]
    fn mixed_sources_are_filtered_before_top_k_and_truncation() {
        let results = vec![
            SearchResult {
                preview: "chat must not be installed".into(),
                score: 0.99,
                source: Some("chat".into()),
                id: Some("chat:1".into()),
                key: None,
            },
            SearchResult {
                preview: "memory one".into(),
                score: 0.98,
                source: Some("memory".into()),
                id: Some("m1".into()),
                key: Some("key-1".into()),
            },
            SearchResult {
                preview: "memory two".into(),
                score: 0.97,
                source: Some("memory".into()),
                id: Some("m2".into()),
                key: Some("key-2".into()),
            },
            SearchResult {
                preview: "memory three".into(),
                score: 0.96,
                source: Some("memory".into()),
                id: Some("m3".into()),
                key: Some("key-3".into()),
            },
        ];
        let (block, records) = render_recall_context(results).unwrap();
        assert!(!block.contains("chat must not be installed"));
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .all(|record| record.provenance[0].starts_with("spectral_search_result:")));
        let attributed = records
            .iter()
            .map(|record| record.text.as_str())
            .collect::<String>();
        assert_eq!(attributed, block);
        assert!(block.chars().count() <= MAX_RECALL_CHARS);
    }

    #[test]
    fn duplicate_and_idless_hits_keep_packet_and_block_in_lockstep() {
        let (block, records) = render_recall_context(vec![
            SearchResult {
                preview: "first version".into(),
                score: 0.99,
                source: Some("memory".into()),
                id: Some("same".into()),
                key: Some("same-key".into()),
            },
            SearchResult {
                preview: "duplicate key must disappear".into(),
                score: 0.98,
                source: Some("memory".into()),
                id: Some("different-id".into()),
                key: Some("same-key".into()),
            },
            SearchResult {
                preview: "anonymous one".into(),
                score: 0.97,
                source: Some("memory".into()),
                id: None,
                key: None,
            },
            SearchResult {
                preview: "anonymous two".into(),
                score: 0.96,
                source: Some("memory".into()),
                id: None,
                key: None,
            },
        ])
        .unwrap();
        assert!(!block.contains("duplicate key must disappear"));
        assert_eq!(records.len(), 3);
        assert_ne!(records[1].key, records[2].key);
        assert_eq!(
            records
                .iter()
                .map(|record| record.text.as_str())
                .collect::<String>(),
            block
        );
    }

    /// The bearer token is not optional. A daemon that rejects the request
    /// yields no block at all — never a partial or unauthenticated read.
    #[tokio::test]
    async fn an_unauthenticated_read_yields_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert!(recall_at(&ep_for(server.uri()), "anything", &repo_scope())
            .await
            .is_none());
    }

    /// (b) A daemon that is down must cost the turn nothing but the bound, and
    /// must inject nothing. This is the normal case for `permagent run` in a
    /// terminal, not an error.
    #[tokio::test]
    async fn a_dead_daemon_injects_nothing_within_the_bound() {
        // Port 1 on loopback: nothing listens, and the refusal is immediate —
        // the same shape as a daemon that was never started.
        let start = Instant::now();
        let out = recall_at(
            &ep_for("http://127.0.0.1:1".to_string()),
            "anything",
            &repo_scope(),
        )
        .await;
        let elapsed = start.elapsed();
        assert!(out.is_none(), "a dead daemon must inject nothing");
        assert!(
            elapsed <= BRIDGE_TIMEOUT + Duration::from_millis(250),
            "recall must stay inside its budget even with no daemon: {elapsed:?}"
        );
    }

    /// A daemon that accepts the connection and then never answers is the worst
    /// case, and it is the one the budget exists for: the turn proceeds, late.
    #[tokio::test]
    async fn a_wedged_daemon_is_abandoned_at_the_bound() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(5))
                    .set_body_json(serde_json::json!({ "results": [] })),
            )
            .mount(&server)
            .await;

        let start = Instant::now();
        let out = recall_at(&ep_for(server.uri()), "anything", &repo_scope()).await;
        let elapsed = start.elapsed();
        assert!(out.is_none());
        assert!(
            elapsed <= BRIDGE_TIMEOUT + Duration::from_millis(250),
            "a wedged daemon must be abandoned at the bound, not waited out: {elapsed:?}"
        );
    }

    /// A stale block from an earlier turn is worse than no block: it presents
    /// turn 1's memories as background for turn 9's unrelated question. When a
    /// turn recalls nothing, the previous turn's block is cleared.
    #[tokio::test]
    async fn a_turn_with_no_hits_clears_the_previous_turns_block() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "preview": "weak", "score": 0.1 }]
            })))
            .mount(&server)
            .await;
        let ep = ep_for(server.uri());

        assert_eq!(
            recall_block_at(&ep, "anything", &repo_scope(), false).await,
            None,
            "nothing recalled and nothing stale ⇒ write nothing"
        );
        assert_eq!(
            recall_block_at(&ep, "anything", &repo_scope(), true).await,
            Some(String::new()),
            "nothing recalled but a block is installed ⇒ clear it"
        );
    }

    /// (c) The write is fire-and-forget: the response path never waits on the
    /// daemon. Proven against a server that takes two seconds to answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn persisting_a_turn_does_not_wait_for_the_daemon() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/coding-sessions/turn"))
            .and(header(
                "authorization",
                "Bearer fixture-token-not-the-real-one",
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(serde_json::json!({ "accepted": true })),
            )
            .mount(&server)
            .await;

        let start = Instant::now();
        spawn_persist(
            ep_for(server.uri()),
            "sess-1".to_string(),
            3,
            "user said".to_string(),
            "assistant said".to_string(),
            Some("/tmp/proj".to_string()),
            None,
        );
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "the turn must not wait on the memory write: {:?}",
            start.elapsed()
        );

        // …and it does eventually arrive, so "fire and forget" is not "drop".
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "the detached write must still reach the daemon"
        );
    }

    /// Lane R60a: eval runs were not hermetic. `endpoint()` resolved the daemon
    /// token and port from `$HOME/.permagent` by hand, ignoring
    /// `PERMAGENT_PATH_ROOT`, so a harness run under a fresh per-run root still
    /// reached the operator's live daemon and injected their private Brain
    /// recall into the eval model's prompt. Here HOME holds a decoy "operator"
    /// token and port: a rooted process must not see either, and must still
    /// reach a daemon that belongs to its OWN root.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_path_rooted_process_never_resolves_the_operators_daemon() {
        let operator_home = tempfile::tempdir().unwrap();
        let operator_dir = operator_home.path().join(".permagent");
        std::fs::create_dir_all(operator_dir.join("secrets")).unwrap();
        std::fs::write(
            operator_dir.join("secrets").join("daemon_token.json"),
            r#"{"token":"operator-real-token"}"#,
        )
        .unwrap();
        std::fs::write(operator_dir.join("config.yaml"), "daemon:\n  port: 4999\n").unwrap();
        let run_root = tempfile::tempdir().unwrap();
        let home = operator_home.path().to_str().unwrap();
        let root = run_root.path().to_str().unwrap();
        let _env = env_lock::lock_env([("HOME", Some(home)), ("PERMAGENT_PATH_ROOT", Some(root))]);

        assert!(
            endpoint().is_err(),
            "a fresh path root holds no daemon token, so the bridge must fail closed \
             instead of borrowing the operator's from HOME"
        );
        assert!(
            recall_context("where does the scanner run?", &repo_scope())
                .await
                .is_none(),
            "no recall may be installed for a process whose root has no daemon"
        );

        std::fs::create_dir_all(run_root.path().join("secrets")).unwrap();
        std::fs::write(
            run_root.path().join("secrets").join("daemon_token.json"),
            r#"{"token":"rooted-token"}"#,
        )
        .unwrap();
        std::fs::write(
            run_root.path().join("config.yaml"),
            "daemon:\n  port: 4123\n",
        )
        .unwrap();
        let ep = endpoint().expect("a daemon owned by the same root must stay reachable");
        assert_eq!(ep.token, "rooted-token");
        assert_eq!(ep.base, "http://127.0.0.1:4123");
    }

    /// An empty half is not a turn. The route rejects it (400); spending a task
    /// on a guaranteed refusal is just noise in the daemon's log.
    #[tokio::test]
    async fn an_empty_turn_is_not_sent() {
        // No endpoint is resolvable in the test environment either way, so this
        // asserts the guard rather than the transport: it must not panic and
        // must not spawn.
        persist_turn(
            "sess".to_string(),
            0,
            "   ".to_string(),
            "answer".to_string(),
            None,
            None,
        );
    }

    // ── Lane R61a: repo-scoped recall and user-approved widening ──

    fn query_pairs(url: &url::Url) -> HashMap<String, String> {
        url.query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    fn plekk() -> ResolvedMemoryTarget {
        ResolvedMemoryTarget::Project {
            slug: "plekk".to_string(),
            name: "Plekk".to_string(),
            root_path: Some("/work/plekk".to_string()),
        }
    }

    fn never_prompted(_: &MemoryAccessPrompt) -> MemoryGrantAnswer {
        panic!("the user must not be asked on this path")
    }

    #[test]
    fn a_harness_scope_builds_a_repo_scoped_query_from_its_working_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(main.join("crates/core")).unwrap();

        let scope = MemoryScope::for_working_dir(&main.join("crates/core"));
        let canonical_main = std::fs::canonicalize(&main).unwrap();
        let canonical_cwd = std::fs::canonicalize(main.join("crates/core")).unwrap();
        let url = search_url("http://127.0.0.1:4123", "retry policy", &scope, 3).unwrap();
        let pairs = query_pairs(&url);
        assert_eq!(url.path(), "/api/brain/search");
        assert_eq!(pairs["q"], "retry policy");
        assert_eq!(pairs["source"], "memory");
        assert_eq!(pairs["scope_cwd"], canonical_cwd.to_string_lossy());
        assert_eq!(pairs["scope_repo_root"], canonical_main.to_string_lossy());
        assert!(
            !pairs.contains_key("scope_projects") && !pairs.contains_key("scope_all"),
            "a fresh session holds no grant: {pairs:?}"
        );
    }

    #[test]
    fn a_linked_worktree_names_its_main_checkout_as_the_repo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("permagent-runtime");
        let worktree = tmp.path().join("permagent-worktrees").join("laneR61a");
        std::fs::create_dir_all(main.join(".git").join("worktrees").join("laneR61a")).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!(
                "gitdir: {}\n",
                main.join(".git/worktrees/laneR61a").display()
            ),
        )
        .unwrap();

        let scope = MemoryScope::for_working_dir(&worktree);
        let expected_root = std::fs::canonicalize(&main)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let expected_cwd = std::fs::canonicalize(&worktree)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(scope.repo_root(), Some(expected_root.as_str()));
        assert_eq!(scope.working_dir(), Some(expected_cwd.as_str()));
    }

    #[tokio::test]
    async fn a_scope_with_no_repo_and_no_grant_recalls_nothing() {
        let empty = MemoryScope::default();
        assert!(search_url("http://127.0.0.1:4123", "anything", &empty, 3).is_none());
        // Fails closed before any endpoint or network is touched.
        assert!(recall_context("anything", &empty).await.is_none());
    }

    #[test]
    fn a_request_without_a_terminal_is_refused_and_the_scope_is_unchanged() {
        let mut scope = repo_scope();
        let before = scope.clone();
        for target in [plekk(), ResolvedMemoryTarget::All] {
            let decision = decide_memory_access(
                &mut scope,
                &target,
                Some("acme-api"),
                "copy their retry pattern",
                false,
                never_prompted,
            );
            assert_eq!(decision, MemoryAccessDecision::Refused);
            assert_eq!(
                memory_access_result_text(&decision, &target, None),
                MEMORY_ACCESS_HEADLESS_REFUSAL
            );
        }
        assert_eq!(scope, before, "a refused request never widens recall");
        assert!(MEMORY_ACCESS_HEADLESS_REFUSAL
            .contains("needs the user's approval in an interactive terminal"));
        assert!(MEMORY_ACCESS_HEADLESS_REFUSAL.contains("Continue with this repo's context"));
    }

    #[test]
    fn approving_for_the_session_widens_recall_to_exactly_that_project() {
        let mut scope = repo_scope();
        let mut asked = None;
        let decision = decide_memory_access(
            &mut scope,
            &plekk(),
            Some("acme-api"),
            "copy their retry pattern",
            true,
            |prompt| {
                asked = Some(prompt.clone());
                MemoryGrantAnswer::AllowForSession
            },
        );
        let asked = asked.expect("the user must be asked");
        assert!(asked.target_label.contains("Plekk") && asked.target_label.contains("plekk"));
        assert_eq!(asked.reason, "copy their retry pattern");
        let MemoryAccessDecision::Granted {
            search_scope,
            for_session: true,
        } = &decision
        else {
            panic!("expected a session grant, got {decision:?}");
        };
        let granted = query_pairs(&search_url("http://d", "retry", search_scope, 5).unwrap());
        assert_eq!(granted["scope_projects"], "plekk");
        assert!(
            !granted.contains_key("scope_cwd"),
            "the grant's own search shows what the grant added"
        );

        // Later turns' recall: this repo plus exactly the granted project.
        let recall = query_pairs(&search_url("http://d", "retry", &scope, 3).unwrap());
        assert_eq!(recall["scope_cwd"], "/work/acme-api");
        assert_eq!(recall["scope_projects"], "plekk");
        assert!(!recall.contains_key("scope_all"));

        // The same scope again is covered for the session: nobody is re-asked.
        let again = decide_memory_access(
            &mut scope,
            &plekk(),
            Some("acme-api"),
            "more",
            true,
            never_prompted,
        );
        assert!(matches!(
            again,
            MemoryAccessDecision::Granted {
                for_session: true,
                ..
            }
        ));
        // But a wider scope is a new decision.
        let wider = decide_memory_access(
            &mut scope,
            &ResolvedMemoryTarget::All,
            Some("acme-api"),
            "patterns",
            true,
            |_| MemoryGrantAnswer::Deny,
        );
        assert_eq!(wider, MemoryAccessDecision::Denied);
    }

    #[test]
    fn denying_keeps_memory_scoped_to_this_repo() {
        let mut scope = repo_scope();
        let before = scope.clone();
        let decision = decide_memory_access(
            &mut scope,
            &ResolvedMemoryTarget::All,
            Some("acme-api"),
            "identify patterns",
            true,
            |_| MemoryGrantAnswer::Deny,
        );
        assert_eq!(decision, MemoryAccessDecision::Denied);
        assert_eq!(scope, before);
        let text = memory_access_result_text(&decision, &ResolvedMemoryTarget::All, None);
        assert!(text.contains("denied access to all memories"), "{text}");
    }

    #[test]
    fn allow_once_searches_the_grant_without_widening_the_session() {
        let mut scope = repo_scope();
        let before = scope.clone();
        let decision = decide_memory_access(
            &mut scope,
            &ResolvedMemoryTarget::All,
            Some("acme-api"),
            "identify patterns",
            true,
            |_| MemoryGrantAnswer::AllowOnce,
        );
        let MemoryAccessDecision::Granted {
            search_scope,
            for_session: false,
        } = &decision
        else {
            panic!("expected a one-time grant, got {decision:?}");
        };
        let once = query_pairs(&search_url("http://d", "patterns", search_scope, 5).unwrap());
        assert_eq!(once["scope_all"], "true");
        assert_eq!(scope, before, "allow once never widens later recall");
        let text = memory_access_result_text(
            &decision,
            &ResolvedMemoryTarget::All,
            Some("Memories from all memories …\n- a fact\n"),
        );
        assert!(text.contains("for this request only") && text.contains("- a fact"));
    }

    #[test]
    fn the_sessions_own_repo_needs_no_approval() {
        let mut scope = repo_scope();
        let own = ResolvedMemoryTarget::Project {
            slug: "acme-api".to_string(),
            name: "Acme API".to_string(),
            root_path: Some("/work/acme-api".to_string()),
        };
        let decision = decide_memory_access(
            &mut scope,
            &own,
            Some("acme-api"),
            "why",
            false,
            never_prompted,
        );
        assert_eq!(decision, MemoryAccessDecision::AlreadyInScope);
    }

    #[test]
    fn a_request_must_name_its_scope_reason_and_query() {
        let args = |value: serde_json::Value| match value {
            serde_json::Value::Object(map) => map,
            _ => unreachable!(),
        };
        let ok = parse_memory_access_request(Some(&args(serde_json::json!({
            "scope": "project", "project": "plekk", "reason": "patterns", "query": "retry"
        }))))
        .unwrap();
        assert_eq!(ok.target, MemoryAccessTarget::Project("plekk".to_string()));
        assert!(parse_memory_access_request(Some(&args(serde_json::json!({
            "scope": "project", "reason": "patterns", "query": "retry"
        }))))
        .is_err());
        assert!(parse_memory_access_request(Some(&args(serde_json::json!({
            "scope": "everything", "reason": "patterns", "query": "retry"
        }))))
        .is_err());
        assert!(parse_memory_access_request(Some(&args(serde_json::json!({
            "scope": "all", "query": "retry"
        }))))
        .is_err());
        assert!(parse_memory_access_request(None).is_err());
        assert_eq!(memory_access_tool().name, MEMORY_ACCESS_TOOL_NAME);
    }

    #[tokio::test]
    async fn a_widened_session_labels_its_recall_and_sends_the_grant() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/brain/search"))
            .and(query_param("scope_cwd", "/work/acme-api"))
            .and(query_param("scope_projects", "plekk"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "source": "memory", "key": "p1", "preview": "Plekk retries with jitter", "score": 0.9 }]
            })))
            .mount(&server)
            .await;
        let mut scope = repo_scope();
        scope.grant(&plekk());
        let block = recall_at(&ep_for(server.uri()), "retry", &scope)
            .await
            .expect("the granted project's memory is recalled");
        assert!(block.starts_with(RECALL_HEADER_WIDENED), "{block}");
        assert!(block.contains("Plekk retries with jitter"));
    }
}
