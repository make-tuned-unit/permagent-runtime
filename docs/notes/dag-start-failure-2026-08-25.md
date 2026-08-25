# The DAG that never started — session 20260825_3, 2026-08-25

Jesse asked the agent (MiniMax-M2.7) to name a new project, create it, then set up a
multi-step DAG for Claude Code to build. The project was created; the DAG was never
started. From `~/.permagent/spectral/permagent.db` (`messages` 15944–15965); no chat
text is quoted here.

## What happened, in tool calls
1. `project_list {}` → success
2. `project_create {"name":"CivicLedger","root_path":null,"description":null,"tags":[…]}` → success
3. `shell {"command":"mkdir -p …/civic-ledger"}` → exit 0
4. `project_update {"id_or_slug":"civicledger","root_path":null}` → **success**, `root_path: null`
5-6. `project_update`, identical args, twice → `BLOCKED by the runaway-loop guard: …
   with these exact arguments just ran and produced the same result.` Session ends.
   No DAG tool was ever called. The capability is not missing — `create_roadmap`
   + `decompose_roadmap` are exactly it — the flow died two steps short of them.

## Root cause — a product bug (1), with model behaviour (2) on top

**Product bug.** `project_update` took a change-set in which every field was absent
or already equal, changed nothing, and answered `Updated project "CivicLedger"`
with `root_path` still null. A success reply that is not a success leaves a model
no way to correct itself, so it retried identically. The guard was the only thing
that noticed.

**Model behaviour.** The intent was to set the path — the model's own reasoning says
so between calls — but it emitted `root_path: null` three times. `null` on this
argument means "clear the field" (`Option<Option<String>>`), so nothing moved.

**Not the guard.** `tool_monitor` keys on `(name, args, result_hash)` — it already
counts distinct argument sets and fires only on exact repeats with an unchanged
result. It behaved correctly and preserved the work. **Not infra:** no provider
4xx/5xx and no `127.0.0.1:8080` traffic here; the split's six deaths are separate.

## Fix

`handle_update` now refuses an update that would change nothing. The refusal names
every field passed and the value it already held ("root_path was already null"),
says the identical retry will not help, and explains that `null` clears rather than
sets (`project_manager.rs`, `no_op_update_reason`). The tool description agrees.
