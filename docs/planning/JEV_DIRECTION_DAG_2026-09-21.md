# DAG J — implementing the agreed direction, with Jev as a configurable judge provider

**Date:** 2026-09-21 · **Status:** **AGREED 2026-09-21 15:30** with `permagent-runtime-46` — ownership, gate model and tiers accepted; the judge-role design **amended by the peer** (§4 Phase B); its DoD additions adopted (§3). Dispatch open on every coordinator-owned node.
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

**Added by `permagent-runtime-46`, each from a defect it caught this week:**

9. **Red-then-green means the test was OBSERVED failing**, not argued to fail.
10. **A passing test must be proven to have executed something** — assert on elapsed time or side effects, not on
    `Ok`. (Three integration tests claimed a five-minute worker wait and returned in 15 µs.)
11. **"Did this test pass BEFORE my change?"** — one command, run before blaming the diff. Known pre-existing noise:
    four `goal_engine` tests fail **in isolation** (sqlx "requires a Tokio context") and pass in the full suite — not a
    worker's to fix.
12. **Process-wide guards key per DATABASE, never per session id** — session ids are per-day sequences that repeat
    across stores.
13. **Behavioural assertions over source-greps** — a guard test broke because an unrelated string contained
    `qwen2.5:7b`.
14. **UI diffs use `space.*` / `textSize.*` tokens**; the fitness tests reject raw pixels.
15. **Verify the INSTALLED artifact, not the tree** — on a dirty shared checkout "built from commit X" is false; string
    the bundled binary for a known literal.
16. **No tool schema may carry a top-level `oneOf` / `allOf` / `anyOf`** — Anthropic rejects the whole request. (Live
    outage 2026-09-21: `extensionmanager__get_memory`, `ext_manager.rs:505`; fixed by the peer with a guard test over
    `all_possible_tools()`. **No worker touches that schema.**)
17. **Every worker report states "NOT BUILT"** when it did not build. *"A worker forbidden from cargo is reasoning
    about compilation from reading, and its prose reads identically either way."* **No node closes on a worker's
    claim.**
18. **Gate order inside a window:** focused target → the module's neighbours → the full lib suite. Gates are batched,
    in windows the peer opens.

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
| **A3** *(taken by the peer)* | **Evidence ledger** — per action: did the callee report success · did a check run after the last mutation · is the duration plausible. A "done" with no supporting row is held. | `cost_router/hold_done.rs`, `agents/agent.rs`, `tool_monitor.rs` | **PEER** | — | would have caught both live cases: browser "Done" on a no-op type; 15 µs "five-minute" tests |
| **A4** *(taken by the peer)* | Cost-ledger fields: `call_purpose`, `trigger_ref`, `retry_of_call_id`, **`pre_call_evidence`**, `tool_call_count`, `completion_kind` — set in `record_provider_usage()`, provenance from `reserve_provider_invocation_for_model()`. | `agents/reply_parts.rs`, `session/session_manager.rs`, schema migration | **PEER** | — | historical rows stay `unknown`, never guessed |
| **A5** | ~~Serve the recurring charges~~ **PREMISE WRONG — reduced by its own Phase-0 audit.** The route already serves them: `routes/finance.rs:355-366` uses `spend_forecast`'s list on success, and `recurring: vec![]` is only the fallback when the forecast errors (verified). What is actually missing is **coverage**: no test asserts `recurring` anywhere, and there is no route test for `GET /api/finance`. | `crates/goose/src/finance_ledger.rs` tests only | coordinator | T0 | a ledger test with two same-payee transactions asserts a non-empty `recurring`; no production change |

### Phase B — Jev as a configurable judge provider (Jesse's explicit ask)

**Reference for every worker on Phase B:** TypeSafe's own agent skill — installed for Claude Code on 2026-09-21 (`typesafe@typesafe-ai`, user scope; reviewed first: MIT, guidance only, no commands). Codex and Cursor workers do not load Claude plugins, so their prompts must tell them to read `https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md` and the docs index `https://docs.typesafe.ai/llms.txt` before writing any request-building code. The key is in the keychain as `TYPESAFE_API_KEY` (name only) and is **verified live**: `docs/research/jev-evidence/jev-live-smoke-2026-09-21.json` is a real response to use as a wire fixture.

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
- **Role — AMENDED BY THE PEER, and it is right.** Do **not** add `Judge` to `WorkflowRole` (`recommend.rs:259`): that
  enum *is* the text routing table — a fixed `[WorkflowRole; 5]` enumerated by `derive_role`, `resolve_role_model`,
  `mappings_to_persist`, the Settings picker and the statusline — so the recommender would propose Sonnet for the judge
  and the UI would offer it; the first invariant would be *violated by construction* and defended by runtime asserts
  forever. Instead the decision module gets **its own parallel namespace**: a small `DecisionRole` enum in
  `crates/goose/src/decision/` with keys `PERMAGENT_DECISION_JUDGE_PROVIDER` / `_MODEL`, mirroring `role_map`'s
  half-configured rule exactly (`Some` only when **both** keys are non-empty). Both invariants then hold
  **structurally**: the judge is not in the text table, and no text role can resolve to it **as long as `jev` and
  `systemone` are never registered in `providers::create`** — unknown ids already hard-fail there. `role_map.rs` is not
  touched at all.
