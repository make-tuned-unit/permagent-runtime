//! Read-only MCP bridge for external-CLI goal workers.
//!
//! ## The gap this closes
//!
//! An external-CLI worker (`ExternalCliEngine` — a real `claude`/`codex` binary
//! in an isolated worktree) goes through none of goose's `PromptManager`,
//! `extension_manager`, or Brain. Its entire knowledge of Permagent is the flat
//! string assembled at dispatch, and it has **no tool that reaches back** — the
//! board, the project row, the notes and the session history are all visible to
//! an in-process session and invisible to the harness that actually does the
//! goal work. The only remedy available before this was more injection, and the
//! 2026-08-10 goals A/B measurement on the code map already showed where that
//! ends: injected bulk changes no behaviour. What changed behaviour was adding
//! a tool the worker could ASK, at the moment it needed the answer.
//!
//! So: four tools, served over stdio, declared to the worker CLI through its
//! `--mcp-config` at spawn ([`super::goal_engine`]).
//!
//! ## Two invariants, enforced structurally rather than by prose
//!
//! **Read-only.** The roster is a fixed allowlist, and dispatch is by exact
//! name against it. There is no write path to disable, no "mutating" flag to
//! get wrong: a call to `card_create` fails because `card_create` is not a name
//! this server knows, and the refusal names the four that are. A worker's
//! writes to goal state must keep going through `goal_transition`'s guard,
//! never around it via a bridge the worker holds open for hours.
//!
//! **Project-scoped.** Every query binds the project id the server was started
//! with; none of them takes a project as a free parameter. A `project_id`
//! argument IS accepted — and refused when it names a different project —
//! because a silent ignore teaches a worker that the argument works. Today a
//! worktree-isolated worker has zero read access to Permagent; a bridge that
//! let it enumerate every project on the machine would be a regression from
//! that baseline, not an improvement.
//!
//! ## Why the JSON-RPC loop is hand-written
//!
//! `permagent`'s `rmcp` dependency carries client-side features only; the
//! server + stdio transport features live on `permagent-mcp`, which does not
//! (and should not) depend on `permagent` and so cannot reach `cards` /
//! `projects` / `project_notes` / `sessions`. Turning on `rmcp/server` for the
//! whole of `permagent` to serve four read-only queries is a large change to a
//! workspace-wide dependency for a small surface. MCP over stdio is
//! newline-delimited JSON-RPC 2.0 with four methods that matter, so they are
//! implemented here directly — which also leaves [`handle_request`] a pure
//! function, so the read-only and scoping invariants above are testable without
//! spawning anything.

use serde_json::{json, Value};

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Character cap on any tool answer, matching `code_map::MAP_QUERY_MAX_CHARS`.
/// An uncapped board on a busy project is the feature defeating itself: the
/// worker pays more to read the answer than the exploration it replaced.
pub(crate) const ANSWER_MAX_CHARS: usize = 2_000;

/// The complete tool roster. Read-only by construction: this list IS the
/// dispatch table, so a tool that does not appear here cannot be called.
pub(crate) const TOOL_NAMES: &[&str] = &[
    "board_query",
    "project_get",
    "notes_search",
    "session_history",
];

/// Whether a worker CLI can be given the bridge.
///
/// Today: claude only, because `--mcp-config` is a per-invocation flag and the
/// dispatcher can therefore hand each worker a config confined to its own
/// worktree. Codex configures MCP servers through `~/.codex/config.toml`, which
/// is machine-global — attaching the bridge there would scope one worker's
/// project to every codex run on the box, which is exactly the isolation this
/// module exists to keep. So codex workers get no bridge until a per-invocation
/// path exists, and the dispatch digest must not promise them one.
pub fn bridge_supported(bin: &str) -> bool {
    bin.contains("claude")
}

