# Prime RLM kernel and executable skills

**Date:** 2026-08-25 · **Status:** Part A implemented (PR 1); Part B queued (PR 2) · **Base:** `origin/main` @ `be058519`
**Closes:** the two rows marked **missing** in `PRIME_INTEGRATION_SEAMS.md`.

Both primitives already exist on main as **in-process prototypes**, and that doc's "Shipped" section already
calls them landed. It is wrong — neither is durable. `rlm.rs:19` is a process-local `DashMap` lost on restart,
persisted only as a whole-blob `rlm_state` key on goal metadata (`goal_a2a.rs:105`) — option (3), the racy one.
`executable_skills.rs:171` reaches `tokio::process::Command` with **no approval gate**, no receipt, no inputs
schema, no registry.

---

# Part A — RLM kernel

## Storage decision: `permagent.db`, new `rlm_context` table, mirrored to the Brain

| | Brain (`memory.db`) | **`permagent.db`** | goal metadata JSON |
| --- | --- | --- | --- |
| Exact read-back | no — recall is ranked | **yes** | yes |
| Transactional CAS | no | **yes** | no |
| In backup snapshot set | yes | **yes — `backup.rs:112`, `DbTarget::Spectral`** | yes (in `cards`) |
| WAL + checkpoint timer | yes | **yes — `wal_checkpoint.rs:108`** | yes |
| Versioned migrations | n/a | **yes — `spectral_schema.rs`, ladder ~v50** | n/a |
| Concurrent writers | n/a | **per-key CAS** | **last-writer-wins** |

**Chosen: `permagent.db`** — zero new durability work. I was asked to verify snapshot coverage and add it if
absent; **it is already covered**. Its ladder is explicitly additive and base-independent, so one more `CREATE
TABLE IF NOT EXISTS` is house style, and every other durable control-plane object — cards, projects, decisions,
`cost_ledger` — already lives there. One concept, one place.

**Rejected — Brain as primary.** Recall is ranked, not exact; a control plane that cannot guarantee reading back
the cell it just wrote is the wrong tool. Every write goes through ingest + fingerprinting + graph linking on a
blocking thread (`brain_handle.rs:465-527`), and fingerprint dedupe silently drops an identical re-write.

**Rejected — goal metadata JSON.** `persist_rlm_snapshot` (`goal_a2a.rs:105-122`) reads the whole
`metadata_json` blob, mutates one key, writes it back with no version guard. Any concurrent write to
`attempt_count`, `last_error`, or `worktree_path` between the read and the `UPDATE` is silently lost. A live bug
this retires.

**Brain mirror (write-through).** On each *version change*, best-effort
`brain.remember_with("rlm/{scope}/{scope_id}/{key}", summary, opts)` — state becomes recallable in conversation
and outlives the goal, without putting the Brain on the read path. Idempotent by construction: the Brain returns
`WriteOutcome::Inserted` only for genuinely new content — exactly the assertion for the "mirrored once per key
change" test. A mirror failure logs, never fails the SQLite write.

## Schema (ladder step v52, additive + base-independent)

```sql
CREATE TABLE IF NOT EXISTS rlm_context (
  scope TEXT NOT NULL, scope_id TEXT NOT NULL, key TEXT NOT NULL,   -- scope: 'session'|'goal'
  value_json TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, expires_at TEXT,  -- NULL = no TTL
  PRIMARY KEY (scope, scope_id, key));
CREATE INDEX IF NOT EXISTS idx_rlm_context_expiry
  ON rlm_context(expires_at) WHERE expires_at IS NOT NULL;
```

## Semantics

**Concurrency — optimistic version check.** `set` takes `expected_version: Option<i64>`. `Some(v)` compiles to
one `UPDATE … SET version = version + 1 WHERE scope=? AND scope_id=? AND key=? AND version=?`; `rows_affected()
== 0` returns `RlmError::VersionConflict`. A single SQLite `UPDATE` is atomic, so the CAS needs no explicit
transaction; `set_many` wraps a batch in one. `None` is a deliberate blind upsert for first writes. A conflict is
**never** resolved by overwriting. The row commits before the tool returns and reads are read-through, so restart
recovery is just querying the table — no rebuild step.

