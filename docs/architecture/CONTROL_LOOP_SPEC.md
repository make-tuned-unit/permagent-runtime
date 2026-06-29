# The Henry↔CC Control Loop — Architecture Spec (Epic #399)

**Status:** authoritative design baseline. Read-only audit + Jesse's rulings (this session).
**Scope:** Henry launches a Claude Code (CC) session in a visible terminal tab, hands it a goal,
monitors it (and other concurrent sessions) at **zero steady-state cost**, escalates gates it can't
auto-clear to the Decision Inbox, and relays Jesse's answer back into that specific session.

> Cost is a first-class constraint: **no polling, no LLM-in-the-loop monitoring, no raw-output
> firehose.** Henry is invoked a handful of times per multi-hour session, at human-meaningful
> moments only. Every decision below flows from that.

---

## The keystone finding

`claude` already speaks a **deterministic, bidirectional, structured gate protocol** over
`--output-format stream-json`, and it is **fully implemented and tested in this repo today**
(`crates/goose/src/providers/claude_code.rs`). A gate is a single NDJSON line:

```json
{"type":"control_request","request_id":"perm_1",
 "request":{"subtype":"can_use_tool","tool_name":"Write",
            "input":{"path":"foo.txt","content":"hello"},"tool_use_id":"tu_1"}}
```

and an answer is a single NDJSON line written back to stdin (`claude_code.rs:860-901`):

```json
{"type":"control_response","response":{"subtype":"success","request_id":"perm_1",
 "response":{"behavior":"allow","updatedInput":{...},"toolUseID":"tu_1"}}}
```

This is **push, deterministic, zero-LLM, zero-poll** — it satisfies the cost constraint by
construction. We prefer it over reading CC's TUI everywhere.

---

## The five subsystems

### PIECE 1 — LAUNCH (visible tab + capture)

Henry can already **launch-and-own a visible session.** `project_launch` (PR #409,
`crates/goose/src/agents/platform_extensions/project_manager.rs:387`) emits a `ProjectLaunch` event
→ `ui/command-center/src/hooks/useAppNavigate.ts:103` → `BuildView.tsx:34` →
`TerminalManager.createProjectTab` → `ui/desktop/src-tauri/src/terminal.rs:48 spawn_pty_session`
(a real PTY the app owns, keyed `pty-<uuid>`), and the initial command is injected via
`write_to_pty(session_id, "claude\n")` (`Terminal.tsx:136`). The tool already advertises
`command:"claude"` in its schema.

But that visible session runs `claude` as the **interactive TUI**, and the PTY reader
(`terminal.rs:125-182`) emits raw bytes as `pty_data` to the webview only — **no parse, no tee,
no structured events.** The reader thread *is* the natural tee point (every output byte passes
through it in Rust), but nothing parses there today.

Separately, `ClaudeCodeProvider` (`claude_code.rs:333`) runs
`claude --input-format stream-json --output-format stream-json --verbose` as a **headless
subprocess** (no PTY, not visible) — this is where the structured protocol lives.

➡️ The gap is not "can Henry launch CC" (yes) — it's that the visible path and the structured
path are two different launches. Unifying them is the central design move (Fork 1).

### PIECE 2 — GATE PARSER (the heart)

**(a) stream-json:** reliable. `control_request`/`can_use_tool` carries
`{request_id, tool_name, input, tool_use_id}` — everything needed for `{session, question,
options}`. Enabled by `--permission-prompt-tool stdio` (`claude_code.rs:350-365`, gated on goose
mode). The parser, structs, and tests already exist (`claude_code.rs:119-135`, `:793-915`, tests
`:1355-1438`).
*Caveat:* stream-json events do **not** carry `session_id` — it's implicit in the pipe/process.
For a visible-PTY tee, the **PTY `pty-<uuid>` is the correlation key** (one tab = one process).

**(b) TUI:** no structured events. Detecting a gate means parsing ANSI/prompt text out of the
`pty_data` firehose — fragile, can't reliably extract the question, and to do it *meaningfully*
you'd need an LLM reading output (violates cost) or brittle regex.

➡️ **Recommendation (RULED): stream-json structured gates.** The only option that yields reliable
`{question, tool, options}` events **and** costs zero in steady state. The parser lives in Rust,
consuming the tee from Piece 1, emitting a gate event to the existing `PermagentEvent`/activity bus.

### PIECE 3 — SESSION REGISTRY / MULTIPLEX

Two **unconnected** registries today:
- **PTY-side:** `PtySessions(Mutex<HashMap<String, PtySession>>)` (`terminal.rs:15`), keyed
  `pty-<uuid>`, addressable for writes via `write_to_pty`. This is the visible, multiplexed,
  per-tab registry — exactly what N concurrent sessions need.