/// Tool definitions as the MCP `tools/list` result.
///
/// Every one is annotated `readOnlyHint: true`. The annotation is advisory to
/// the client; the allowlist above is what actually enforces it.
pub(crate) fn tool_definitions(project_label: &str) -> Value {
    json!([
        {
            "name": "board_query",
            "description": format!(
                "List goal cards on {project_label}'s kanban board (live read, unlike the \
                 dispatch-time digest in your brief). Optionally filter by column name or \
                 assignee. Read-only; scoped to this project."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "column": { "type": "string", "description": "Column name to filter by, e.g. 'In Progress'." },
                    "assigned_to": { "type": "string", "description": "Worker key to filter by." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false }
        },
        {
            "name": "project_get",
            "description": format!(
                "The full project row for {project_label}: description, root path, repo, \
                 notes, tags, and metadata (build command, publish sequence, strategy, \
                 brand). Read-only; scoped to this project."
            ),
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false }
        },
        {
            "name": "notes_search",
            "description": format!(
                "Search {project_label}'s project notes by substring, newest first. \
                 Read-only; scoped to this project."
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Substring to match in a note's title or body." }
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false }
        },
        {
            "name": "session_history",
            "description": format!(
                "List recent agent sessions that ran under {project_label}'s root path, \
                 newest first. Read-only; scoped to this project."
            ),
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false }
        }
    ])
}

/// Argument keys that name a project. Present-and-different is refused; the
/// alternative — ignoring them — teaches a worker the parameter took effect.
const PROJECT_ARG_KEYS: &[&str] = &[
    "project_id",
    "project",
    "project_id_or_slug",
    "project_slug",
];

/// Refusal text for a call outside the read-only roster. Names the roster, so
/// the worker learns the boundary instead of retrying variations of a write.
pub(crate) fn unknown_tool_refusal(name: &str) -> String {
    format!(
        "'{name}' is not available. This bridge is READ-ONLY and exposes exactly: {}. \
         Permagent state is changed through the goal lifecycle (review/approve), never \
         from inside a worker.",
        TOOL_NAMES.join(", ")
    )
}

/// Scope check shared by every tool: a project-naming argument must either be
/// absent or name the project this server is bound to.
fn check_scope(args: &Value, project_id: &str, project_slug: &str) -> Result<(), String> {
    for key in PROJECT_ARG_KEYS {
        let Some(requested) = args.get(*key).and_then(Value::as_str) else {
            continue;
        };
        if requested != project_id && requested != project_slug {
            return Err(format!(
                "Refused: this bridge is scoped to project {project_id} (slug {project_slug}) \
                 — the goal you were dispatched for. It cannot read project '{requested}'."
            ));
        }
    }
    Ok(())
}

fn cap(mut s: String) -> String {
    if s.chars().count() > ANSWER_MAX_CHARS {
        s = s.chars().take(ANSWER_MAX_CHARS).collect();
        s.push_str("\n[output cut at the character budget — narrow the query]");
    }
    s
}