**The sync-signature problem.** `retry_context_block` (`dispatch_brief.rs:20`) is sync and cannot `await`.
Rather than churn every caller, the `DashMap` survives as a **read-through cache**: a new `async fn
hydrate(pool, scope, scope_id)` loads the namespace, and the async dispatch/resume path calls it before building
the brief, so `retry_context_block` and `quoted_brief_block` keep their signatures. Writes are SQLite-first,
then cache — a failed write can never read back as success. `hydrate_from_metadata` stays as a one-shot legacy
import for cards still carrying `rlm_state`.

**TTL / GC and size limits.** `expires_at`; reads filter `expires_at IS NULL OR expires_at > now`. The sweep
piggybacks the existing hourly `wal_checkpoint` timer rather than adding a second loop. Default: no TTL for
`goal` scope, 30 days for `session`; on goal Complete/Cancelled the namespace is deleted *after* the mirror
lands. Caps are 64 KiB per value, 256 keys and 1 MiB per namespace; over-limit writes are **refused**, never
truncated — a truncated JSON cell is a corrupt cell that reads back as valid-looking garbage.
`quoted_brief_block` gains a render cap.

**Privacy — detect and refuse, do not redact.** `crate::privacy::redact` (`privacy.rs:46`) is the repo's single
source of secret patterns, but calling it here would be actively wrong: it also scrubs `/Users/…` paths and
UUIDs, precisely what a worker stores (worktree pointers, goal ids). The hook reuses only the credential-shaped
subset (`sk-`, `pk-`, `bearer …`, `api_key|token|password`) and **refuses** the write. Storing a
silently-redacted value under a key the model reads back is a lie it cannot detect.

## Tool surface (orchestrator, beside `steer_goal`/`message_goal`)

| Tool | Params | Returns |
| --- | --- | --- |
| `context_set` | `key`, `value`, `scope?="session"`, `expected_version?`, `ttl_secs?` | `{key, version}` |
| `context_get` | `key`, `scope?` | `{key, value, version, updated_at}` \| not-found |
| `context_list` | `scope?`, `prefix?` | keys + versions (values bounded) |
| `context_delete` | `key`, `scope?` | `{deleted}` |

Scope defaults to the calling session. `scope: "goal"` is opt-in and requires the session be bound to a goal
card; otherwise refused with the reason.

## Worker-side API (`crates/goose/src/rlm.rs`) and the A2A seam

```rust
pub async fn get(pool, scope: Scope, scope_id: &str, key: &str) -> Result<Option<Cell>, RlmError>;
pub async fn set(pool, scope, scope_id, key, value: Value, opts: SetOpts) -> Result<Cell, RlmError>;
pub async fn list(pool, scope, scope_id) -> Result<BTreeMap<String, Cell>, RlmError>;
pub async fn delete(pool, scope, scope_id, key) -> Result<bool, RlmError>;
pub async fn hydrate(pool, scope, scope_id) -> Result<(), RlmError>;  // fills cache before a brief
pub fn quoted_brief_block(session_key: &str) -> Option<String>;       // unchanged, sync

/// A2A seam — the sibling worker calls ONLY this; it must not touch `rlm_context` or the metadata
/// blob. Bounded ring of the last 8 messages, so two senders cannot clobber each other. Mirrored to
/// the Brain. `goal_a2a.rs:78-80` + `persist_rlm_snapshot` go away.
pub async fn write_a2a_feedback(
    pool: &Pool<Sqlite>, to_goal: &str, message: &serde_json::Value,
) -> Result<Cell, RlmError>;
```

---

# Part B — Executable skills

## Today there are three "skills", not one

**(1) DB rows** — `skills` + `skill_executions` (`spectral_schema.rs:317,348`), served at `GET
/permagent/skills` (`routes/skills.rs:142`), shown by the UI Skills panel. **(2) Markdown `SKILL.md` folders** —
from `~/.permagent/skills` and six other roots (`platform_extensions/skills.rs:147-190`), loaded by
`load_skill`. **(3) Executable packages** — the `skills/` dir, run by `run_executable_skill`, in no registry.

"One concept, one place" means **(1) is the registry**: it already carries `name, description, definition_json,
version, skill_path, status`, and `skill_executions` already carries `input_json, output_json, error_message,
status, started_at, completed_at` — i.e. it is already a receipt table, already exposed at
`/permagent/skills/{id}/executions`. No new table, no new UI section.

## Manifest (`skill.toml`, stored verbatim in `skills.definition_json`)

