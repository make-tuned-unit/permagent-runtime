# Lane R61a: harness memory is scoped to its repo, and wider access needs the user's terminal approval

Branch `lane/R61a` off `53de25c5`. This lane only edited code. Nothing was compiled or run here; the coordinator runs the build and test gates.

## Ruling

Jesse, 2026-09-16: "The harness should only ever have access to memories about the project repo it is actively in unless the user tells them to look at a different project repo or memories to learn or identify patterns. giving access to memories should be a decision the user approves in terminal like how Codex/Claude Code do."

## How project attribution works today (before this lane)

- **A memory's project is its `wing`.** Spectral's `MemoryHit.wing: Option<String>` is in the pinned Spectral checkout at `spectral-ingest/src/lib.rs:231`. For a project, the wing is `projects.slug`.
- **Projects table.** `projects.root_path` (`crates/goose/src/projects.rs:16`) is canonicalized when it is saved (`resolve_root_path`, `projects.rs:102`). The table lives in the sessions DB, which the CLI and the daemon share when they run under the same root.
- **How Chat turns get a wing.** The wing is set by `chat_turn_opts` (`crates/goose-server/src/brain_ops.rs:429`, `wing` at `:454`). The drain decides it with `session_wing::decide_turn_wing` (`brain_ops.rs:686`). That decision needs:
  - a session project hint, `sessions.project_hint_id` / `project_hint_wing`, set by `set_session_project_hint` (`crates/goose/src/session_wing.rs:429`);
  - corroboration from the turn itself (content name, alias, or a tool path under `root_path`).
  - The hint is also dropped after `HINT_GAP_SECONDS` (10 minutes) of silence (`session_wing.rs:118`, applied in `decide_turn_wing_at` `:635`).
- **Harness turns were unattributed.** `remember_coding_turn` (`crates/goose-server/src/routes/coding_session.rs`) only put `Harness working directory: <cwd>` into the tool text. Nothing ever set a project hint for a CLI session, so every harness turn was stored with no wing. The 10-minute gap rule would have dropped a hint between most coding turns anyway.
- **Recall read the whole Brain.** `brain_sync::recall_context` called `GET /api/brain/search?source=both` with no scope. `query_spectral` (`crates/goose-server/src/routes/brain.rs`) returned every recall hit, and `query_fts` returned every chat row. Spectral has no wing filter at recall time; `wing` only boosts ranking.
- **Sovereignty.** No `record_egress` call names memory recall. The recall block reaches the provider inside the prompt, and the provider guard records that as ordinary egress with a content hash (`crates/goose/src/providers/sovereign_guard.rs`). The loopback search is not egress. Nothing about this needed to change, and every claim in the audit is still accurate.

## Design

1. **Default scope is the current repo.**
   - The client sends `scope_cwd`: the session's canonical working directory, taken from `session.working_dir`, not from the process cwd.
   - When that directory is inside a linked git worktree, it also sends `scope_repo_root`, the main checkout. This is read from the `.git` file on disk, not by spawning git.
   - The daemon resolves these against its own project registry (`session_wing::resolve_project_wing_for_paths`). The deepest `root_path` that contains either path wins, compared by path component. `personal` never counts, and neither does a root of `/`.
   - A scoped search keeps only Spectral memories whose wing is in the scope, and drops chat FTS rows, which have no project attribution.
   - A directory that no project claims gets **no** memories (fail closed).
   - A request with no `scope_*` parameter behaves exactly as before. That is the Brain view's whole-brain search, so Chat and the UI are unchanged.
2. **Wider access needs an explicit grant.**
   - The coding harness registers `request_memory_access { scope: "project"|"all", project?, reason, query }` as a **frontend tool**. The agent parks the call until the CLI session loop answers it, so the model can only ask.
   - The loop resolves the requested project (by slug, or by a path inside its repo) against the registry.
   - It prints the scope and the model's reason, then asks with `cliclack::select`, the same surface as tool approval: **Allow once / Allow for this session / Deny**.
     - Allow once searches the granted scope once and returns the hits in the tool result.
     - Allow for this session also adds the grant to `CliSession.memory_scope`, so later recall includes it (`scope_projects` / `scope_all`).
     - Deny changes nothing.
     - Ctrl-C or any prompt error counts as a deny.
   - A project the session already has for this session is not asked about again. Asking about the session's own repo returns "already in scope".
   - **Grants never persist.** They live only in `CliSession.memory_scope` and end with the process. The tool prompt's "Always Allow" option does persist, so it is deliberately not offered here.