- **Provider-side:** a single `OnceCell<Arc<Mutex<CliProcess>>>` per provider
  (`claude_code.rs:269`) — **not** per-session-keyed; `session_id` is only a payload field.
- **Goal-side:** orchestrator cards carry `worker_session_id` in metadata (`orchestrator.rs:769`)
  — tracks dispatched *goals*, not PTYs.

➡️ For the loop, the **PTY `pty-<uuid>` is the natural addressing key** (gate→inbox→relay all
reference it). What's missing is a thin **session registry tying
`{pty_session_id → project_id, goal_id?, gate request_id}`** so a gate routes to the right inbox
row/project and an answer routes back to the right PTY. Small new structure, not a rebuild.

### PIECE 4 — GATE → DECISION INBOX BRIDGE

The `decisions` table (`crates/goose/src/session/spectral_schema.rs:1019`) already has everything
except a gate kind:
- `kind CHECK IN ('approve_review','unblock','choice','risk_gate','malformed')` — **needs a new
  `session_gate` value** (a CHECK-constraint migration).
- `project_id` and `goal_id` columns already exist → project/goal association is free.
- `payload_json` is untyped TEXT → carries `{question, target_session_id, request_id, tool_name,
  input, options}` with **no new columns**.
- `answer` / `answer_choice_id` / `answer_input` / `acted_by` already model the response.

**Context-load is already built and reusable verbatim:** the discuss-with-persona seam (#303,
`crates/goose-server/src/routes/session_events.rs:609`) hydrates a decision into Henry's chat via
`extend_system_prompt("discuss_decision", …)` keyed by `app_context.view_state.discuss_decision_id`,
loading the decision authoritatively from the DB. Clicking the gate decision → Henry opens already
knowing the session's gate. No new context-load path needed.

**Answer path:** `POST /api/decisions/{id}/answer` → `answer_decision` (tier gate) → `execute_effect`
(`crates/goose-server/src/routes/decisions.rs:122`, `crates/goose/src/decisions.rs:576`). A
`session_gate` decision needs a **new effect arm**: relay the answer to `target_session_id`
(the L3 hook).

### PIECE 5 — RELAY BACK (L3, the supervision boundary)

**MECHANISM (buildable now, deterministic, no LLM):**
- Write the `control_response` NDJSON to the target session's stdin. For the
  visible-PTY-stream-json design, that's `write_to_pty(pty_session_id, control_response + "\n")`
  (`terminal.rs:217`). For the headless provider, `claude_code.rs:893-900` already does exactly this
  via a oneshot channel.
- ⚠️ `write_to_pty` has **no auth/ownership guard** — anything knowing a `session_id` can write to
  that stdin. The relay path makes this security-relevant (Fork 3 / S5 adds a guard).

**SAFETY GATE — already exists and is load-bearing:**
- `risk_policy` table (`spectral_schema.rs:1109`) maps `action_class → tier (0/1/2)`; unknown ⇒
  **Tier 2 fail-closed** (`decisions.rs:190`).
- `answer_decision` enforces it: **Tier 2 ⇒ `acted_by='jesse'` only**, Tier 1 ⇒ `jesse` or
  `henry-policy`, else `Forbidden` (`decisions.rs:610-625`).
- Henry **already auto-clears Tier-1** decisions today via `henry_approve_on_verifier_pass`
  (`crates/goose/src/decision_inbox/policy.rs:36`, `acted_by="henry-policy"`), with an end-to-end
  test proving Tier-2-via-Henry is rejected (`policy.rs:253`).

➡️ **MECHANISM is buildable now. POLICY is Jesse's** (Fork 2): the new decision is *which CC
tools/inputs map to which `action_class`/tier*. The irreversibility surface (a relayed "allow" to
`Bash: rm -rf` or `git push`) is governed by that mapping — and the fail-closed-to-Tier-2 default
protects everything unmapped by default.

---

## Cost profile (the target this architecture meets)

| Approach | Reliable `{q,opts}`? | Steady-state cost | Poll? | LLM-in-loop? |
|---|---|---|---|---|
| **stream-json `control_request`** | ✅ deterministic | **0** (parser idle until a line arrives) | ❌ push | ❌ |
| TUI output parsing | ❌ fragile | high (continuous read) | — | ✅ or brittle regex |
| #423 quiescence (ECHO+1500ms) | ❌ "went quiet", no question | per-active-session timer | ✅ **it's a poll** | ❌ |