```toml
name = "rustfmt-check"; version = "1.0.0"; runner = "command"  # runner: command | recipe | prompt
description = "Fail if any tracked Rust file is unformatted."
command = "cargo fmt --all -- --check"
cost_hint = "free"                     # free | local | cloud-cheap | cloud-expensive
required_tools = ["developer__shell"]
[inputs]                               # JSON Schema, validated before the runner is chosen
type = "object"; properties = { path = { type = "string" } }
[[verify]]                             # exactly the #1091 CompletionCheck shape
type = "command_exit_zero"; cmd = "cargo fmt --all -- --check"; expect = "/^$/"
```

`verify` deserializes into `Vec<CompletionCheck>` (`verification/checks.rs:28-70`) — the same enum, `expect` and
all, so a skill's contract and a goal's completion checks are one type. A skill that passes its own `verify` is
the only skill allowed to report success.

## `skill_run` — one tool, executing through the existing gates

`skill_run { name, inputs? }` replaces `run_executable_skill` (deleted, not kept alongside):

1. Resolve `name` in the `skills` table → `skill_path`; re-check path containment as today
   (`executable_skills.rs:104-146`); absolute and `..` paths refused.
2. Validate `inputs` against the manifest's JSON Schema; refuse on mismatch.
3. **Approval.** For `runner = "command"`, call `verification_approval::decide(cmd, cwd, source, &mut
   settings, &cfg)` (`verification_approval/mod.rs:395`) — the same ladder that gates completion-check commands
   (`Tier::{Auto, AgentTrust, User}`). A `Parked` outcome files a `PROPOSAL_CHECK_APPROVAL` card
   (`decisions.rs:53`) and `skill_run` returns "awaiting approval" rather than executing. **This is the hole
   today**: `run_executable_skill` reaches `Command` with no gate at all.
4. Execute; capture exit code, stdout/stderr tail (8 KiB each), duration.
5. Run the manifest's `verify` checks; their result — not exit code alone — decides pass.
6. **Receipt** → one `skill_executions` row. Model-backed skills additionally get their per-call
   `cost_ledger` row via `append_cost_ledger` (`session_manager.rs:407`); that table is per-LLM-call, so it is the
   wrong home for a shell skill's receipt and the right one for a model skill's cost.
7. **Outcome → RLM.** `rlm::set(pool, scope, scope_id, "skill/{name}/last", {...})` with pass/fail,
   exit code, stdout tail, and receipt id, under Part A's size cap.

**Blocker, flagged honestly:** the "#1125 delegate precedence" my brief cites for model-backed skills is **not
on main** — `decide_delegate_model` lives only on the unmerged branch `fix/delegate-honours-model-pins`. PR 2
therefore ships `command` and `prompt` runners only; `recipe` returns "not yet available" behind a one-line seam
calling `delegate_routing(...)` once that merges.

## Migrating a prompt-only skill (the worked example)

A markdown skill migrates by gaining a wrapper manifest, not by being rewritten: a `skills` row with `runner =
"prompt"`, `skill_path` = the existing `SKILL.md` dir, `definition_json` derived from the `SkillMdMeta`
front-matter (`skill_md.rs:85-92`). `skill_run` on a `prompt` skill returns the rendered body — identical to
`load_skill` today — so the two agree and `load_skill` becomes a thin alias.

---

# Delivery

**PR 1 `feat/prime-rlm-kernel`** — v52 migration; `rlm.rs` over SQLite with the DashMap demoted to a read-through
cache; `context_*` tools; `write_a2a_feedback`; TTL sweep on the WAL timer. Tests: survives a simulated restart
(drop cache, re-read from a reopened pool); version-conflict rejection; TTL GC deletes only expired rows; mirror
fires once per version change (assert `WriteOutcome`); a credential-shaped value is refused and no row lands.

**PR 2 `feat/prime-executable-skills`** (fresh branch off main after PR 1 merges) — manifest + schema validation;
registry rows in `skills`; `skill_run` through the approval ladder; receipts in `skill_executions`; outcomes into
RLM; one prompt skill migrated, plus `skills/examples/hello-json`. Tests: manifest round-trip; a `User`-tier
command parks instead of running; `verify` failure fails the skill despite exit 0; receipt row written.

Seam-inventory rows flip to **done** with `file:line` at the end of each PR.
