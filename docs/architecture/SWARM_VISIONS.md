# Swarm Visions — Two Expanded-Ambition Designs

**Status:** DESIGN ONLY — awaiting Jesse's rulings. Zero code, zero runtime files. Captures two
ambitions as architecture so they can be ruled before anything builds (the #538 pattern).
**Scope:** Vision 1 — the Enricher as a *visible world-character* (extends slice 4 of
[`UNIFIED_WORKSPACE.md`](./UNIFIED_WORKSPACE.md), #255/#256/#257). Vision 2 — the Git Steward
expanded into a CI-running, fix-dispatching, multi-repo, cross-device actor (a **new epic**, distinct
from the #318 output-fix already in flight).
**Method:** Both designs were audited against `origin/main` (`7dbe8adbc`) in Phase 0; §"Established
reality" sections below are facts confirmed in code, not assumptions. Where the ambition exceeds what
git's model can give honestly, this doc flags a **research spike** rather than hand-waving a design.

---

# Vision 1 — The Enricher as a Character in the World

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

## 1.4 Decision points — Vision 1

| # | Decision | Recommendation | Why it's yours to rule |
|---|---|---|---|
| **V1-A** | Is the Enricher a peer worker or a Henry subagent? | **Peer worker** (Librarian pattern): own `WorkerPersona`, own `roster.ts` presence, scheduler-driven. Proposals gated through Henry's existing review surface. | Sets the entire structural template (which slots get filled) and the org metaphor of the world. |
| **V1-B** | World View placement / zone. | The world has no people/CRM zone yet. Options: ground floor near the Reader (both are "records/intake" workers), or a new "records" alcove. **Recommend ground floor near Reader** for v1; a dedicated zone is a world-bible expansion. | A world-bible aesthetic call — not derivable from code. |
| **V1-C** | Persona identity — name, traits, voice. | Working name **"The Enricher"** (parallels "The Librarian"/"The Reader"); traits e.g. `["meticulous", "deferential", "evidence-bound"]`; voice deferred to the W4 voice phase. **Recommend confirm a name, defer voice.** | Persona/naming is yours; it shapes how the character reads to a user. |
| **V1-D** | Proposal surface: decision-inbox vs Kanban cards. | #538 says "decision-inbox seam." But the Steward routes to **`cards::create_card` (v1)** today because the decision-inbox stays dormant behind `orchestrator.enabled` (see Vision 2 §2.1). **Recommend the Enricher follow whatever the Steward uses at build time** — one surface for all agent proposals, not two. | Couples to the orchestrator-enable timeline; a product-coherence call. |
| **V1-E** | Presence-before-worker? Ship an inert character early as a teaser, or only when the worker is real? | **Ship presence + worker together.** A character visibly "working" with no real enrichment behind it violates the honesty law (cf. the empty-cave correction). The roster entry lands in the same slice-4 change. | A direct application of the project's honesty law — confirm you agree. |

**Gating (not a decision, a fact):** all of Vision 1 is gated behind slice 4, which is gated behind
**B + 2b verified** (`UNIFIED_WORKSPACE.md` §6–7). Nothing here builds until the bridge connects people
to the graph and manual editing has proven the write path by hand. This doc adds **no new gate** — it
describes the character layer that rides *on top of* slice 4 when slice 4 is built.

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

## 2.6 Decision points — Vision 2

| # | Decision | Recommendation | Why it's yours to rule |
|---|---|---|---|
| **V2-A** | Steward's role: actor that writes/merges, or detector/proposer only? | **Detector/proposer only.** It runs CI + detects + proposes; Henry orchestrates fixes via the existing `ExternalCliEngine`; the existing `advance_goal_checked` review gate stands before every merge. Extend the code-gated model, don't abandon it. | Defines the entire safety posture of a large new autonomous capability. |
| **V2-B** | The auto-vs-gated line for **fix-dispatch.** | CI-run + detection = **autonomous** (effect-free). Fix-*merge* = **always human**. The open question is fix-*dispatch*: always Tier-2 (Jesse approves each), or a Tier-1 Henry-policy auto-dispatch for a narrow trivial class (e.g. dep-bump CI fixes, formatting)? **Recommend Tier-2-for-all in v1**, add a Tier-1 allowlist later once trust is established. | This is *the* autonomy/trust boundary — exactly the call the human-in-the-loop architecture reserves for you. |
| **V2-C** | Repo registry source. | **`~/.permagent/repos.yaml`, seeded by a root-folder scan that *proposes* additions** for your confirmation. Explicit config over hidden auto-scan. | Defines the unit of "all repos" and how new ones enter management. |
| **V2-D** | Cross-device sync model. | **Git-remote-as-authority; no authoritative device; each Steward acts only on its own local checkout; cross-device awareness is read-only; un-fast-forwardable divergence is a human `risk_gate`.** Scope to git-state reconciliation (I). | The crux of the hard problem — and the ruling that makes it buildable instead of a distributed-consensus project. |
| **V2-E** | Scope of "in sync": committed git state, or byte-for-byte working tree? | **Committed/pushed git state only (scope I).** Working-tree mirroring (scope II) is **out of v1** — its own research spike, or rule it out entirely. | Decides whether this is a buildable epic or a multi-quarter distributed-systems research project. |
| **V2-F** | Is this one epic or several? | **Several slices, sequenced:** (1) CI-runner + detection (autonomous, low-risk), (2) multi-repo registry, (3) fix-dispatch wiring (the V2-B gate), (4) cross-device reconciliation (after the V2-D spike). CI-runner ships value first and standalone. | Sequencing/sizing a large epic; the cross-device piece must not block the rest. |
| **V2-G** | Proposal surface (shared with Vision 1, V1-D). | The Steward routes to `cards::create_card` (v1) today because the decision-inbox is dormant behind `orchestrator.enabled`. The expanded Steward's richer proposals (CI-fix goals, reconcile ops) **want the decision-inbox** (typed payload, tiering, proof gate). **Recommend: this epic is gated on enabling the orchestrator + performing the documented `cards → decisions` swap.** | Couples the epic to the orchestrator-enable timeline you control. |

---

## 3. Cross-vision notes

- **Shared substrate.** Both visions are *presentation/orchestration layers over already-bounded
  workers*: Vision 1 dresses the slice-4 Enricher as a character; Vision 2 dresses CI/repo hygiene as a
  proposing worker. Neither asks for a new safety primitive — both reuse the persona/roster/worker-
  descriptor stack (Vision 1) and the steward-safety/decision-inbox/goal-dispatch stack (Vision 2).
- **Shared proposal-surface question.** V1-D and V2-G are the same question — *which human-review surface
  do agent proposals land on?* — and should be ruled once. Today it is the Kanban card; the richer answer
  is the decision-inbox once the orchestrator is enabled.
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

## 5. Decision summary (for ruling)

**Vision 1 — Enricher character:** V1-A peer-worker · V1-B world placement · V1-C persona identity ·
V1-D proposal surface · V1-E presence-with-worker. *All gated behind slice 4 (B + 2b) regardless.*

**Vision 2 — Expanded Steward:** V2-A detector/proposer-only · V2-B fix-dispatch tier line · V2-C repo
registry · **V2-D cross-device model (the crux)** · V2-E sync scope (spike-gated) · V2-F slice sequence ·
V2-G proposal surface. *Cross-device scope (I) needs a sizing spike; scope (II) needs its own larger
spike or a ruling-out.*