/// Run one read-only tool against the DB, already scope-checked.
pub(crate) async fn call_tool(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    project: &crate::projects::Project,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    check_scope(args, &project.id, &project.slug)?;

    match name {
        "board_query" => {
            let column = args.get("column").and_then(Value::as_str);
            let assignee = args.get("assigned_to").and_then(Value::as_str);
            let columns = crate::cards::list_columns(pool, &project.id).await?;
            let cards = crate::cards::list_cards(pool, &project.id, Some("goal"), None).await?;
            let mut out = String::new();
            for card in &cards {
                let col_name = columns
                    .iter()
                    .find(|c| c.id == card.column_id)
                    .map(|c| c.name.as_str())
                    .unwrap_or("(unknown column)");
                if column.is_some_and(|c| !col_name.eq_ignore_ascii_case(c)) {
                    continue;
                }
                let who = card.assigned_to.as_deref().unwrap_or("unassigned");
                if assignee.is_some_and(|a| who != a) {
                    continue;
                }
                out.push_str(&format!("- {} — {col_name} — {who}\n", card.title));
            }
            if out.is_empty() {
                return Ok(format!(
                    "No goal cards on {}'s board match that filter.",
                    project.name
                ));
            }
            Ok(cap(format!("Goals on {}'s board:\n{out}", project.name)))
        }
        "project_get" => {
            // Serialized from the row, so a field added to `Project` shows up
            // here without this arm being edited — and nothing about OTHER
            // projects can appear, because there is only one row to serialize.
            let body = json!({
                "id": project.id,
                "slug": project.slug,
                "name": project.name,
                "description": project.description,
                "status": project.status,
                "root_path": project.root_path,
                "site_url": project.site_url,
                "repo_url": project.repo_url,
                "notes": project.notes,
                "tags": project.tags,
                "metadata": project.metadata_json,
            });
            Ok(cap(serde_json::to_string_pretty(&body).unwrap_or_default()))
        }
        "notes_search" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|q| !q.is_empty())
                .ok_or_else(|| "notes_search needs a non-empty 'query'.".to_string())?;
            let needle = query.to_lowercase();
            let notes = crate::project_notes::list_notes(pool, &project.id).await?;
            let mut out = String::new();
            for note in notes.iter().filter(|n| {
                n.body.to_lowercase().contains(&needle)
                    || n.title
                        .as_deref()
                        .is_some_and(|t| t.to_lowercase().contains(&needle))
            }) {
                out.push_str(&format!(
                    "## {} ({})\n{}\n\n",
                    note.title.as_deref().unwrap_or("(untitled)"),
                    note.updated_at,
                    note.body
                ));
            }
            if out.is_empty() {
                return Ok(format!(
                    "No notes on project {} match \"{query}\".",
                    project.name
                ));
            }
            Ok(cap(out))
        }
        "session_history" => {
            // Scoped by working_dir: sessions carry no project_id, so the
            // project's root path is the only honest boundary. A session that
            // ran elsewhere is another project's business.
            let Some(root) = project.root_path.as_deref().filter(|r| !r.is_empty()) else {
                return Ok(format!(
                    "Project {} has no root path recorded, so its sessions cannot be \
                     identified.",
                    project.name
                ));
            };
            let prefix = format!("{}%", root.trim_end_matches('/'));
            let rows = sqlx::query_as::<_, (String, String, String, String)>(
                "SELECT id, name, working_dir, updated_at FROM sessions
                 WHERE working_dir = ? OR working_dir LIKE ?
                 ORDER BY updated_at DESC LIMIT 20",
            )
            .bind(root)
            .bind(&prefix)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
            if rows.is_empty() {
                return Ok(format!("No sessions recorded under {root}."));
            }
            let mut out = format!("Recent sessions under {root}:\n");
            for (id, name, dir, updated) in &rows {
                out.push_str(&format!("- {updated} — {name} [{id}] in {dir}\n"));
            }
            Ok(cap(out))
        }
        other => Err(unknown_tool_refusal(other)),
    }
}

// ── JSON-RPC plumbing ────────────────────────────────────────────────────────

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Text content result. `is_error` marks a refusal as a TOOL error rather than
/// a protocol error, which is what lets the model read the reason and adapt —
/// a JSON-RPC error would surface to it as a broken tool.
fn text_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// The outcome of routing one request: a reply to write back, a tool call the
/// async layer must run first, or nothing (a notification).
pub(crate) enum Routed {
    Reply(Value),
    Call {
        id: Value,
        name: String,
        args: Value,
    },
    Nothing,
}

/// Pure request router: everything except the DB reads themselves.
///
/// Splitting here is what makes the two invariants testable — a write attempt
/// and a cross-project query are both decided in this function, with no daemon,
/// no process and no database.
pub(crate) fn handle_request(req: &Value, project_label: &str) -> Routed {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "initialize" => Routed::Reply(ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "permagent-goal-context", "version": env!("CARGO_PKG_VERSION") },
                "instructions": format!(
                    "Read-only view of Permagent for project {project_label}. These tools \
                     answer live, unlike the snapshot in your dispatch brief. Nothing here \
                     can change Permagent state."
                )
            }),
        )),
        "ping" => Routed::Reply(ok(id, json!({}))),
        "tools/list" => Routed::Reply(ok(id, json!({ "tools": tool_definitions(project_label) }))),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // THE read-only gate: a name outside the roster never reaches a
            // handler, so there is no write path to forget to close.
            if !TOOL_NAMES.contains(&name.as_str()) {
                return Routed::Reply(ok(id, text_result(&unknown_tool_refusal(&name), true)));
            }
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Routed::Call { id, name, args }
        }
        // Notifications carry no id and want no reply.
        m if m.starts_with("notifications/") => Routed::Nothing,
        other => Routed::Reply(err(id, -32601, &format!("Method not found: {other}"))),
    }
}