3. **Headless sessions.**
   - `can_prompt` is `interactive && stdin is a TTY && stdout is a TTY`. Without it, the request is refused before anything is read, with this result: `Refused: memory access beyond this repo needs the user's approval in an interactive terminal, and this session has none. Continue with this repo's context.` Recall stays scoped to the repo.
   - I did **not** use "headless dispatch policy installed" as the signal. `wants_headless_goal_dispatch` (`builder.rs`) also installs that policy for *interactive* Auto-mode harness sessions, so using it would have blocked approvals for a real user at the terminal.
   - **Goal workers.** Internal workers get only `GOAL_WORKER_EXTENSION_KEYS = ["developer", "orchestrator"]` (`orchestrator.rs:5050`), so no `search_memory`. They run in-process and never pass through `CliSession::process_agent_response`, so they have no brain_sync recall either. `GLOBAL_BRAIN` is set only by the daemon (`goose-server/src/state.rs:425`). A worker therefore can never see more than its coordinator. It actually sees nothing.
4. **`persist_turn` carries attribution.**
   - The CLI now sends `workingDir` (the same canonical directory recall uses) plus `repoRoot`.
   - `remember_coding_turn` calls `session_wing::assert_harness_session_project`. It sets the project hint (once only, never overwriting an existing one) and stamps the turn clock when the turn is queued, so the drain does not drop the hint over the gap. The cwd and repo root go into the tool text, and the drain's `ToolPath` corroboration then writes the wing.
   - Before this lane, harness turns did **not** carry attribution (see the data model above).
5. **Recipe.** One sentence in `permagent-coding.yaml`, and the inline recipe test pins it.
6. **Scope of the change.**
   - `brain_sync` serves only CLI sessions; there is no Chat path through it. So recall is repo-scoped for **every** CLI session, including a plain `permagent session`, which the ruling also covers.
   - The `request_memory_access` tool is registered only for the coding harness (`coding_harness`). A non-coding CLI session is scoped to its repo and has no way to widen. Registering the tool unconditionally in `builder.rs` would close that gap.

## What changed

| File | Change |
|---|---|
| `crates/goose/src/session_wing.rs:681` | `project_wing_for_paths` (pure, deepest root by path component). |
| `crates/goose/src/session_wing.rs:723` | `resolve_project_wing_for_paths` (registry, canonical form of each path). |
| `crates/goose/src/session_wing.rs:773` | `assert_harness_session_project` (once-only hint, clock stamped when the turn is queued). Tests at `:1517`, `:1548`, `:1568`, `:1597`. |
| `crates/goose-server/src/routes/brain.rs` | `scope_*` query params. `SearchScope` (`:55`: `Unscoped` / `All` / `Wings`). `resolve_search_scope` (`:113`). `retain_in_scope` (`:145`), applied before layering (`:252`). `brain_search` (`:326`) skips FTS under a wing scope. Tests at `:1558`, `:1572`, `:1584`, `:1595`. |
| `crates/goose-server/src/routes/coding_session.rs` | `CodingTurnReq.repo_root` (`:251`). Project attribution (`:305`) and repo-root evidence before `persist_chat_turn`. |
| `crates/goose-cli/src/session/brain_sync.rs` | `MemoryScope` (`:128`). `main_checkout_root` (`:212`). `search_url` (`:247`). Scoped `recall_context` (`:323`). Scoped headers (`:403`, `:406`). `persist_turn` gains `repo_root` (`:510`). `request_memory_access` tool, parse, resolve (`:702`), `decide_memory_access` policy (`:803`), `search_granted` (`:848`), result text. Tests at `:1368`–`:1613`. |
| `crates/goose-cli/src/session/mod.rs` | `CliSession.memory_scope` (`:293`, built at `:420`). Recall passes it (`:1886`). Frontend-request branch in the stream loop (`:2095`). `answer_memory_access_request` (`:2869`). `prompt_memory_access` (`:2939`). `persist_turn` uses the scope's paths (`:2386`). |
| `crates/goose-cli/src/session/builder.rs:1494` | Registers the `memory_access` frontend extension for coding-harness sessions. |
| `crates/goose-cli/src/recipes/builtin/permagent-coding.yaml:149` | The scope sentence (plus a Lane R61a header comment). |
| `crates/goose-cli/src/recipes/search_recipe.rs:179` | Assertions pinning that sentence. |

Existing brain_sync tests were updated for the new signatures. The live-daemon test now requires `source=memory` and `scope_cwd`. The R60a test still passes a scope.

## Tests (compile/run owned by the coordinator)

