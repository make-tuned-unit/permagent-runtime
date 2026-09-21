# DAG J — implementing the agreed direction, with Jev as a configurable judge provider

**Date:** 2026-09-21 · **Status:** PROPOSED — awaiting `permagent-runtime-46`'s agreement on ownership and order.
**Asked by:** Jesse — *"Coordinate with the other session to create a structured DAG to implement the agreed upon
direction. Use Codex via plugin and Cursor via CLI as workers. Verify every piece of code end to end for full wiring
with full regression tests. Add JEV as a model provider so we can have it configured for its new role. Work
methodically… use the cheapest workers capable for the job."*
**Direction being implemented:** the joint position in `docs/research/CODEX_DEBATE_2026-09-21.md` §7 and the order in
`docs/research/JEV_FRONTIER_IDEAS_2026-09-21.md` §4 (as revised by `permagent-runtime-46`).
**Coordinator:** `permagent-runtime-42` (Fable — plans, reviews, gates; writes no feature code).
**Live status board:** `permagent-worktrees/JEV_DAG_BOARD.md`.

---

## 1. Rules of the DAG

1. **Deterministic first.** Every node that fixes a defect without a model ships before any node that uses one.
2. **The judge is local by default and private state never leaves the machine.** Hosted Jev is `DataLocality::Cloud`;
   a call site that declares its state `Private` is **refused** on a Cloud backend, fail-closed, with an egress-audit
   row. Tighten-only for gates; fail-awake for anything that can suppress work; code owns every state machine.
3. **One node = one workflow with a named label source and a kill date** (the Codex test). No platform nodes.
4. **Ownership is by file, agreed with the peer.** `permagent-runtime-46` owns `cost_router/**`, `agents/agent.rs`,
   `agents/reply_parts.rs`, `platform_extensions/orchestrator.rs`, `goal_engine.rs`, `providers/formats/**`,
   `providers/canonical/**`, `ui/**`. Nodes touching those are **its** to take, schedule or decline.
5. **Workers edit; they never build.** A worker works in its own worktree, **edit-only, no cargo, no npm install**,
   and hands back a diff. It may not touch a path outside its node's allow-list.
6. **Gates run in the main checkout, one node at a time**, on the warm target, inside an announced window.
   *Why this departs from the dispatch standard's per-lane `CARGO_TARGET_DIR` (#584):* a cold target for this workspace
   needs 40–60 GB and the volume has ~25 GiB free. #584 exists to stop one lane running another lane's test binary;
   serialising every gate through `scripts/build-guard.sh`'s lock on a single target achieves the same thing. **This is
   a proposal to the peer, not a fait accompli.**