- **launch** = **1** Henry call (compose the goal prompt)
- **hours of CC work** = **0** Henry cost (Rust parser only; idle until a `control_request` arrives)
- **each gate** = **1** bus event + **1** inbox row + **0-or-1** Henry call (0 when a deterministic
  tier/policy mapping auto-clears or escalates; 1 only when a Tier-1 genuinely needs Henry reasoning)
- **answer + relay** = deterministic write

**Where the LLM is, auditable from the slicing:** exactly two places — **S1 launch** (compose goal)
and **S3 gate** (0-or-1, only Tier-1-needs-reasoning). S2/S4/S5/S6 are LLM-free.

---

## Project-state memory overlay (Henry accretes per-project state)

Henry must accrete project-state knowledge from the CC work he oversees — but he **enqueues**
memories to the **Librarian** (the Brain-write specialist), he does not write Brain directly, and
memory-creation must be **deterministic and milestone-driven**, riding the loop's existing discrete
events. Steady-state Henry-LLM cost for memory = **0**.

1. **Which events → a memory:** the loop's discrete milestones — **session-launched**,
   **gate-resolved** (with the answer), **session-completed/exited**. Not per-turn, not per-chunk.
2. **Librarian enqueue path — DOES NOT EXIST YET (blocker).** The Librarian is a **scheduled batch
   runner** draining `brain.list_undescribed()` (`crates/goose/src/agents/platform_extensions/librarian.rs:566`,
   `scheduling.rs:324`) — there is **no work-queue to enqueue into**, and the CRM People
   Librarian-enrichment cited as the precedent is itself **deferred to v1.5, not built**
   (`crates/goose/src/people.rs:19-23`). Honoring "Henry orchestrates, rarely does" requires building
   a `librarian_work_queue` + enqueue fn + a drain pass in `run_batch`.
3. **project_id on Brain memories — THE GATING BLOCKER.** The `memories` table has **no
   `project_id`** and `recognition_events` has none either (`spectral_schema.rs:194`, `:832`).
   **Epic #70 is CLOSED but only propagated `project_id` to the app-DB tables (cards / decisions /
   inbox_files) — it did NOT reach the Spectral Brain memory tables.** So this is a **new dependency**
   (project_id on memories + recall), not "wait for #70". Without it, "state of project P" cannot be
   a filtered recall.
4. **Recall by project — also blocked.** `recall`/`recall_cascade` take only `{query,
   visibility/context}` (`crates/goose/src/brain_handle.rs:117,134`) — no project filter.
5. **Slicing:** a **separate slice** (S6), a deterministic memory-consumer of the loop's milestone
   events → new Librarian queue, hard-gated on the project_id-on-Brain-memories dependency. Do not
   weave into the inbox-bridge slice.

---

## Build slicing (dependency order; LLM-in-loop marked)

**Deterministic plumbing (plain code, no LLM, cheap) — the spine:**

- **S0 — #424 reconciliation + #423 trim — DONE.** Findings in the "S0 Reconciliation Findings"
  section above: #424 is Case (b)-degenerate → S1 is a sibling engine; bridge registry required;
  #423 trimmed to exit-only.
- **S1 — Launch a stream-json CC session in a visible PTY tab** *(Piece 1; depends S0).* A new
  **sibling** `GoalEngine` (`SupervisedCliEngine`) — NOT an extension of #424. Reuses #424
  scaffolding + `claude_code.rs` protocol structs; runs `claude --output-format stream-json
  --input-format stream-json --verbose --permission-prompt-tool stdio` (no `--dangerously-skip-permissions`)
  in a Build-tab PTY, stdin writable for relay, stdout streamed/tee'd. Supports **both** dispatched-goal
  and free-standing watched entry points. **LLM: 1 call at launch.**
- **S2 — Gate parser + session registry** *(Pieces 2+3; pure Rust, no LLM; depends S1).* Tee
  `pty_data` → NDJSON parser → `control_request` → gate event on the bus; thin
  `{session→project,goal,request_id}` registry. **Cost: 0 steady-state.**
- **S3 — Gate → inbox bridge** *(Piece 4; depends S2).* New `session_gate` kind (CHECK migration),
  `payload_json` carries question+target_session_id+options, reuse the `discuss_decision`
  context-load. **LLM: 0–1 per gate.**
