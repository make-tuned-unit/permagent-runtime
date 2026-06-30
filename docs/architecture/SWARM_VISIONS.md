# Swarm Visions — Two Expanded-Ambition Designs

**Status:** RATIFIED 2026-06-30 — all decision points ruled (V1-0 + V1 + V2 + the shared proposal-surface
question). Still DESIGN ONLY: nothing builds from this doc. Vision 1 is gated behind B + 2b + slice 4;
Vision 2 is a new epic that needs the cross-device sizing spike first. Zero code, zero runtime files.
Captures two ambitions as architecture so they were ruled before anything builds (the #538 pattern).
**Scope:** Vision 1 — the Enricher as a *visible world-character* (extends slice 4 of
[`UNIFIED_WORKSPACE.md`](./UNIFIED_WORKSPACE.md), #255/#256/#257). Vision 2 — the Git Steward
expanded into a CI-running, fix-dispatching, multi-repo, cross-device actor (a **new epic**, distinct
from the #318 output-fix already in flight).
**Method:** Both designs were audited against `origin/main` (`7dbe8adbc`) in Phase 0; §"Established
reality" sections below are facts confirmed in code, not assumptions. Where the ambition exceeds what
git's model can give honestly, this doc flags a **research spike** rather than hand-waving a design.

---

# Vision 1 — The Enricher as a Character in the World

## 1.0 V1-0 — The gating question: is the Enricher redundant with the Librarian? — RULED

> **DECISION V1-0 — RULED 2026-06-30: (a) the Librarian ABSORBS enrichment** as a second Brain-maintenance
> task — **conditioned on task-level distinctness** (its own capability descriptor line, its own egress
> declaration, and its own `librarian_enrich_*` event namespace under the shared identity). There is **no
> 4th agent.** The other V1 decisions collapse into Librarian-*task* decisions, most of them moot.

This decision gates the rest of Vision 1 and was evaluated before it. The two options:

- **(a) Librarian absorbs enrichment** — the single Brain-maintenance worker *describes* (summarizes what is
  already in the Brain) **and** *enriches* (acquires structured typed fields for people from external
  sources), with the #538 bounds applying to the enrichment task. **Leaner** — no 4th identity / persona /
  descriptor / roster / world slot.
- **(b) A distinct Enricher agent** — a 4th `WorkerPersona` + roster presence, preserving a separate
  character.

**Recommendation, with reasoning — (a):**

1. **A separate identity adds no safety; the bounds do.** Both workers write through `SafeBrain`. The #538
   enrichment bounds (review-gated, manual-wins via `FieldSource`, structured-fields-only) are enforced at
   the **write seam**, so they attach to the *enrichment task* regardless of which worker runs it. Splitting
   the identity buys nothing on the safety axis — `set_entity_field`'s manual-wins arbitration is per-field
   in the graph, blind to worker identity.
2. **Near-zero migration cost — decide now, before either is built.** The Librarian's `engine` is
   `Pending` (Phase 0: `default_roster()` — engine *not yet wired*). Absorbing enrichment adds a task to an
   **unbuilt** worker; it is not a refactor of a live one. This is the cheapest possible moment to fold them.
3. **One Brain-maintenance worker is more coherent.** `entity_fields` *are* graph/Brain data; enrichment
   *is* Brain curation of a typed shape. "Describe" and "enrich" are both "make the Brain better." Two
   near-identical Brain-writing workers is exactly the duplication that drifts (the dead-store failure class
   this codebase keeps fighting), and it forces the user to ask "what's the difference between these two?"
4. **Less infrastructure, per CLAUDE.md.** No 4th roster/persona/descriptor/world slot. Minimal infra,
   explicit over magic.
5. **The character vision survives, richer.** The **Librarian is the character.** A Brain-keeper who both
   *organizes* (describe) and *gathers* (enrich) is a fuller single presence than two thin overlapping ones —
   and you (V1-B/C) already get to design that one character's feel.

**The condition that preserves what (b) was protecting** — the one genuine difference between the two tasks
is **egress**: describe is a `$0` local summarizer that never leaves the machine; enrich *acquires external
facts* (web/email), which is a real trust/provenance boundary (the deferred web-search egress concern). So
absorption is ruled **only with task-level distinctness kept intact:**

- **Distinct capability line** in the Librarian's `WORKER_DESCRIPTOR` — describe and enrich are listed as
  two named capabilities, with enrich carrying an explicit **external-egress flag** and provenance note.
- **Distinct event namespace** — `librarian_enrich_started / _field_proposed / _completed / _error`, parallel
  to `librarian_describe_*`, so the character's visible "working" state is honest about *which* task is
  running (never blurs a local summarize with an external acquire).
- **The #538 write bounds** apply to the enrich task unchanged.

This captures (a)'s leanness and architectural coherence while keeping (b)'s only real win — an honest,
separately-described, egress-flagged external-acquisition capability — under one identity.

**Consequence for the rest of Vision 1:** §1.2's "four declarations" collapse to **two additions to an
existing worker** — an `enrich` task + its `librarian_enrich_*` events + a descriptor *line* (not a new
descriptor). V1-A (peer vs subagent) is moot (the Librarian is already a peer worker). V1-B/C
(placement/persona) are moot for *creating* a character — they fold into your existing right to design the
Librarian's feel (§1.4). V1-E (presence honesty) still applies, to the enrich task's visible state.

## 1.1 Established reality (audited, do not redesign)

| Thing | Where | What it is |
|---|---|---|
| **Enricher (the worker)** | `UNIFIED_WORKSPACE.md` §6 slice 4 | A background worker that writes `FieldSource::Enriched` through the *same* `set_entity_field(...)` write seam a human uses (2b). Gated behind **B + 2b verified**. Bounds (unchanged here): review-gated, manual-wins (#499, already enforced), **structured fields only**. |
| **Character roster** | `ui/command-center/src/components/world/roster.ts` | `AgentIdentity { id, name, role: 'orchestrator'\|'agent', trimColor, isHenry, mezzanineLocked, home:{x,y,z}, weathering }`. Three today: **Henry** (orchestrator, crown), **The Librarian** (`mezzanineLocked`, Brain curation), **The Reader** (ground floor, OCR/ingest). |
| **Character render** | `world/WorldAgents.tsx`, `AgentCharacterV2` | Maps `ROSTER` → characters; resolves the orchestrator's name from the configured persona (not hardcoded "Henry"). Per-frame `advanceMotion()` drives locomotion; `useAgentRuntimeStates()` reads live state. |
| **Activity state** | `world/types.ts` | `activity: 'idle' \| 'walking' \| 'working' \| 'thinking'`; HUD badge `hudState: 'idle' \| 'available' \| 'working' \| 'error'`. |
| **Live-state wire** | `routes/events.rs` (`/events` WS) | The Librarian's visible state is driven by `librarian_describe_started / _token / _completed / _error` events (snake_case `type`, `payload.state`). `agent_state_tick.rs` heartbeats Henry every 2s. |
| **Worker persona** | `config/agent_identity.rs` | `WorkerPersona { first_name, role, traits, tone, tool_kinds, availability_check, engine }`. Librarian is a `default_roster()` entry with `engine: WorkerEngineKind::Pending`. |
| **Self-knowledge** | `agents/self_knowledge/mod.rs` | `WORKER_DESCRIPTORS` (scheduler, librarian), `SURFACE_DESCRIPTORS`, `GUARD_DESCRIPTORS`. Each is a `const FeatureDescriptor { id, category, state_source: StateSource::Queryable\|Static, display_name, what_it_does, why_it_matters, teaching }`. |

**The key finding:** "make the Enricher a character" is **not a new mechanism** — it is the **Librarian
pattern applied to a fourth agent.** The Librarian is already a background worker (engine `Pending`)
that has (a) a `WorkerPersona`, (b) a `roster.ts` presence locked to a world zone, (c) a
`WORKER_DESCRIPTOR` in the self-knowledge brief, and (d) `*_describe_*` events that drive its visible
activity. The Enricher becomes a character by filling those same four slots. There is no new rendering,
state, or wire machinery to invent — only four declarations to add, all gated behind slice 4.

## 1.2 The design: presentation layer over the bounded worker

> **Contingency (V1-0 = a):** under the ratified ruling the "fourth agent" framing below is **superseded** —
> the four declarations become **two additions to the existing Librarian** (an `enrich` task +
> `librarian_enrich_*` events + a descriptor *line* with an egress flag). The mechanism described here is
> identical; only the *count* changes (extend one worker, don't mint a new one). This section is kept for the
> mechanism; read "the character" as "the Librarian's enrich task."

The #538 bounds are **load-bearing and unchanged**. The character is strictly a *presentation* of the
bounded worker — it changes nothing about what the worker is allowed to do:

```
  [ bounded worker, slice 4 ]                 [ character layer, this doc ]
  enrich sweep (scheduler-driven)   ──emits──►  enricher_* events ──► visible 'working' state
  propose field value               ──routes──►  decision-inbox card ──► visible 'idle/available'
  set_entity_field(.., Enriched)    ◄──gated──   manual-wins, review-gated, structured-only  (UNCHANGED)
```

Four declarations, each gated on the same flag as slice 4:

1. **`roster.ts` entry** — `AgentIdentity { id: 'enricher', role: 'agent', isHenry: false, ... }`, placed
   in a world zone (§ Decision V1-B).
2. **`WorkerPersona`** in `default_roster()` — name, traits, tone, `engine: Pending` until the slice-4
   worker is wired (§ Decision V1-C for the persona identity).
3. **`WORKER_DESCRIPTOR`** — a `FeatureDescriptor` with `StateSource::Queryable`, honest copy:
   *"proposes field values about people for your review; never writes a field silently; your manual
   edits always win."* (The standing self-knowledge rule — ships in the slice, not optional.)
4. **`enricher_*` events** — mirror `librarian_describe_*`: `enricher_enrich_started`,
   `enricher_field_proposed`, `enricher_completed`, `enricher_error`. These drive the character's
   `working ↔ idle` state exactly as the Librarian's events do.

## 1.3 The relationship question — peer worker, Henry-orchestrated approval

The dispatch asks: peer agent like the Librarian, or a Henry subagent? The audit makes the answer
structural, not aesthetic:

- For **identity and presence**, the Enricher is a **peer worker** like the Librarian — it does
  autonomous background work on a scheduler cadence (enrichment sweeps), it is not invoked per
  conversation-turn, and it occupies its own world zone. It is a `WorkerPersona`, not a subagent of Henry.
- For **its work product**, every proposal flows through the **same review gate Henry already owns**.
  The Enricher never writes a field that the human (or Henry-policy) has not approved. So: *peer in
  identity, gated in action.* This is exactly the Librarian's shape (peer worker) plus the Steward's
  shape (propose → human-gated), composed.

This means the character's visible "action" is honest: when you see the Enricher working, it is
producing **proposals**, and those proposals appear on the review surface — not silent graph writes.

## 1.4 Decision points — Vision 1 (RULED 2026-06-30)

All contingent on **V1-0 = (a)** (§1.0). Because enrichment is absorbed into the Librarian, most of these
collapse from "new-agent" decisions to "Librarian-task" decisions or become moot.

| # | Decision | Ruling | Note |
|---|---|---|---|
| **V1-0** | Distinct Enricher agent, or Librarian absorbs enrichment? | ✅ **(a) Librarian absorbs**, conditioned on task-level distinctness (descriptor line + egress flag + `librarian_enrich_*` events). No 4th agent. (§1.0) | Gates all of V1; shrinks the rest. |
| **V1-A** | Peer worker or Henry subagent? | ✅ **Peer worker** — *moot under V1-0*: the Librarian is already a peer worker (engine `Pending`). Enrichment proposals still route through Henry's review surface. | Absorbed into the existing Librarian. |
| **V1-B** | World View placement / zone. | ⏸️ **DEFERRED to Jesse** — and largely moot: the Librarian already lives on the mezzanine (`mezzanineLocked`). Not blocking (V1 is gated behind B + 2b + slice 4). | Yours to rule when you design the feel. |
| **V1-C** | Persona identity — name, traits, voice. | ⏸️ **DEFERRED to Jesse** — moot for *creating* a character: the Librarian already has its persona. Enrichment is a new *task* of that persona, not a new identity. | Yours; folds into the Librarian's feel. |
| **V1-D** | Proposal surface (shared with V2-G — ruled once below). | ✅ **Target the Decision Inbox; build against the card seam until the orchestrator is enabled; migrate card→inbox when it goes live** (§3 / §2.1). | Same ruling as the Steward output-fix is following now. |
| **V1-E** | Presence honesty — faked vs real activity. | ✅ **Enforce.** Visible presence reflects **actual** worker activity, never faked — the enrich task drives the Librarian's visible state only when it is really working (the same real-state-not-theater principle throughout; cf. the empty-cave correction). | Applies to the enrich task's visible state. |

**Gating (not a decision, a fact):** all of Vision 1 is gated behind slice 4, which is gated behind
**B + 2b verified** (`UNIFIED_WORKSPACE.md` §6–7). Nothing here builds until the bridge connects people
to the graph and manual editing has proven the write path by hand. This doc adds **no new gate** — under
V1-0 it describes the **enrich task** the Librarian gains *on top of* slice 4 when slice 4 is built.

---

# Vision 2 — The Git Steward, Expanded

> A Steward that **runs CI, resolves errors, dispatches fixes via Henry, and keeps all repos clean,
> organized, and in sync with root folders across devices.** This is a **new epic**, far beyond today's
> read/propose hygiene reporter — and distinct from the #318 output-fix already being built.

## 2.1 Established reality (audited, do not redesign)

| Thing | Where | What it is today |
|---|---|---|
| **Steward safety core** | `crates/goose/src/steward/mod.rs` | `GitOpKind` (Read / BranchCreate / LocalCommit / BranchDelete / HistoryRewrite / ForcePush); `is_protected_branch()` (hard never-touch: `main`/`master`/`develop`/`release-*`/`hotfix-*`); `classify_risk() -> RiskLevel`; `route(kind, branch) -> Routing { Autonomous \| RequiresApproval \| HardBlocked }`; `surface_destructive_proposal(pool, DestructiveProposal) -> ProposalOutcome { CardCreated \| HardBlocked \| NotDestructive }`. |
| **Steward tool** | `agents/platform_extensions/steward.rs` | `propose_git_op` tool. Validates the op is destructive, calls `surface_destructive_proposal`. |
| **Steward recipe** | `goose-server/src/automation/steward.yaml` | **Read/propose ONLY**, `$0` local model. Drafts commit messages / PR descriptions / changelogs / stale-branch reports. **Never executes** a destructive command — escalates via `propose_git_op`. |
| **Decision inbox** | `decisions.rs`, `routes/decisions.rs`, `decision_inbox/` | `Decision { kind, tier (1=Henry\|2=Jesse-only), headline, detail, payload, status, answer, ... }`; `create_decision`; `answer_decision -> (Decision, DecisionProof)`. `DecisionProof` is a non-Copy token **consumed by** `advance_goal_checked`. Tier comes from the `risk_policy` table; unknown classes **fail closed to Tier 2**. (PR #314, merged; stays dormant behind `orchestrator.enabled`.) |
| **Goal dispatch** | `agents/platform_extensions/goal_engine.rs` | `ExternalCliEngine::spawn()` runs `claude`/`codex` in an **isolated git worktree** off a baseline commit; per-worktree post-commit hook captures true work-base (#523); returns `GoalEvidence { worktree_path, baseline_commit, head_commit, work_base_commit }`. |
| **Goal transition guard** | `goal_transition.rs` | `advance_goal_checked(pool, card_id, action, actor, proof, effects)` — the **single legal mutator** of goal lifecycle. Proof-gated, tier-gated, audited. Terminal states trigger worktree reap (#504). |

**The key finding:** the existing stack already contains **every safety primitive the expanded Steward
needs** — a risk classifier, a propose→human-gate seam, an isolated-worktree fix-dispatcher, a
proof-gated transition guard, and a tamper-evident audit chain. **What's missing is entirely additive
and orthogonal to safety:** (a) running CI, (b) a multi-repo registry, (c) wiring detection → fix-goal
dispatch, and (d) cross-device coordination. The right move is to **extend the code-gated model, not
abandon it** — exactly as the dispatch directs.

**Two hard caps the audit imposes on the design:**
1. **The Steward writes no code and merges nothing.** It is a *detector and proposer*. Henry
   orchestrates the fix (existing `ExternalCliEngine`); a worker writes the code in an isolated
   worktree; the existing `advance_goal_checked` review gate stands between the fix and any merge.
   This keeps the "large autonomous actor" decomposed into already-bounded pieces.
2. **There is no multi-repo or cross-device infrastructure today.** Goals are single-repo; the Steward
   is single-repo. Multi-repo is a buildable extension; **cross-device sync is a genuine
   distributed-state problem that needs its own spike** (§2.5).

## 2.2 The autonomous-action surface — what's auto, what's gated

Map each new capability onto the existing `Routing` model rather than inventing a new permission system:

| New action | Mutates a repo? | Proposed routing | Rationale |
|---|---|---|---|
| **Run CI** (invoke the project's test/lint/build) | No | **Autonomous** (read-only observation) | Spends compute, touches no git state. Bound by *resource/cost*, not safety — gate on a concurrency/disk budget (cf. the M1 16GB / `df` discipline), not on human approval. |
| **Detect** (CI red, dirty tree, stale branch, diverged remote) | No | **Autonomous** | Pure observation; produces a proposal, never an effect. |
| **Dispatch a fix** (spawn a worker to fix red CI) | Yes (in an isolated worktree) | **Tier-gated** (§ Decision V2-B) | This is the load-bearing decision. A fix-dispatch is autonomous *code generation*; it must be gated. The fix lands in a worktree, never on a branch the user sees, and goes through the existing review gate before merge. |
| **Merge / push a fix** | Yes (shared branch) | **Always human** (existing `advance_goal_checked` review gate) | Unchanged from today. No fix is ever auto-merged. |
| **Reconcile cross-device** (pull/push/stash to track the remote) | Yes (local git state) | **Proposed → gated** via `surface_destructive_proposal` | Reuses the exact propose→approve seam already built (§2.5). |

The autonomous surface is therefore **detection + CI-running**, both effect-free. Every *effect*
(fix-dispatch, merge, reconcile) routes through a gate that already exists. The "large autonomous actor"
is, by construction, a **loud detector** wired to **already-bounded actuators**.

## 2.3 Multi-repo — registry and per-repo state

Single-repo today; multi-repo is additive:

- **Which repos?** A new explicit registry — `~/.permagent/repos.yaml` (the `agent.yaml` pattern),
  **seeded by scanning the root folder(s)** (e.g. `~/dev/*` that are git repos). "In sync with root
  folders" implies the root folder *is* the unit of management, so the registry mirrors it. Explicit
  config over a hidden auto-scan (the CLAUDE.md rule) — the scan *proposes* additions; the user confirms.
- **State per repo:** a `steward_repos` table (or extend the existing schema) tracking, per repo:
  last CI status + run id, working-tree clean/dirty, ahead/behind vs each remote, last sweep time, last
  proposal. This is the Steward's model of the world it reconciles against.
- **Cadence:** the existing scheduler tick drives a per-repo sweep, one repo at a time under the
  cargo/disk concurrency cap.

## 2.4 Relationship to Henry — Steward proposes, Henry orchestrates

The clean decomposition the audit supports:

```
 Steward (detector, $0 local)              Henry (orchestrator)            Worker (ExternalCliEngine)         Human
 ───────────────────────────              ────────────────────            ──────────────────────────        ─────
 run CI → red ─────────────┐
 build a fix proposal ─────┴──► decision/card ──► dispatch fix-goal ──► fix in isolated worktree ──► review gate ──► approve merge
 detect dirty/diverged ────────► reconcile proposal ──(surface_destructive_proposal)──────────────────────────► approve op
```

The Steward **never** calls `ExternalCliEngine` or `advance_goal_checked` itself — it emits a proposal;
Henry (the orchestrator) owns dispatch and the review loop. This keeps a single orchestration authority
and reuses the entire existing goal pipeline (worktree isolation, work-base capture, push guard, reap).

## 2.5 Cross-device sync — the hard part (SPIKE-FLAGGED)

This is the least-defined piece and the one the dispatch rightly singles out. Designing it honestly:

**What "in sync with root folders across devices" can mean — two very different problems:**

- **(I) Git-state reconciliation** (committed/pushed work): each device's root folder holds the same set
  of repos, each at a git state compatible with the shared remote. **Git already solves the hard part** —
  the remote *is* the authoritative shared state; push/pull *is* the sync protocol; merge/rebase *is*
  conflict resolution. The Steward's job is only to *observe local divergence* (ahead/behind/dirty) and
  *propose reconciling actions* through the gate it already has.
- **(II) Working-tree sync** (uncommitted edits, untracked files, mirror-the-folder-byte-for-byte): this
  is **outside git's model** and is a real distributed-systems problem — file watchers, conflict merge on
  concurrent edits, CRDT-or-equivalent, the Dropbox/Syncthing problem. There is **no primitive for this
  anywhere in the codebase.**

**Recommended design (the simplifying ruling):**

1. **Scope cross-device sync to (I) git-state reconciliation only.** Commit/push is the sync boundary.
   Uncommitted working-tree mirroring (II) is **explicitly out of v1** — flag it as its own research
   spike or rule it out (§ Decision V2-E).
2. **The remote is authoritative; there is no authoritative *device*.** This sidesteps distributed
   consensus entirely: there is no "which laptop wins," because the laptops don't arbitrate — the remote
   does, via git.
3. **Each device's Steward acts only on its own local checkout.** It can *read* that the remote is ahead
   ("device B pushed since my last sweep") and *propose a local pull*; it never reaches across to mutate
   another device. Cross-device awareness is **read-only**; every cross-device *effect* is a local action
   proposed to the local human through the existing gate.
4. **Divergence that git can't fast-forward** (true conflict, force-push detected, diverged history) is a
   **human decision**, surfaced as a `risk_gate` decision/card with the diff — never auto-resolved.

This reduces "cross-device distributed state" to **"per-device git-remote reconciliation,"** which is
buildable on the existing seams. The genuinely hard, undefined problem (II — real-time working-tree
sync) is **named and deferred**, not pretended-solved.

> **SPIKE REQUIRED:** Even scope (I) has open questions that warrant a short research spike before any
> build: multi-remote repos (origin vs fork), repos with no remote, detecting a *device's* identity for
> the per-repo state table, and how a "stale local that the user abandoned" is distinguished from "work
> in progress." Decision V2-D rules the model; the spike sizes the (I) build. Scope (II) is a separate,
> larger spike that should not block (I).

## 2.6 Decision points — Vision 2 (RULED 2026-06-30)

| # | Decision | Ruling |
|---|---|---|
| **V2-A** | Steward's role: actor that writes/merges, or detector/proposer only? | ✅ **Detector/proposer only — RULED INVIOLABLE.** The Steward writes no code and merges nothing. It detects (CI failures, repo issues, sync divergence) and **proposes** (dispatches a fix goal via Henry, surfaces a card). Every actual change goes through Henry's gated dispatch + Jesse's review. This is the hard cap that makes "an agent that runs CI and resolves errors" safe. |
| **V2-B** | The auto-vs-gated line for **fix-dispatch.** | ✅ **Same gates as any Henry goal** — a Steward-proposed fix is goal → worker (isolated worktree) → Review → Jesse approves. **No special auto-approve, no shortcut.** (CI-run + detection remain autonomous because they are effect-free; the *fix* is gated like all goal work.) |
| **V2-C** | Repo registry source. | ✅ Reasonable as designed (`~/.permagent/repos.yaml`, seeded by a root-folder scan that *proposes* additions for confirmation) — **settle exact shape when V2 is scoped for build.** |
| **V2-D** | Cross-device sync model (scope I). | ✅ **RULED as designed: git-remote-as-authority; no authoritative device; each Steward acts only on its own local checkout; cross-device awareness is read-only; un-fast-forwardable divergence = human `risk_gate`.** Git's remote IS the consensus — this sidesteps distributed state. **Proceed to the sizing spike before building.** |
| **V2-E** | Scope of "in sync": committed git state, or byte-for-byte working tree? | ✅ **Scope I (committed/pushed git state) only.** **Scope II (real-time working-tree sync) is RULED OUT for now** — no primitive, hard distributed-systems problem, no clear payoff over commit-and-push. Its own spike *if ever needed*; not built. Scope I delivers "repos in sync across devices" for committed state — the real want. |
| **V2-F** | Is this one epic or several? | ✅ Reasonable as designed (sliced: CI-runner + detection → multi-repo registry → fix-dispatch wiring → cross-device reconciliation after the spike) — **settle sequence when V2 is scoped for build.** |
| **V2-G** | Proposal surface (shared with V1-D — ruled once below). | ✅ **Target the Decision Inbox; build against the card seam until the orchestrator is enabled; migrate card→inbox when it goes live** (§3). Same approach the Steward output-fix follows now. |

---

## 3. Cross-vision notes

- **Shared substrate.** Both visions are *presentation/orchestration layers over already-bounded
  workers*: Vision 1 dresses the slice-4 Enricher as a character; Vision 2 dresses CI/repo hygiene as a
  proposing worker. Neither asks for a new safety primitive — both reuse the persona/roster/worker-
  descriptor stack (Vision 1) and the steward-safety/decision-inbox/goal-dispatch stack (Vision 2).
- **Shared proposal-surface question — RULED ONCE (V1-D = V2-G).** Both visions **target the Decision
  Inbox** (#314), but **build against the card seam** (`cards::create_card`) until the orchestrator is
  enabled — the inbox stays dormant behind `orchestrator.enabled`. **Migrate card→inbox when the
  orchestrator goes live** (the same path the Steward output-fix is on now). The designs are **not blocked**
  on the inbox; the migration couples to Jesse's orchestrator-enable timeline — noted, not blocking.
- **The honesty law applies to both.** A character that visibly "works" must be doing real, bounded work
  (V1-E); a Steward that reports "in sync" must mean a precise, mechanical thing (V2-E), not a vibe.
- **Self-knowledge obligation (standing rule).** Every user-facing capability in either vision ships its
  `<permagent_self>` descriptor in the same change — a `WORKER_DESCRIPTOR` for the Enricher and for the
  expanded Steward's new surfaces, gated on the same flag as the feature.

## 4. What these designs explicitly do NOT do

- Do **not** change the #538 Enricher **bounds** (review-gated, manual-wins, structured-fields-only). The
  character is presentation only.
- Do **not** add any new gate to Vision 1 — it rides on slice 4, itself gated behind B + 2b.
- Do **not** give the expanded Steward the ability to write code or merge. It detects and proposes; Henry
  orchestrates; the existing review gate approves.
- Do **not** design real-time working-tree cross-device sync (scope II). It is named and deferred to its
  own spike, or ruled out.
- Do **not** build anything. This is a design for Jesse to rule on; the slices are authored after the
  rulings, in their own dispatches.

---

## 5. Decision summary (RULED 2026-06-30)

**Vision 1 — Enricher:** **V1-0 = (a) the Librarian absorbs enrichment** (no 4th agent; conditioned on
descriptor line + egress flag + `librarian_enrich_*` events) · V1-A peer-worker (moot, absorbed) · V1-B/C
placement & persona DEFERRED to Jesse (largely moot — the Librarian already has its character) · V1-D
proposal surface → Decision Inbox via card seam (shared ruling) · V1-E presence honesty ENFORCED. *All gated
behind slice 4 (B + 2b) regardless.*

**Vision 2 — Expanded Steward:** V2-A **detector/proposer-only — INVIOLABLE** · V2-B fix-dispatch = **same
gates as any Henry goal** (no shortcut) · V2-C repo registry reasonable (settle at build) · **V2-D
cross-device scope I RULED as designed — proceed to sizing spike** · V2-E scope II **RULED OUT** (own spike
if ever) · V2-F slice sequence reasonable (settle at build) · V2-G proposal surface → Decision Inbox via card
seam (shared ruling). *This is a new epic; the cross-device sizing spike precedes any build.*

**Shared:** proposal surface ruled once (§3) — target the Decision Inbox, build against the card seam,
migrate when the orchestrator is enabled.
