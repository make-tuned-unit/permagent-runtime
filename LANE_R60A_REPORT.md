# Lane R60a — hermetic eval runs: no operator Brain recall

Branch `lane/R60a` off `6a821677`. Edit-only lane: nothing was compiled or run here; the coordinator gates compiles.

## Defect

Record r60 s1, fixture `select-checks-worker-before-dispatch`: the logged request
(`laneM17/dschat-r60/s1-runs/dschat-select-checks-worker-before-dispatch-*/data/logs/llm_request.0.jsonl`)
contains a `Shared Brain context from Chat and prior Harness work` block under
`# Turn-specific Instructions`, filled with the operator's real history (the "DAG Wave Sequencer" note, and
"Cursor probes as **not available**"). The model believed the stale Cursor note, escalated instead of
dispatching, and failed the fixture. The same private text was sent to the eval provider (DeepSeek).

## Root cause

- The block is rendered by `crates/goose-cli/src/session/brain_sync.rs:219` (`render_recall_context`),
  fetched by `recall_context` (`:151`), which the CLI session calls before every turn
  (`crates/goose-cli/src/session/mod.rs:1876`).
- No store is opened in the CLI process. It queries the **live daemon** over loopback: `endpoint()`
  (`brain_sync.rs:120-128`) runs `GET http://127.0.0.1:<port>/api/brain/search` with the daemon's bearer
  token. The daemon owns the Brain, so the recall comes from the daemon's store (the operator's
  `~/.permagent`), not from the run's `<run>/data/spectral/permagent.db`.
- The token and port came from `crates/goose-cli/src/commands/daemon.rs`: `load_daemon_token` joined
  `dirs::home_dir()/.permagent/secrets/daemon_token.json`, and `config_path` (used by `read_daemon_port`)
  joined `dirs::home_dir()/.permagent/config.yaml`. Both skipped `PERMAGENT_PATH_ROOT`.
- The eval runner isolates each run only with `PERMAGENT_PATH_ROOT=<run>/data`
  (`crates/permagent-eval/src/invocation.rs:182`) and leaves `HOME` as it is. So every eval child found the
  operator's real token, reached the operator's live daemon, and pulled their Brain into its prompt.
- The daemon already writes its token under the root, at `Paths::data_dir()/secrets`
  (`crates/goose-server/src/state.rs:1358`), and `permagent doctor` already reads it there
  (`crates/goose-cli/src/commands/doctor/checks.rs:282`). Only this reader ignored the root.
- The same credentials also let the eval child write back into the operator's daemon:
  - `brain_sync::persist_turn` (`brain_sync.rs:299`, called at `session/mod.rs:2339`) saved eval turns as
    Brain memories.
  - `spend_announce.rs:784-785` and `:804-805` posted eval spend and harness runs into the operator's cost
    and run views.

## Fix (root, no kill switch)

`crates/goose-cli/src/commands/daemon.rs`:
- `config_path()` (`:33`) now returns `Paths::in_config_dir("config.yaml")`.
- New `daemon_token_path()` (`:47`) returns `Paths::in_data_dir("secrets").join("daemon_token.json")`.
  `load_daemon_token` (`:76`) uses it.

Without a root, both paths are exactly the same as before (`~/.permagent/...`), so shared-brain recall in
normal Chat and Harness sessions is unchanged. With a fresh root there is no token, so `endpoint()` fails.
Recall, turn persistence and spend announce were already built to stay silent when that happens, so they
now do nothing. A process still reaches a daemon that belongs to its own root. The daemon is effectively
rooted through its token, so no env switch was needed and the runner exports nothing new.

The CLI's other token users (`commands/activity.rs`, the doctor checks, `daemon_ws_url`) now follow the root
too. Doctor was already half-rooted, so this makes it consistent.

## Live Status and persona roster (item 3): rooted already, no fix needed