- **Budget identity.** Hosted `jev` is a **paid** provider, and a paid reservation requires a durable task identity —
  the exact thing that silently killed every scheduled run on 09-19 → 09-21 (fixed in `0cafe04c`). **A judge call from
  a scheduled or headless context must be inside a budget task or it is refused before the wire — an explicit test in
  B2, not an assumption.**
- **Vendor neutrality.** One trait with one backend is a trait shaped like that backend. **A second, deterministic
  `DecisionBackend` lands in the same node**, however simple — otherwise the seam is unfalsifiable.
- **Fail awake.** Not only *unset role → deterministic path*: **backend unreachable, timeout, malformed distribution and
  refused-by-locality all fall to the deterministic path and log** — never block, never silently pick.
- **Never a classifier over explicit state** — if the answer is already in structured state, no question is asked.
  A call-site rule **with a test**, not a convention.
- **Privacy guard:** every call site declares `StateClass::{Public, Private}`. `Private` + `Cloud` → refused before the
  request is built, recorded in `egress_audit`, fail-closed. Batching guard: ≤ 20 questions per state; a larger batch
  is split, never sent. *Source for the cliff:* `realZachi/pg-jev`'s own benchmark (1–20 rows per state 100 %, 40 →
  92–98 %, 80 → 77–94 %), reproduced independently by `yodablocks/jev-orderby-bench` — both measured on **items**
  batched into one state on **hosted** Jev. For *questions about one item* on a *local* backend it is an **assumption
  to be measured in B4**, not a fact.
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

## 4b. Phase V — the voice incident of 2026-09-21 (added at Jesse's request)

*Jesse: "I tried voice and it got stuck on listening and then the iOS app crashed. It might be because it is an old
build." Reviewed from the daemon log and the crash report pulled off the phone. Raw evidence (kept out of the repo —
the crash report carries device identifiers): `permagent-worktrees/evidence/voice-incident-2026-09-21/`.*

### What happened — TRACED from logs, second by second (local time)