7. **No source edit lands in the shared checkout while a release build is running or planned** — checked with `ps` and
   confirmed with the peer first (today's lesson). Before any release build the builder runs `git status --short`.
8. **Every cargo call:** ≥ 11 GiB free, a stated growth estimate (**4–5 GiB** for a lib-test relink, measured), a df
   poll with a 7 GiB kill, start + finish notices to `permagent-runtime-46` and `sidebar-14`. Nothing heavy on the M1.
9. **A node is DONE only with pasted evidence**, never an assertion — see §3.

## 2. Workers, cheapest capable first

| Tier | Use for | Worker and model |
|---|---|---|
| **T0 mechanical** | a one-function fix with an obvious test; wiring a field; renames | Cursor CLI `--model composer-2.5` · or Codex `--model gpt-5.3-codex-spark` |
| **T1 standard** | a bounded feature in 1–3 files following an existing pattern | Cursor CLI `--model gpt-5.3-codex` |
| **T2 design-bearing** | a new module or trait, security-sensitive code, anything cross-cutting | Codex plugin default (`gpt-6-astra`), `--write`, rooted in the lane worktree |
| **Audit / adversarial review** | Phase-0 audits; reviewing another worker's diff | Codex read-only (`write:false`) — a *different* worker from the author |
| **Plan, review, gate, merge** | — | Fable (this session). Never the implementer. |

Start every node one tier lower than it looks; escalate only on a failed gate, carrying the failure forward.
Invocations: `cursor-agent -p --force --trust --workspace <worktree> --model <m> "<prompt>"`;
Codex via the `codex:codex-rescue` subagent, run with the worktree as cwd (a worktree-rooted Codex job can edit but may
not be able to commit — the coordinator commits after review).

## 3. Definition of done — every node

1. **Phase-0 audit** recorded: what already exists, exact files/functions, the pattern mirrored. *Located* vs *traced*
   stated for every behavioural claim.
2. **Unit tests** for the change, including the failing case that motivated it (red before, green after — shown).
3. **A wiring test** proving the production path reaches the new code — the house pattern of a source-scanning or
   call-path test, not just a test of the function in isolation.
4. **Regression:** the owning crate's **full** lib suite — `scripts/build-guard.sh -- cargo test -p permagent --lib`
   (baseline today: 5,783 passed) or `scripts/test-daemon.sh --lib <filter>` for the daemon crate; plus
   `cargo clippy -p <verified crate name> --lib --bins -- -D warnings` and `cargo fmt --all --check`. Frontend nodes:
   `npx tsc --noEmit && npx vite build` and the vitest file.
5. **End-to-end evidence** where a live path exists (a real call against the running component, or the eval bench),
   with the command and its real output pasted into the board.
6. **Adversarial review** of the diff by a worker that did not write it.
7. **Coordinator review**, then commit by the coordinator with the evidence summarised in the message.
8. **Self-knowledge descriptor** in the same change for any user-facing capability (standing rule).

## 4. The graph

```
A1 adversary fail-closed ─────────────┐
A2 librarian contract leftovers ──────┤
A5 serve recurring charges ───────────┤            (independent; T0–T1; coordinator-owned files)
                                      │
B1 decision module + /v1/systemone ───┼──► B2 provider registration + role + locality guard ──► B4 bench arm
                                      │            │                                              │
A3 evidence ledger   (PEER) ──────────┤            └──► B3 Settings UI entry   (PEER, ui/**)      │
A4 call-purpose ledger fields (PEER) ─┘                                                           ▼
                                                   C1 ingest-triage experiment on the eval bench (needs B2, A4)
                                                   C2 trajectory controller for cheap-rung runs  (PEER; needs B2, A3)
D*  instructions ledger · graph verification (Spectral) · inbox ranking · typed honesty half · voice partials
    — NOT in this DAG's first pass; each needs its own ruling from Jesse.
```

### Phase A — correctness, no model

| Node | What | Files (allow-list) | Owner | Tier | Acceptance beyond §3 |
|---|---|---|---|---|---|
| **A1** | `adversary_inspector`: a verdict that is not exactly ALLOW or BLOCK, and **any error**, goes to hold/ask — never allow. Today it parses `starts_with("BLOCK")`, treats **everything else — including an empty reply — as ALLOW** (`:322-348`), allows on any error or timeout (`:437-450`), and allows when no provider is available (`:253`). Phase-0 audit done (Cursor `composer-2.5`, verified): three fail-open paths, not two; hold-and-ask is `InspectionAction::RequireApproval`; the inspector is registered always but **off unless `adversary.md` exists**; no test covers verdict parsing or the error paths; `test_adversary_enabled_default_tools` currently *asserts* no-provider → Allow and must change on purpose. | `crates/goose/src/security/adversary_inspector.rs` (+ its tests) | coordinator | T2 (security) → review by Codex read-only | tests for: malformed verdict, empty reply, provider error, timeout — each must NOT allow; existing ALLOW/BLOCK behaviour unchanged |
| **A2** | Librarian leftovers: (a) `field_after_label` must anchor labels to line starts — a FACTS sentence containing "terms:" splits wrongly today; (b) the native Ollama request sends `think:false` and a JSON-schema `format`, **passed from the Librarian caller only**, not set globally in the shared builder; (c) the endpoint-failure fallback honours the scheduled model instead of the hard-coded `DEFAULT_MODEL`. | `platform_extensions/librarian.rs`, `mesh/pool.rs` (a new opt-in constructor only) | coordinator | (a) T0 · (b) T1 · (c) T0 | (a) a regression test with "payment terms: net 30" in FACTS; (b) request-body tests for both paths + a live describe on the resident 7B; (c) test the fallback model. **Not included — needs Jesse's ruling:** lowering the minimum term/category counts (a retrieval-quality decision). |
| **A3** | **Evidence ledger** — per action: did the callee report success · did a check run after the last mutation · is the duration plausible. A "done" with no supporting row is held. | `cost_router/hold_done.rs`, `agents/agent.rs`, `tool_monitor.rs` | **PEER** | — | would have caught both live cases: browser "Done" on a no-op type; 15 µs "five-minute" tests |
| **A4** | Cost-ledger fields: `call_purpose`, `trigger_ref`, `retry_of_call_id`, **`pre_call_evidence`**, `tool_call_count`, `completion_kind` — set in `record_provider_usage()`, provenance from `reserve_provider_invocation_for_model()`. | `agents/reply_parts.rs`, `session/session_manager.rs`, schema migration | **PEER** | — | historical rows stay `unknown`, never guessed |
| **A5** | ~~Serve the recurring charges~~ **PREMISE WRONG — reduced by its own Phase-0 audit.** The route already serves them: `routes/finance.rs:355-366` uses `spend_forecast`'s list on success, and `recurring: vec![]` is only the fallback when the forecast errors (verified). What is actually missing is **coverage**: no test asserts `recurring` anywhere, and there is no route test for `GET /api/finance`. | `crates/goose/src/finance_ledger.rs` tests only | coordinator | T0 | a ledger test with two same-payee transactions asserts a non-empty `recurring`; no production change |

### Phase B — Jev as a configurable judge provider (Jesse's explicit ask)

**Design, to be confirmed by the peer before B1 starts.** Jev generates no text, so it is **not** a `Provider`
(`providers/base.rs:590` — `stream`/`complete`); forcing it through that trait would make every caller handle a model
that cannot answer. It gets its own small seam, and it is registered and configured like a provider:

- **`crates/goose/src/decision/`** — `Question { Choice | Score | Noul }` (always with a none-of-these option for
  `Choice`), `Answer { distribution }` — **no `confidence` scalar** (measured to be worse than the distribution),
  `trait DecisionBackend { fn data_locality(); async fn decide(state, questions) }`.
- **One wire client for `/v1/systemone`**, the shape hosted Jev and the local Jev-compatible servers (Laya's adapter,
  `von`, `jev-local`) all speak. **Provider ids:** `jev` (hosted, `https://api.typesafe.ai`, key `TYPESAFE_API_KEY` in
  the keychain, `DataLocality::Cloud`) and `systemone` (any compatible endpoint; `Local` **only** when the host is
  loopback or on the trusted mesh — otherwise Cloud, fail-closed).
- **Role:** `PERMAGENT_ROLE_JUDGE_PROVIDER` / `PERMAGENT_ROLE_JUDGE_MODEL`, the existing `PERMAGENT_ROLE_*` pattern
  (`cost_router/role_map.rs` — **peer-owned; the peer adds the role or tells me how**). Unset = no judge; every call
  site then takes its deterministic path. **The judge role never resolves to a text model and a text role never
  resolves to the judge.**
- **Privacy guard:** every call site declares `StateClass::{Public, Private}`. `Private` + `Cloud` → refused before the
  request is built, recorded in `egress_audit`, fail-closed. Batching guard: ≤ 20 questions per state (measured
  accuracy cliff); a larger batch is split, never sent.
- **Journal, right-sized:** `(site, question_version, state_ref, distribution, acted)` — a **reference** to the state,
  never a copy, so forgetting a memory cascades. Thresholds live in config, never as literals.
- **Accounting:** judge calls go through the same accounted path as `complete_fast` and join
  `permagent-eval/src/dispatch_inventory.rs` as a new seam kind, so the bench can see them.

| Node | What | Owner | Tier | Acceptance beyond §3 |
|---|---|---|---|---|
| **B1** | `decision` module, types, trait, `/v1/systemone` client, batching guard, injection rail text in the question builder | coordinator | T2 (Astra) | wire-format tests against the request/response JSON quoted verbatim from TypeSafe's API docs (`docs/research/jev-evidence/jev-research-A-facts.md` §1.7); an in-process mock server; timeouts, 401/422/429/529 handling; **no live hosted call in tests** |
| **B2** | registration, keychain key, role resolution, locality + `StateClass` guard, journal table, accounting | coordinator + **PEER for `role_map.rs`** | T2 | tests: Private+Cloud refused and audited; loopback = Local; unset role = deterministic path; judge role cannot resolve to a text model. **End-to-end:** a real call to a Jev-compatible endpoint — *see "needs Jesse" below* |
| **B3** | Settings entry: pick the judge provider, endpoint, key; show locality plainly | **PEER** (`ui/**`) | T1 | tsc + vite + vitest; honest copy — *"sends the text you judge to TypeSafe in the US"* when `jev` is selected |
| **B4** | bench arm: latency p50/p95, accuracy, flip rate under option reordering, on **public fixtures** | coordinator | T1 | numbers pasted into the board; no private data in any fixture |

### Phase C — the first typed uses

| Node | What | Owner | Needs |
|---|---|---|---|
| **C1** | **The one funded experiment:** judge each large tool result at ingest — *"does this contain the answer to the question that caused the call?"* — leave a stub for what is dropped. Arms: none · head/tail · BM25 · k-sample agreement · judge. Scored on `permagent-eval`'s exit-code oracle, input tokens, and re-ask rate. | coordinator (`large_response_handler.rs`, `permagent-eval`) | B2, A4 |
| **C2** | Trajectory controller for cheap-rung runs: the judge **labels** progress per turn; **code** applies the invariant and escalates. True label = verify outcome; cost-to-abort from per-run budget ids. | **PEER** (orchestrator) | B2, A3 |

**Kill dates.** C1: if no arm beats head/tail on task success at equal-or-lower tokens after one full bench round, the
judge arm is removed. C2: decided by the peer.

## 5. Needs Jesse

1. **A TypeSafe key (or "local only").** B2's end-to-end check needs a real `/v1/systemone` endpoint. Either a hosted
   key (waitlist, or via Vercel AI Gateway / Cloudflare) — used **only** on public fixtures — or approval to download a
   ~400 MB local Jev-compatible model to the M1 (heads-up to both peers). Until one exists, B2 is verified against a
   mock and says so on the board.
2. **Minimum term/category counts** in the Librarian (they force padding on sparse memories) — a retrieval-quality
   ruling, deliberately left out of A2.
3. **Phase D** — each item is a product decision and gets its own ruling.

## 6. Order of execution

`A1 ‖ A2a ‖ A5` (three independent T0–T2 lanes, edits in parallel, gates serialised) → `A2b, A2c` → `B1` → `B2` →
`B4` → `C1`. Peer-owned `A3`, `A4`, `B3`, `C2` run on the peer's schedule; `C1` waits for `A4`, `C2` for `A3`.