/// Serve the bridge over stdio until the client closes it.
///
/// Refuses to start for an unknown project: a bridge that silently answers
/// nothing is worse than one that fails at spawn, where the dispatcher sees it.
pub async fn serve_stdio(project_id: &str) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let pool = crate::session::SessionManager::instance()
        .pool_clone()
        .await?;
    let project = crate::projects::get_project(&pool, project_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("goal-context bridge: no project '{project_id}'"))?;
    let label = format!("\"{}\" (slug: {})", project.name, project.slug);

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let reply = match handle_request(&req, &label) {
            Routed::Nothing => continue,
            Routed::Reply(v) => v,
            Routed::Call { id, name, args } => match call_tool(&pool, &project, &name, &args).await
            {
                Ok(text) => ok(id, text_result(&text, false)),
                // A refusal or a DB failure comes back as a tool error the
                // model can read, not a protocol error it can only choke on.
                Err(msg) => ok(id, text_result(&msg, true)),
            },
        };
        stdout
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .and(stdout.flush().await)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABEL: &str = "\"P\" (slug: p)";

    fn call(name: &str, args: Value) -> Value {
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        })
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::session::spectral_schema::init_spectral_db(&pool)
            .await
            .unwrap();
        pool
    }

    async fn project(pool: &sqlx::Pool<sqlx::Sqlite>, name: &str) -> crate::projects::Project {
        crate::projects::create_project(
            pool,
            crate::projects::CreateProject {
                name: name.into(),
                slug: None,
                description: None,
                root_path: None,
                site_url: None,
                repo_url: None,
                notes: None,
                tags: None,
            },
        )
        .await
        .unwrap()
    }

    /// The read-only invariant, structurally: every write-shaped tool the
    /// in-process extensions expose is rejected at the router, before any
    /// handler or database. There is no mutating handler to disable because
    /// there is no mutating handler.
    #[test]
    fn every_write_shaped_tool_is_refused_at_the_router() {
        for write in [
            "card_create",
            "card_update",
            "card_move",
            "card_delete",
            "project_create",
            "project_update",
            "project_delete",
            "column_create",
            "goal_advance",
            "context_set",
            "set_project_strategy",
        ] {
            let Routed::Reply(reply) = handle_request(&call(write, json!({})), LABEL) else {
                panic!("{write} was routed to a handler instead of being refused");
            };
            let result = &reply["result"];
            assert_eq!(result["isError"], json!(true), "{write} was not an error");
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("READ-ONLY"), "{write}: {text}");
            // The refusal teaches the boundary rather than inviting retries.
            assert!(text.contains("board_query"), "{write}: {text}");
        }
    }

    /// …and the four roster tools DO route through, or the test above would
    /// pass on a server that refuses everything.
    #[test]
    fn the_roster_tools_route_to_a_handler() {
        for name in TOOL_NAMES {
            match handle_request(&call(name, json!({})), LABEL) {
                Routed::Call { name: got, .. } => assert_eq!(&got, name),
                _ => panic!("{name} did not route to a handler"),
            }
        }
    }

    /// Advertised tools and the dispatch table are the same set. A tool
    /// advertised but unroutable is a dead entry a worker pays tokens for; a
    /// tool routable but unadvertised is a surface nobody reviewed.
    #[test]
    fn advertised_tools_match_the_allowlist_exactly() {
        let defs = tool_definitions(LABEL);
        let advertised: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(advertised, TOOL_NAMES);
        for def in defs.as_array().unwrap() {
            assert_eq!(
                def["annotations"]["readOnlyHint"],
                json!(true),
                "{} is not annotated read-only",
                def["name"]
            );
        }
    }

    /// Cross-project isolation. A worktree-isolated worker has zero read access
    /// today; a bridge that let it name someone else's project would be a
    /// regression from that baseline, not an improvement.
    #[tokio::test]
    async fn a_query_naming_another_project_is_denied() {
        let pool = test_pool().await;
        let mine = project(&pool, "Mine").await;
        let theirs = project(&pool, "Theirs").await;

        for tool in TOOL_NAMES {
            let err = call_tool(
                &pool,
                &mine,
                tool,
                &json!({ "project_id": theirs.id, "query": "x" }),
            )
            .await
            .unwrap_err();
            assert!(err.contains("Refused"), "{tool}: {err}");
            assert!(
                err.contains(&mine.id),
                "{tool} must name its own scope: {err}"
            );
            assert!(
                !err.contains(&theirs.name),
                "{tool} leaked the other project's name: {err}"
            );
        }
        // The slug spelling of the same escape is refused too.
        let err = call_tool(
            &pool,
            &mine,
            "project_get",
            &json!({ "project": theirs.slug }),
        )
        .await
        .unwrap_err();
        assert!(err.contains("Refused"), "{err}");
    }

    /// Naming your OWN project explicitly is fine — the check is scope, not a
    /// blanket ban on the argument.
    #[tokio::test]
    async fn naming_your_own_project_is_allowed() {
        let pool = test_pool().await;
        let mine = project(&pool, "Mine").await;
        let out = call_tool(
            &pool,
            &mine,
            "project_get",
            &json!({ "project_id": mine.id }),
        )
        .await
        .unwrap();
        assert!(out.contains("Mine"));
        let out = call_tool(
            &pool,
            &mine,
            "project_get",
            &json!({ "project": mine.slug }),
        )
        .await
        .unwrap();
        assert!(out.contains("Mine"));
    }

    /// Even reaching `call_tool` directly — past the router — a write name is
    /// refused. Defence in depth: the router is the gate, this is the floor.
    #[tokio::test]
    async fn call_tool_itself_refuses_an_unknown_name() {
        let pool = test_pool().await;
        let mine = project(&pool, "Mine").await;
        let err = call_tool(&pool, &mine, "card_create", &json!({ "title": "x" }))
            .await
            .unwrap_err();
        assert!(err.contains("READ-ONLY"), "{err}");
    }

    /// `board_query` answers live and only about this project's board.
    #[tokio::test]
    async fn board_query_returns_only_this_projects_cards() {
        let pool = test_pool().await;
        let mine = project(&pool, "Mine").await;
        let theirs = project(&pool, "Theirs").await;
        for (p, title) in [(&mine, "My goal"), (&theirs, "Their goal")] {
            crate::cards::seed_goal_columns(&pool, &p.id).await.unwrap();
            crate::cards::create_card(
                &pool,
                crate::cards::CreateCard {
                    project_id: p.id.clone(),
                    title: title.into(),
                    description: None,
                    card_type: Some("goal".into()),
                    column_id: None,
                    created_by: None,
                    metadata_json: None,
                },
            )
            .await
            .unwrap();
        }
        let out = call_tool(&pool, &mine, "board_query", &json!({}))
            .await
            .unwrap();
        assert!(out.contains("My goal"));
        assert!(
            !out.contains("Their goal"),
            "another project's board leaked: {out}"
        );
    }

    /// A tool answer is capped like `map_query`'s: an uncapped board on a busy
    /// project costs more to read than the exploration it replaced.
    #[tokio::test]
    async fn answers_are_capped() {
        let pool = test_pool().await;
        let mine = project(&pool, "Mine").await;
        crate::cards::seed_goal_columns(&pool, &mine.id)
            .await
            .unwrap();
        for i in 0..200 {
            crate::cards::create_card(
                &pool,
                crate::cards::CreateCard {
                    project_id: mine.id.clone(),
                    title: format!("A reasonably long goal title number {i:04}"),
                    description: None,
                    card_type: Some("goal".into()),
                    column_id: None,
                    created_by: None,
                    metadata_json: None,
                },
            )
            .await
            .unwrap();
        }
        let out = call_tool(&pool, &mine, "board_query", &json!({}))
            .await
            .unwrap();
        assert!(out.contains("[output cut"), "cut must be announced");
        assert!(out.chars().count() <= ANSWER_MAX_CHARS + 80);
    }

    /// Handshake shape: a client that cannot initialize sees no tools at all.
    #[test]
    fn initialize_advertises_tools_and_a_read_only_instruction() {
        let req = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize" });
        let Routed::Reply(reply) = handle_request(&req, LABEL) else {
            panic!("initialize must reply");
        };
        assert_eq!(reply["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        let instructions = reply["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("Nothing here can change Permagent state"));

        // A notification gets no reply at all — a reply to one is a protocol
        // violation that some clients treat as fatal.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(matches!(handle_request(&note, LABEL), Routed::Nothing));
    }
}