- **S4 — Classification + tier mapping** *(depends S3; needs Jesse's Fork-2 policy).* Map CC
  `tool_name`+`input` → `action_class` → tier, fail-closed to Tier 2.

**Gated-last (the supervision boundary):**

- **S5 — L3 RELAY** *(Piece 5; gated on Forks 2+3 / #401/#402; depends S4).* The relay channel
  carries **two** NDJSON message types into the session's stdin: **`control_response`** (gate answers,
  gated by policy) and **user-input** (steering — "tell Henry to update the session"; also powers the
  S7 Tier-1 escape hatch). New `execute_effect` arm writes to `target_session_id`. Add the
  `write_to_pty` ownership guard. **Ship last, behind the policy rulings.**

**Parallel, separately gated:**

- **S6 — Project-state memory consumer + librarian_work_queue** *(depends S1/S3/S5 milestone
  events; HARD-GATED on project_id reaching Brain memories + recall).* **Cost: 0 LLM.**

**Two-tier (Tier 1 in v1; Tier 2 deferred):**

- **S7 — "Take Control" handoff** *(depends S1).*
  - **Tier 1 (v1, ~free):** a UI input box letting Jesse type a message relayed through the **same
    NDJSON channel** as gate answers; pauses Henry's auto-gate-advancing on that session while Jesse
    drives. Rides on S5's relay — no new transport.
  - **Tier 2 (DEFERRED — open fork):** true raw interactive CC — **(ii)** fresh interactive `claude`
    resuming the worktree via conversation continuity vs **(iii)** NDJSON-injection-only. Do not
    resolve now. S1 does **not** depend on Tier 2.

---

## Open forks for Jesse

**Fork 1 — Transport — RULED: PTY in the Build-tab terminal.** Each Henry request → a new Build-tab
tab → CC/Codex runs there as a stream-json session, visibly; relay via `write_to_pty`. Not a piped
subprocess, not a custom renderer. The tab shows the NDJSON stream; hand-typing is the S7 Tier-1
escape hatch (rides the relay channel). Session kind = **both** (dispatched-goal or free-standing
watched). Steering = **tell-Henry primary** (user-input NDJSON folds into S5's relay).

**Fork 2 — Auto-clear policy (#401/#402) — OPEN.** The CC-tool → `action_class` → tier mapping.
Which `can_use_tool` gates may Henry auto-relay (Tier 0/1) vs must escalate to Jesse (Tier 2)?
The engine (tiers, fail-closed default, `henry-policy` actor) exists; the mapping is policy.

**Fork 3 — Irreversibility safety-gate location — OPEN.** The tier check in `answer_decision` keys
on `action_class`, which must be derived from the CC gate's `tool_name`+`input` at gate-creation
time. Recommend: classify in the Rust parser, fail-closed to Tier 2 for anything unmapped. Also:
add an ownership guard to `write_to_pty` now that it's a relay sink.

**Fork S7 — "Take Control" mechanism — OPEN, DEFERRED.** (ii) worktree handoff to a fresh
interactive `claude` via conversation continuity, vs (iii) user-input NDJSON injection through the
relay channel.

---

## Jesse's rulings (this session)

- **DETECTION:** stream-json structured gates. **Not** TUI parsing; **not** #423's quiescence
  heuristic.
- **FORK 1 — TRANSPORT (RESOLVED):** **PTY in the Build-tab terminal.** Each Henry request → a new
  Build-tab tab → Henry invokes CC/Codex there as a stream-json session → runs visibly; Henry
  oversees and approves gates; relay via `write_to_pty`. **Not** a piped subprocess, **not** a custom
  renderer. Reuse the existing `terminal.rs` / Build-tab infra. (This resolves the PTY-vs-piped
  sub-fork that S0 surfaced.)
- **SESSION KIND (RESOLVED — BOTH):** A supervised session is **either** a dispatched-goal (Jesse
  says "set up a goal" → uses #424's worktree + goal-review scaffolding via `GoalEngine`) **or** a
  free-standing watched session (Jesse says "just run CC" → no goal/review). **S1 must support both
  entry points.** The session machinery (launch + parse + relay) must be **independent** of whether
  it is wrapped as a goal — the supervised session is *not* always a `GoalEngine` dispatch.
- **STEERING (RESOLVED):** PRIMARY = "tell Henry to update the running session" — Henry relays a
  **user-input NDJSON** message into the **same stdin** he answers gates on. **This folds into S5's
  relay** (same channel; new message type: user-input vs `control_response`). ESCAPE HATCH = "basic
  Take Control" in v1 (see S7 Tier 1).
- **#423:** trimmed to `TerminalProcessExited` only (DONE). The loop uses its **own** completion
  (stream-json `type:"result"` + subprocess exit), **not** #423's `pty_exit` — don't couple them.
- **BLOCKERS:** S6 (project-state memory) is hard-gated on (a) `project_id` reaching Brain memories +
  recall — **closed #70 did NOT deliver this to the Brain tables** — AND (b) a new
  `librarian_work_queue` (the enqueue path doesn't exist; the CRM precedent is itself unbuilt).
- **LLM-in-loop is exactly S1 (launch) + S3 (gate, 0–1).** Everything else deterministic.

---

## Reconciliation note: #59 / PR #424

PR #424 (#59) runs `claude` **headless in a detached git worktree** via a
`GoalEngine`/`ExternalCliEngine`, tracked as a dispatched goal. Jesse's "visible tab" is the
`project_launch` → PTY → `claude` path. These are **different mechanisms and different registries
today.** The collapse: make Jesse's visible tab **the same stream-json CC session #424 launches,
surfaced in a terminal** — visibility becomes a presentation flag on one launch mechanism, one
registry, one gate parser, one relay. **Settled in S0 (see below): the collapse does NOT hold —
#424's engine is gate-bypassed and headless, so S1 is a sibling engine, not an extension.**

---

## S0 Reconciliation Findings (#426 — #424 now merged, `badb7f3c5`)

**#424 is Case (b)-degenerate.** `ExternalCliEngine` (`crates/goose/src/agents/platform_extensions/goal_engine.rs`)
launches the shipped claude worker (`config/agent_identity.rs:299`) as:

```
claude -p "{prompt}" --dangerously-skip-permissions --output-format stream-json
```

via `run_external_cli` (`goal_engine.rs:282`): `Command::new(bin)` with `stdin(Stdio::null())`,
`stdout(Stdio::piped())`, and `cmd.output().await` — i.e. **buffered to completion, then the stdout
is discarded** (only the exit code becomes `GoalOutcome::{Success,Failed,TimedOut}`). It does **not**
use the `claude_code.rs` provider (that's a separate `Provider` impl used only when claude is a
*model backend*).

This **actively defeats the loop**, three ways:
1. `--dangerously-skip-permissions` → CC auto-approves everything → **no `control_request` ever
   fires** → nothing to detect.
2. `stdin: null` → **no channel to relay** a `control_response`. `DispatchedWork { run_id,
   join: JoinHandle<GoalOutcome> }` retains **no writable handle** to the live process.
3. `cmd.output()` buffers + discards → **no streaming parse**, no visibility.

**Conclusion: S1 is a SIBLING engine, not an extension.** #424 contributes the *goal/worktree/review*
half (reusable scaffolding); it contributes **nothing** to the gate-detection / visible / interactive
half and must not be flag-extended into it.

**Session-identity (Pieces 1+3 do NOT fully collapse):** #424's `run_id = "cli-<uuid>"` (recorded as
the card's `worker_session_id`) identifies the goal/worktree but is **not a relay address** — there is
no writable handle, stdin is null. **S2's registry must bridge** `{loop_session_id → (run_id/goal_id
if any, project_id, LIVE writable stdin/PTY handle, pending request_id)}`. The relay handle is
**net-new**, held by S1's new engine.

### S1 scope (sharpened, startable)

S1 = a new sibling `GoalEngine` impl (e.g. `SupervisedCliEngine`) that:
- **Reuses #424 scaffolding:** the `GoalEngine` trait, `GoalTask`, `create_goal_worktree`,
  `DispatchedWork`/`GoalOutcome`, and the dispatch wiring (`run_id → worker_session_id`).
- **Reuses `claude_code.rs` protocol structs** (`IncomingControlRequest` / `ControlResponse` /
  `PermissionResponse`) for parse/serialize.
- **Replaces the invocation:** `claude --output-format stream-json --input-format stream-json
  --verbose --permission-prompt-tool stdio` (**drop** `--dangerously-skip-permissions`), in a **PTY**
  (Build-tab terminal), stdin piped/writable for relay, stdout streamed through a read loop that tees
  to the bus (feeds S2's parser) and is visible in the tab.
- **Must NOT reuse** `run_external_cli`'s `cmd.output()` buffering or the
  `--dangerously-skip-permissions` roster args.
- **Supports both entry points:** dispatched-goal *and* free-standing watched session.

### Take Control (S7) — two tiers (v1 + deferred)

- **Tier 1 (v1, ~free):** a manual-input affordance — a UI box letting Jesse type a message relayed
  through the **same NDJSON channel** as gate answers (the primary steering path). Pauses Henry's
  auto-gate-advancing on that session while Jesse is driving. Rides entirely on S5's relay channel;
  no new transport.
- **Tier 2 (DEFERRED — open fork):** true raw interactive CC — **(ii)** spawn a fresh interactive
  `claude` resuming the worktree via conversation continuity vs **(iii)** NDJSON-injection-only.
  Cost/seam differ; **do not resolve now.** S1 does **not** depend on Tier 2.