In the r60 prompt this block reads: Librarian `0 described, 0 pending`, both flags off, no briefings. Those
are fresh-root values, not the operator's live state. Sources:
- **Dispatchable workers roster:** `load_agent_config()` reads `Paths::config_dir()/agent.yaml`
  (`crates/goose/src/config/agent_identity.rs:1126`), plus built-in defaults. The status for each worker
  comes from `ProbeCache::probe`: a binary on `PATH`, or local model HTTP. That reads machine state but
  carries no private content. This is why it says "Cursor — available" (the binary is on the bench
  machine's PATH).
- **Background worker status:** `worker_live_state_for` (`crates/goose/src/agents/self_knowledge/mod.rs:283`).
  - Librarian counts come from the in-process `LIBRARIAN_STATE` static (`librarian_state.rs:101-106`).
  - The scheduler count comes from the in-process scheduler service.
  - Initiative, Steward and strix read `FeatureFlags` from `Config::global()`, which is rooted through `Paths`.
- **Briefings:** `briefings::unacknowledged` on `session_manager.pool_clone()`, which is the run's rooted
  session DB (`crates/goose/src/agents/reply_parts.rs:1178`).

## Tests (edit-only; not yet compiled)

1. `crates/goose-cli/src/session/brain_sync.rs:719` —
   `a_path_rooted_process_never_resolves_the_operators_daemon` (`#[tokio::test]`,
   `#[serial_test::serial]`, `env_lock`).
   - Sets `HOME` to a decoy operator home that holds a token and `port: 4999`, and `PERMAGENT_PATH_ROOT`
     to a separate fresh temp root.
   - Asserts that `endpoint()` returns an error and `recall_context()` returns `None`. The old code would
     have returned the decoy token.
   - Then writes a token and `port: 4123` into the root and asserts `endpoint()` uses both, so a daemon
     that belongs to the root is still reachable.
2. `crates/permagent-eval/src/runner.rs:2603` — `run_task_hands_the_child_a_root_with_no_daemon_credential`.
   - Runs `run_task` with a harness stand-in that, at spawn time, inspects the `PERMAGENT_PATH_ROOT` the
     child actually receives.
   - Asserts the root exists, sits inside `runs_root`, and holds no `secrets/daemon_token.json` and no
     `config.yaml`.
   - This guards the one thing the fix relies on: nobody may later seed a credential or the operator
     config into the run root.

### Gate filters

```
cargo test -p permagent-cli --lib brain_sync
cargo test -p permagent-eval --lib run_task_hands_the_child_a_root_with_no_daemon_credential
cargo test -p permagent-eval --lib env_policy
```

(`brain_sync` is in `permagent-cli`'s lib target (`crates/goose-cli/src/lib.rs:13`), not in the `permagent`
crate. Nothing in `permagent --lib` changed.) Suggested extra check:
`cargo clippy -p permagent-cli -p permagent-eval --all-targets -- -D warnings`.

Live verification after the rebuild: rerun `select-checks-worker-before-dispatch` on the dschat tier. Then
`grep -c "Shared Brain context" <run>/data/logs/llm_request.*.jsonl` should print 0, and no new
`/api/coding-sessions/turn` or `/spend` entries should show up in the operator daemon log for that run.

## Other non-hermetic reads found (not fixed in this lane)

The eval runner (item 5):
- `runner.rs:702-722` `seed_custom_providers` copies `$HOME/.permagent/custom_providers/` into the run
  root. This is deliberate and documented: they are provider declarations, not sessions or secrets. Keep it.
- `main.rs:943-971` `daemon_port()` and `daemon_token()` read `$HOME/.permagent` for `post_program_handoff`
  (`/api/program/handoff`). This is an operator-invoked program-DAG command, not the per-run child, so
  talking to the live daemon is intended. It is still a second hand-rolled copy of the old HOME join. If
  the eval process itself is ever run under a root, it should share the rooted resolver.
- `graduate.rs:982` already honours `PERMAGENT_PATH_ROOT`.
- Keychain use under `--use-keyring` is for provider keys only, which is expected.

In the harness itself: these read HOME directly and are reachable from an eval child, but are not seen in
the r60 prompt:
- `crates/goose/src/agents/platform_extensions/skills.rs:150-166` `discover_skills` scans
  `~/.agents/skills`, `~/.claude/skills` and `~/.config/agents/skills` alongside the rooted
  `Paths::config_dir()/skills`. If the skills extension surfaces the list or content, the operator's own
  skills (names, descriptions, bodies) would reach the eval model. There were no operator skill names in
  the r60 s1 request. This is the most likely next leak. Suggested fix: when `PERMAGENT_PATH_ROOT` is set,
  skip the HOME-anchored global dirs.
- `crates/goose/src/agents/platform_extensions/librarian_entities.rs:128-135` `ledger_path` is
  `$HOME/.permagent/librarian-entity-state.json`. A run that invokes the Librarian's entity tools would
  read and write the operator's ledger.
- `crates/goose/src/agents/platform_extensions/project_manager.rs:1807-1811`: the document root for
  project docs is `$HOME/.permagent/project-docs/<id>`. Project ids come from the rooted DB, so a collision
  is unlikely, but the path is not rooted.
- `crates/goose/src/scheduler.rs:2621-2623` (automation findings dir) and `crates/goose/src/polybot.rs:310,625`
  are HOME-anchored. They are daemon or tool paths that coding-harness eval fixtures don't use.
- `crates/goose-cli/src/commands/daemon.rs:22-25` `logs_dir()` is still HOME-anchored on purpose: it
  matches the launchd plist that `permagent start` writes.

Side observation: the second recalled hit in r60 was a "User: … / Assistant: …" row that passed the
`source == "memory"` filter (`brain_sync.rs:230-234`). Harness turns saved through `persist_turn` come back
as memory rows. Before this fix, eval runs were feeding that same store.