- **Server filter.** Fixture of rows from two projects plus unattributed rows. A scoped search returns only the current project's rows. Unattributed rows are excluded. An empty scope returns nothing. A granted project adds exactly that project. Only `scope_all` or no scope admits unattributed rows.
- **Wing resolution.** Deepest root wins. `personal` and `/` are ignored. A linked worktree resolves through its main checkout. A harness turn gets its wing after a 3-hour silence. A directory outside every project records nothing.
- **Client query.** The scoped query is built from a real tempdir repo (`scope_cwd`, `scope_repo_root`, `source=memory`, no grants). A tempdir linked worktree names its main checkout. An empty scope recalls nothing and makes no network call.
- **Request tool, non-interactive.** Refused, scope unchanged, prompt never called.
- **Request tool, interactive.** Covers approve-for-session (widens to exactly that project, is not re-asked, a wider scope is a new decision), deny, allow once (session not widened), own repo (no prompt), and argument validation.
- **Wiremock.** A widened recall sends `scope_projects` and uses the widened header.
- The prompt is a closure seam (`decide_memory_access`). No TTY, no process-global `OnceLock`. The only env-touching test is still R60a's, which is `#[serial_test::serial]`.

## Gate filters

Package names: `permagent` = crates/goose, `permagent-cli` = crates/goose-cli, `permagent-daemon` = crates/goose-server.

```
cargo test -p permagent --lib session_wing
cargo test -p permagent-daemon --lib routes::brain
cargo test -p permagent-daemon --test coding_turn_wiring
cargo test -p permagent-cli --lib brain_sync
cargo test -p permagent-cli --lib builtin_permagent_coding_recipe_resolves_and_parses
cargo clippy -p permagent -p permagent-cli -p permagent-daemon --all-targets -- -D warnings
```

## UX: the terminal prompt

```
Permagent is asking to read memories from outside this repo.
  Scope:  project "Plekk" (plekk, /Users/j/dev/plekk)
  Reason: <the model's reason, verbatim>
◆  Allow memory access beyond this repo?
│  ● Allow once              (Search it for this request only)
│  ○ Allow for this session  (Also include it in recall until this session ends)
│  ○ Deny                    (Keep memory scoped to this repo)
```

For `scope: "all"`, the Scope line reads `all memories (every project and general chat memory)`.

Tool results the model sees:
- **Refused (no terminal):** the text quoted above.
- **Denied:** `The user denied access to <scope>. Memory stays scoped to this repo; continue with this repo's context.`
- **Allow once:** `The user approved access to <scope> for this request only.` followed by a `Memories from <scope> (approved by the user; historical hints; verify before relying on them):` block.
- **Allow for this session:** `…for the rest of this session; later turns' recall includes it.` followed by the same block.
- **No hits or daemon unreachable:** `…No matching memories were found (or the memory service is not reachable from this session).`

Recall header: `Shared Brain context from prior work in this repo (historical hints; verify before relying on them):`. After a session grant it becomes `…from prior work in this repo and the memory access the user approved (…)`. The phrase `Shared Brain context` is kept so R60a's live grep check still works.

## Gaps

1. **Existing memories mostly lack attribution.** Spectral measured 1,002 of 1,049 chat turns with no wing (`session_wing.rs` module doc), and every harness turn written before this lane has none. Repo-scoped recall will be sparse until new harness turns build up wings. A backfill of old harness turns is possible from `chat_turn_wing_provenance` / the `Harness working directory:` tool text, but was not done here.
2. **Filtering happens after recall.** Spectral's wing match is a ranking boost, not a filter, so `retain_in_scope` cuts what `brain.recall` already returned. When other projects outrank this repo, a scoped search can come back short or empty. The real fix is a wing-constrained recall in Spectral.
3. **Unregistered repos get nothing.** A directory with no `projects.root_path` gets no memories. A headless harness seeds `root_path` for the project it resolves (`ensure_headless_project_root_path`), but only when that project's root is NULL, and only for the project its roadmap targets. The project is often `personal`, which is never a wing.
4. **Worktree edge case.** Worktree mapping covers `<main>/.git/worktrees/<name>`. A project rooted at a *subdirectory* of the main repo does not match from the same subdirectory of a linked worktree.
5. **Hint is once-only.** A harness session resumed in a different project directory keeps its first project hint. Its turns then fail corroboration and are stored without a wing. That is the safe direction, but those turns are lost to recall.
6. **Daemon-only recall paths are still unscoped.** These run only where `GLOBAL_BRAIN` is set (the daemon), never in the CLI harness: Council brief's `recall("what we were working on")` (`crates/goose/src/council/brief.rs:338`) and the `search_memory` tool. Orchestrator decompose recalls are already slug-scoped.
7. **No widening path outside the coding harness.** The request tool is registered only for coding-harness sessions, so other CLI sessions are repo-scoped with no way to widen (see Design 6).
8. **The frontend tool may be hidden in code-mode catalog disclosure.** In that mode `prepare_tools_and_prompt` filters out tools that are not first-class. Code mode is not the harness default.