| Time | Event | Evidence |
|---|---|---|
| 13:30:11 | App launched on the iPhone 14 Pro Max, **iOS 27.0 (24A435)**, Permagent **1.31.0 build 945** | crash report header |
| 13:30:19 | Voice socket connected — `client=ios_voice`, session `20260910_1`. **The connect line carries no build id.** | `daemon.err` |
| 13:30:20–27 | **Turn 1 worked.** 2.1 s of speech → STT 294 ms → *"I can hear you just fine. What do you need?"* Speech-end → first audio **5.57 s**: TTFT **2,540 ms** (`custom_deepseek / deepseek-chat`) + a **cold first TTS chunk 2,153 ms** (RTF 0.88; the next chunk took 522 ms) | `voice_latency_summary` turn 1 |
| 13:30:32 | Turn 2 capture starts, ~1.3 s after the reply finished playing | |
| 13:30:32–57 | **"Stuck on Listening": the phone streamed audio for 25.8 s** before its end-of-turn fired (`stop_reason: client_stop`). Real signal, not a dead mic: 412,912 samples, none zero or non-finite, **mean RMS 0.037, peak 0.80** (turn 1's speech: mean 0.057, peak 0.28) | `voice_capture_health` turn 2 |
| 13:30:58 | STT returns **empty** in 227 ms → `outcome: empty_stt`, `empty_reason: finite_signal_no_words`, `classification: critical_stall` | `voice_latency_summary` turn 2 |
| 13:30:58 | **The server says nothing.** By design: *"Return to ready with no toast"* (`routes/voice.rs:2410-2422`). The phone goes back to Listening with no sign that 25 s of audio produced nothing | code |
| 13:31:01.04 | Phone sends a **clean Close frame** — the voice screen was closed deliberately, not torn down by a crash | `close_reason: client_close_frame` |
| 13:31:01.81 | **App crashes, 0.8 s later** | `Permagent-2026-09-21-133102.ips` |

### Finding V-1 — the crash is a Swift-6 actor-isolation trap on iOS 27, and a rebuild alone will NOT fix it

`EXC_BREAKPOINT / SIGTRAP` on thread **`com.apple.SwiftUI.AsyncRenderer`**:
`_dispatch_assert_queue_fail ← dispatch_assert_queue ← _swift_task_checkIsolatedSwift ←
swift_task_isCurrentExecutor ← ` **`closure #1 in PermagentApp.body.getter`** ` ← SwiftUICore LazyView.body.getter ←
ViewGraph.updateOutputsAsync ← displayLinkTimer`. *(Symbolicated with `atos` against the matching dSYM, UUID
`266C5672-…`.)* SwiftUI evaluated the app's root scene content on its **background async renderer**; that closure is
main-actor isolated, and in Swift 6 language mode (`SWIFT_VERSION = 6.0`) the runtime asserts the executor and traps.
The closure is the `WindowGroup { ZStack { … } }` content in `ios/PermagentMobile/PermagentMobile/PermagentApp.swift`,
which reads `session.isPaired`, `splashDone` and `route.showVoice` — and the trigger fits: closing voice flips
`route.showVoice`, the root re-renders, the app dies 0.8 s after the Close frame.

- **"Old build" — confirmed, and not the cause.** The matching dSYM is in the Xcode archive dated **2026-09-08 19:59**:
  the phone runs a **13-day-old build.** But `PermagentApp.swift` has **no commits since that archive**, so today's
  source has the same closure. *Located, not yet traced:* whether building against a newer SDK changes the inserted
  check. **No fix is proven until it survives the device.**
- **What changed is the OS.** iOS 27.0 shipped this month; the build predates it.
- Only one Permagent crash report is on the phone, so frequency is unknown.

### Finding V-2 — "stuck on Listening" is end-of-turn detection that never fires, then a silent empty result

- The phone decides when a turn ends (`VoiceVAD.swift`): `keepalive 0.006`, `silenceMs 1400`, `maxTurnMs 60000`. Turn
  2's **mean** level over 25.8 s was **0.037 — six times the keepalive floor** — so the VAD never saw 1.4 s of
  "silence". Either the room was noisy (then the floor is too low for it) or Jesse was speaking (then STT returning
  nothing for 25 s of speech is its own bug). **The log cannot tell which: it records no input route, no noise-floor
  estimate and no end-of-turn reason.** Hypothesis, not finding.
- **The leading hypothesis is one we already had** *(reminder from `permagent-runtime-46`; I had cited the 09-01
  forensics without foregrounding its verdict)*: on 2026-09-01 this exact signature was root-caused to the **Bose
  headset's narrowband HFP microphone — not code, and not fixed by a rebuild.** *Real energy, no intelligible words* is
  what an HFP mic path produces. **If the input route today was Bluetooth HFP, this whole finding collapses into a
  known hardware cause**, V4 (never silent after an empty transcript) becomes almost the entire fix, and the
  headphones-first I/O split designed that day (output on the headset over A2DP, input on the built-in mic so HFP never
  engages) is the structural answer. **One question to Jesse settles it faster than any telemetry: were the Bose
  headphones connected?**
- **It recurs.** 2026-09-19 15:38 — a **19.6 s** capture → `empty_stt / finite_signal_no_words`; the same session has
  turns of 76 s, 42 s and 30 s flagged `critical_stall`.
- The server windows audio over 14 s into ≤ 12 s segments (`sherpa_backend.rs:63-70`), so length alone should not
  produce an empty transcript — *located, not traced* through the windowing path with real audio.
- **The silence after an empty transcript is the part that reads as "stuck".** The 2026-09-01 forensics already
  specified the fix — *empty-transcript feedback after two strikes* and a *ready-state stall detector that fails open* —
  as V2a / V2b of the voice DAG. **Neither is in the running build or in this source path.**

### Finding V-3 — latency, again

First audio 5.57 s after speech end. Avoidable today: the **cold first TTS chunk (2.15 s vs 0.52 s warm)** — pre-warming
Kokoro on socket connect was V2c on 2026-09-01. TTFT 2.54 s is the model thinking; no classifier fixes that.

### Nodes

| Node | What | Files | Tier | Acceptance (on the device — nothing here is done on a simulator) |
|---|---|---|---|---|
| **V1 — P0** | Stop the iOS 27 crash. Phase 0 first: reproduce (open voice → close → crash within ~1 s?), check whether it also fires without voice, and pull any further `.ips`. Candidates, **all hypotheses**: move the scene content into a `RootView: View` so the `App.body` closure touches no isolated state; rebuild against the iOS 27 SDK; `-Xfrontend -disable-dynamic-actor-isolation` **only to confirm the diagnosis — it disables the check, not the bug, and must never ship.** A rebuild alone is **not** a fix; no worker may regress to that. **Read the uncommitted `project.pbxproj` diff in the shared checkout before trusting any device result** — it was already dirty this morning (build number 936 → 941 plus regenerated ids). | `ios/PermagentMobile/PermagentMobile/PermagentApp.swift`, project settings | T2 — Codex Astra; review by a second worker | 25 consecutive voice open/close cycles on the iPhone on iOS 27.0 with no crash and no new `.ips`; the same on the watch relay path. CI runs Xcode 16.4 and this machine 26.x — a local green is **not** evidence CI passes |
| **V2** | **Telemetry before tuning** — the phone reports per turn: input route (built-in / wired / Bluetooth HFP / A2DP), the VAD's noise-floor estimate, and the **end-of-turn reason** (`silence` / `max_turn` / `user_tap` / `barge`). The server logs them on `voice_capture_health`. | `ios/…/VoiceVAD.swift`, `ios/…/Voice*`, `routes/voice.rs` | T1 | a replay of today's turn would have said *why* it ran 25.8 s |
| **V3** | **Build identity — and the input route — on the wire:** app version, build number, git sha, **audio input route and hardware sample rate** in the voice socket hello, logged on connect. Today `client=ios_voice` is all the server knows, so "is it an old build?" took a crash report and a dSYM to answer. | iOS voice client hello, `routes/voice.rs` | T0 | the connect line shows `1.31.0 (945) <sha>` |
| **V4** | **Never silent after an empty transcript** — after an `empty_stt` on a capture longer than N seconds, or two in a row, say so (*"I didn't catch that"*, naming the input device); hands-free auto-started turns get a much shorter cap than 60 s; a ready-state stall detector that fails open. This is V2a/V2b from 2026-09-01, unshipped. | `routes/voice.rs`, `ios/…/VoiceVAD.swift` | T1 | deliberate-silence and deliberate-noise drills on the device both end with spoken feedback |
| **V5** | Pre-warm Kokoro on socket connect (V2c, unshipped) | `routes/voice.rs` / voice TTS | T0 | first chunk ≤ 700 ms on a fresh connection |
| **V6 — later** | Semantic end-of-turn on ASR partials (`JEV_FRONTIER_IDEAS` §1.6). **Blocked on V2's telemetry** and on a resident scorer having somewhere to live; not a substitute for V4 | — | — | — |

**Order: V1 → V3 → V2 → V4 → V5.** V1 is the only P0 in this DAG: it takes the app down. The existing voice programme
(`docs/orchestrator/VOICE_V3_END_TO_END_DAG.md`, branch `feat/voice-v3-dag`) owns W0–W9; V2, V4 and V5 belong inside it
rather than beside it — to be reconciled with whoever holds that lane before dispatch. **V1 is new — nothing in V2 or
V3 anticipated an OS-level isolation trap.**

## 5. Needs Jesse

1. ~~A TypeSafe key~~ **RESOLVED 2026-09-21** — Jesse stored one in the keychain (`TYPESAFE_API_KEY`); a live call returned HTTP 200 in 0.50 s. Public inputs only. *Still open:* approval to download a ~400 MB local Jev-compatible model to the M1, which is what the **local** backend — the default for anything private — needs for its own end-to-end check.
1b. **V1 needs your phone:** the iOS 27 crash fix is only proven by installing a build and running 25 voice open/close cycles on the device.
2. **Minimum term/category counts** in the Librarian (they force padding on sparse memories) — a retrieval-quality
   ruling, deliberately left out of A2.
3. **Phase D** — each item is a product decision and gets its own ruling.

## 6. Order of execution

**V1 (the iOS 27 crash) goes first — it is the only P0.** Then: `A1 ‖ A2a ‖ A5` (three independent T0–T2 lanes, edits in parallel, gates serialised) → `A2b, A2c` → `B1` → `B2` →
`B4` → `C1`. Peer-owned `A3`, `A4`, `B3`, `C2` run on the peer's schedule; `C1` waits for `A4`, `C2` for `A3`.
