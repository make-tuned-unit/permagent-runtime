# Coding Harness Skill Guard DAG (X-skill)

Status: design — 2026-09-09. Owns Excellence DAG row "Safety and trust" (blocker: "unguarded
`run_executable_skill`", `CODING_HARNESS_EXCELLENCE_ITERATION_DAG.md:14`) and implements Part B of
`docs/architecture/PRIME_RLM_AND_SKILLS.md` verbatim. No code in this document; a code lane
implements it (PR 2, `feat/prime-executable-skills`, per that doc's Delivery section).

## Objective

Replace the ungated `run_executable_skill` tool with `skill_run`, executing only through the
approval ladder Permagent already has for completion-check commands, with a registry, an inputs
schema, and a receipt — no new safety subsystem.

```text
S0 freeze skill_run contract (inputs schema, registry row shape, receipt shape)
 ├── S1 registry: skills.toml -> `skills` table (no new table)
 └── S2 inputs-schema validation before any runner is chosen
          │
          ▼
 S3 approval ladder wire-in (verification_approval::decide, Decision Inbox proof)
          │
          ▼
 S4 sandbox posture (process isolation today -> minimal seatbelt/allowlist option)
          │
          ▼
 S5 receipts + RLM outcome (skill_executions row, rlm::set)
          │
          ▼
 S6 adversarial tests; delete `run_executable_skill`
```

## Gate ledger

| Node | Evidence required to pass |
|---|---|
| S0 | schema rejects a skill with no `verify`, no `runner`, or a path outside the skills root |
| S1 | a manifest round-trips through the `skills` table with no new schema object |
| S2 | inputs failing the manifest's JSON Schema never reach a runner |
| S3 | a `Tier::User` command parks a `PROPOSAL_CHECK_APPROVAL`-shaped decision instead of running |
| S4 | the sandbox posture change (if any) is documented and tested independently of the approval gate |
| S5 | every run, pass or fail, produces exactly one `skill_executions` row |
| S6 | all four adversarial recipes below are refused; `run_executable_skill` no longer exists |

## S0/S1 — The current unguarded path, traced exactly

| Step | File:line | What happens |
|---|---|---|
| Tool exposed | `crates/goose/src/agents/platform_extensions/orchestrator.rs:4887` (listed in `get_tools`) | `run_executable_skill` takes one argument, `name` — no `inputs`, no schema |
| Dispatch | `orchestrator.rs:5038` | `"run_executable_skill" => self.handle_run_executable_skill(arguments).await` |
| Handler | `orchestrator.rs:3793-3811` (`handle_run_executable_skill`) | Extracts `name`, calls `crate::executable_skills::run_named(&name)`, returns stdout/stderr/exit — **no approval call anywhere in this function** |
| Root lookup | `crates/goose/src/executable_skills.rs:45-54` (`skills_root`) | Resolves a filesystem dir (`$PWD/skills`, or `PERMAGENT_EXECUTABLE_SKILLS_ROOT`) — **not the `skills` DB table** (`session/spectral_schema.rs:335`); the executable path and the registered/reviewable skill are two different objects today |
| Path containment | `executable_skills.rs:109-149` (`resolve_skill_dir`) | Refuses absolute paths and `..` components, canonicalizes and checks `starts_with(root)` — this part is sound and is kept |
| Manifest | `executable_skills.rs:18-24` (`SkillManifest`) | Only `name`, `description`, `entrypoint` — **no inputs schema, no `verify`, no cost hint, no required tools** |
| **The gate** | `executable_skills.rs:164-172` | ```rust\nlet mut cmd = if looks_like_shell(&manifest.entrypoint) {\n    let mut c = Command::new(\"sh\"); c.arg(&entry); c\n} else { Command::new(&entry) };\ncmd.current_dir(&dir);\ncrate::subprocess::configure_subprocess(&mut cmd);\n``` — straight to `tokio::process::Command`. No `verification_approval::decide` call, no `Tier` check, no Decision Inbox card, no receipt row written before or after |
| Execution | `executable_skills.rs:174-190` | 30s timeout (`RUN_TIMEOUT`, `:16`), captures exit code + stdout/stderr, tries to parse stdout as JSON — returned straight to the model, no `skill_executions` row |

**Proof there is no approval or sandbox today:** grep for `verification_approval` and `decisions::`
inside `executable_skills.rs` returns nothing; the only two production call sites of
`command_gate::decide` are `goose-server/src/verification/checks.rs:401` (completion checks) and
nothing else. `run_named`/`run_skill` (`:152,198`) are reachable from the coding-harness recipe with
only a directory name — any entrypoint under `skills/` runs unattended, unlogged beyond a returned
tool result, every time the model calls the tool.

## S2/S3 — Part B spec (per `PRIME_RLM_AND_SKILLS.md`)

| Piece | Contract |
|---|---|
| Registry | The `skills` table already exists (`session/spectral_schema.rs:335`, served at `GET /permagent/skills`, `goose-server/src/routes/skills.rs:347-348`) carrying `name, description, definition_json, version, skill_path, status`. `skill.toml` (runner: `command`\|`recipe`\|`prompt`, `cost_hint`, `required_tools`, `[inputs]` JSON Schema, `[[verify]]`) is stored verbatim in `definition_json`. No new table. |
| Inputs schema | `inputs` validated against the manifest's JSON Schema before any runner is chosen; mismatch refuses before `Command` is ever built. |
| `verify` | Deserializes into `Vec<CompletionCheck>` — the same enum a goal's completion check uses (`goose-server/src/verification/checks.rs`); a skill's own `verify` result, not its exit code, decides pass. |
| **Approval** | For `runner = "command"`: call `verification_approval::decide(cmd, cwd, source, &mut settings, &cfg)` — the exact function already gating completion-check commands (`verification_approval/mod.rs:395-459`, `Tier::{Auto, AgentTrust, User}` at `verification_approval/command_gate.rs:42-47`, invoked in production only at `goose-server/src/verification/checks.rs:401`). A `Tier::User` outcome is `GateDecision::Parked` (`mod.rs:437-442`); `skill_run` returns "awaiting approval," not a result. |
| Receipt | One `skill_executions` row (`spectral_schema.rs:366`) per run — pass/fail, exit code, stdout/stderr tail, duration. Model-backed (`prompt`/`recipe`) skills additionally get a `cost_ledger` row via `append_cost_ledger` (`session_manager.rs:407`). |
| RLM outcome | `rlm::set(pool, scope, scope_id, "skill/{name}/last", {...})` under Part A's size cap, so a worker can read its own skill's last result back deterministically. |
| Scope | PR 2 ships `command` and `prompt` runners only; `recipe` is blocked on `decide_delegate_model` (branch `fix/delegate-honours-model-pins`, not on `main`) and returns "not yet available." |

## S3 — Approval ladder bound to the Decision Inbox proof pattern

A `Parked` outcome files a `PROPOSAL_CHECK_APPROVAL` card (`decisions.rs:75`, consumed at
`decisions_effects.rs:224` and `goose-server/src/verification/mod.rs:1202-1252`) — the same proposal
type completion checks already use; `skill_run` reuses it rather than minting a new proposal kind.

Two actors exist for *unattended* answers, and both are narrow, session-bound, single-use — the
pattern Lanes R/R2/V2 built for roadmap approval and landing:

| Actor | Where | Single-use proof shape |
|---|---|---|
| `ACTOR_JESSE` | `decisions.rs:22` | Ordinary human answer through the general `answer_decision*` path — no policy switch needed, always available |
| `ACTOR_AUTO_LAUNCH` | `decisions.rs:45` | `auto_approve_roadmap_launch` (`:2276-2420`) and its landing counterpart: only accepts a decision that is still `open` (atomic `UPDATE … WHERE status='open'`, `rows_affected()==0` = lost the race, `:2333-2343`), never touches `VALID_ACTORS` or the general tier gate (`:2278-2283`), and the *consuming* session independently re-checks it is still running the same unattended policy before accepting the answer — an auto-launch proof minted in one session can never be replayed into an interactive one (`:2288-2292`) |

`skill_run`'s `Tier::User` park must reuse `ACTOR_AUTO_LAUNCH`'s exact shape for an unattended
`command` skill: gated behind an explicit policy switch (mirroring `PERMAGENT_HEADLESS_AUTOLAND`,
default off, per the parent plan's X-land node), session-bound, single-use, and only for a decision
whose payload names the specific skill + resolved command — never a blanket "approve skills for this
session" grant. With the switch off (the default), every `Tier::User`/`AgentTrust`-untrusted skill
command parks for `ACTOR_JESSE` exactly like a completion check does today.

## S4 — Sandbox posture

`configure_subprocess` (`crates/goose/src/subprocess.rs:49-56`) today does exactly two things:
`command.process_group(0)` (unix — isolates from the parent's `SIGINT`) and, on Linux only,
`PR_SET_PDEATHSIG` so an orphaned child dies with its parent. **No filesystem, network, or privilege
restriction of any kind.** A minimal upgrade for `skill_run`, additive to the approval gate (not a
replacement for it — an approved-but-malicious skill must still be contained):

| Option | What it adds |
|---|---|
| macOS seatbelt profile (`sandbox-exec` wrapper) | Confines the child to the skill's own directory + declared `required_tools` paths; denies network unless the manifest declares it |
| Allowlisted-command mode | For `runner="command"`, refuse any manifest whose `command` is not built from a small allowlist of binaries (mirrors the completion-check ladder's own allowlist concept in `command_gate.rs`) rather than shelling out to an arbitrary entrypoint |
| Both are advisory hardening; the approval ladder (S3) is the actual gate — a sandboxed-but-unapproved skill must still park |

## S6 — Adversarial tests

| Recipe | Expected outcome |
|---|---|
| A recipe calls `skill_run` for a `command` skill with no prior approval, `PERMAGENT_HEADLESS_AUTOLAND`-style switch off | Parked (`GateDecision::Parked`); zero `skill_executions` row until answered |
| The same recipe forges a `DecisionProof`/`acted_by` value not `ACTOR_JESSE`/`ACTOR_AUTO_LAUNCH` | Refused — `VALID_ACTORS` (`decisions.rs:47`) rejects it before the ladder is even consulted |
| A `Tier::User` decision is answered once, then the recipe replays the same decision id for a second execution | Refused — the atomic `UPDATE … WHERE status='open'` pattern (`:2333-2343`) means the second attempt sees `status != 'open'` and gets `AlreadyResolved`, not a second run |
| A skill's manifest omits `[[verify]]` or its `verify` fails while exit code is 0 | The skill reports failure — exit code alone never decides pass (S0/S2 gate) |
| A skill path attempts `../../etc` or an absolute path | Already refused today (`resolve_skill_dir`, `executable_skills.rs:114-127`) — kept, regression-tested |

## Node order and effort

| Node | Depends on | Effort |
|---|---|---|
| S0 contract freeze | — | S |
| S1 registry (manifest → `skills` table, no new schema) | S0 | S |
| S2 inputs-schema validation | S0 | S |
| S3 approval-ladder wire-in + `ACTOR_AUTO_LAUNCH`-shaped skill proof | S0, S1 | M |
| S4 sandbox posture (seatbelt or allowlist option) | independent of S3 | M (macOS-specific work) |
| S5 receipts + RLM outcome | S1, S3 | S |
| S6 adversarial tests + delete `run_executable_skill` | S2, S3, S5 | S |

S0/S1/S2 can start in parallel; S4 is independent and can run concurrently with S3. S6 cannot land
before S3 exists, or the old ungated tool is deleted with nothing gating its replacement.
