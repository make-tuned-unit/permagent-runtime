# Swarm Visions — Two Expanded-Ambition Designs

**Status:** RATIFIED 2026-06-30 — all decision points ruled (V1-0 + V1 + V2 + the shared proposal-surface
question). **Updated 2026-06-30: the Steward is PROMOTED to a full character-agent** — Vision 2 gains a
character layer (§2.0) mirroring Vision 1's roster pattern, a build sequence (§2.6), and new decision
points (V2-H/I/J). Still DESIGN ONLY: nothing builds from this doc. Vision 1 is gated behind B + 2b + slice 4;
Vision 2 is a new epic that needs the cross-device sizing spike first. Zero code, zero runtime files.
Captures two ambitions as architecture so they were ruled before anything builds (the #538 pattern).
**Scope:** Vision 1 — the Enricher as a *visible world-character* (extends slice 4 of
[`UNIFIED_WORKSPACE.md`](./UNIFIED_WORKSPACE.md), #255/#256/#257). Vision 2 — the Git Steward
**promoted to a peer character-agent that owns repo health**: a CI-running, fix-dispatching, multi-repo,
cross-device *detector/proposer* (a **new epic**, distinct from the #318/#552 output-fix already in flight).
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

**Slice-4 build requirement — the egress flag is a CONTROL, not a LABEL (RULED 2026-06-30).** The
Librarian's enrich task reaches **external** sources (web/email); on a local-first product that is a real
capability expansion, not a cosmetic note. When slice 4 is built (gated behind B + 2b), the enrich task
must ship with an **egress control**, meaning both:

1. **Visible when it egresses** — the `librarian_enrich_*` event namespace surfaces external acquisition as
   it happens (the descriptor line + events already give this).
2. **Gateable / disableable / boundable** — the user can **turn external enrichment off entirely**, and/or
   **bound which sources** it may reach. "Egress-flagged" must never collapse to "we labeled it." The
   describe task (local, `$0`, never leaves the machine) is unaffected; only the enrich task is gated.

This is the project's standing principle applied to the one dangerous capability in V1: a dangerous
capability is **visible AND bounded**, not merely annotated. The control ships *in the slice-4 change* with
the enrich task — not as a follow-up.

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

> A Steward that **owns repo health** — a peer character-agent that **runs CI, detects errors,
> dispatches fixes via Henry, and keeps all repos clean, organized, and in sync with root folders across
> devices.** This is a **new epic**, far beyond today's read/propose hygiene reporter — and distinct from
> the #318/#552 output-fix already being built. The user's framing: *"constantly managing my repos,
> keeping everything clean and CI green"* — boring, ongoing work that should be **delegated to a teammate**,
> not done by hand. So the Steward is **promoted from a scheduled recipe to a full character-agent** (§2.0),
> the same way the Librarian is a peer-agent — while the detector/proposer bounds (§2.2, V2-A) stay
> **inviolable**: "owns repo health" means *constantly watching + proposing*, never *autonomously merging or
> rewriting*. The user approves anything that changes a repo.

## 2.0 The Steward becomes a character — PROMOTE, don't absorb (RULED 2026-06-30)

> **DECISION V2-CHAR — RULED 2026-06-30: the Steward is PROMOTED to its own peer character-agent**, filling
> the same four roster slots that make the Librarian a character. It is **not** absorbed into an existing
> agent (the asymmetry vs the Enricher, below) and it is **not** a Henry subagent — it is a **peer** that
> dispatches fix-goals *via* Henry's gated pipeline. The expanded *capability* (§2.1–2.6) is unchanged and
> still **detector/proposer-only**; this section adds only the *character/presentation* layer over it.

### Promote vs absorb — the asymmetry with the Enricher (the reasoning to record)

Vision 1 ruled the **Enricher COLLAPSES into the Librarian** (V1-0 = a): a second Brain-writing worker is
near-identical to the one that already exists, so a separate identity buys nothing and invites the
dead-store drift this codebase keeps fighting. The Steward is the **opposite ruling for the opposite
reason.** The test that distinguishes them:

> **Does an existing agent already do work of this *kind*?**
> - **Enricher → yes.** The Librarian already curates the Brain; "enrich" is more Brain curation. Same kind
>   of work, same write seam (`SafeBrain`). → **ABSORB** (one coherent Brain-keeper, two tasks).
> - **Steward → no.** Repo/CI/git health is a **genuinely distinct role** with **no existing agent to absorb
>   it.** The Librarian curates the Brain; the Reader ingests documents; Henry orchestrates
>   conversation/goals. *None of them owns repo health.* Cramming "watch CI, keep repos clean" into one of
>   them would be the dead-store failure **in reverse** — a distinct role blurred into a mismatched
>   identity. → **PROMOTE** (its own character).

So the same principle (one identity per coherent kind of work; no near-duplicate workers) yields **absorb**
for the Enricher and **promote** for the Steward. The Steward earns its own character because nothing in the
roster already does its job.

### The four character slots (the Librarian pattern, applied to the Steward)

A worker becomes a visible character by filling the **same four declarations** the Librarian fills — there
is **no new rendering/state/wire machinery to invent** (audited in §1.1's "key finding"; the same fan-out
serves render, AgentPicker, camera-follow, and HUDs). Each is gated on the same flag as the character layer:

1. **`roster.ts` `AgentIdentity`** — a 4th entry
   (`ui/command-center/src/components/world/agents/roster.ts`):
   `{ id: 'steward', name: 'The Steward', role: 'agent', trimColor: AGENT_TRIM.steward, isHenry: false,
   mezzanineLocked: false, home: {…}, weathering }`. Today the roster has exactly three (Henry, The
   Librarian, The Reader); the Steward is the fourth. Needs a trim color + a world-zone home (**V2-H**).
2. **`WorkerPersona`** in `default_roster()`
   (`crates/goose/src/config/agent_identity.rs`) — `first_name`, `role`, `traits`, `tone`, `tool_kinds`,
   `availability_check`, `engine`. Today's roster: Claude Code / Codex (both `ExternalCli`) and the Librarian
   (`Pending`). The Steward's `engine` is the open question (**V2-I**): it is a **read-only detector** that
   *proposes* and dispatches fixes *via Henry*, so it does not run `claude`/`codex` itself — its "engine" is
   the scheduler-driven sweep recipe, i.e. `Pending`/a local-detector kind, **not** `ExternalCli`.
3. **`WORKER_DESCRIPTOR`** — **extend the in-flight `git_steward` descriptor #552 is already adding.**
   *Audited reality:* on `origin/main` the Steward exists in self-knowledge only as a **guard**
   (`steward::secret_scan::SELF_KNOWLEDGE_FEATURE` in `GUARD_DESCRIPTORS`) plus the safety core; the
   `git_steward` **worker** descriptor is being added by #552 (branch `steward-output-318`,
   `KNOWN_WORKER_IDS = [..., "git_steward"]`). The character layer **promotes that descriptor into
   `WORKER_DESCRIPTORS`** and expands its copy to describe the watcher/proposer role (`StateSource::Queryable`,
   honest copy: *"watches CI and repo health; proposes fixes for your review; writes no code and merges
   nothing on its own"*). The standing self-knowledge rule applies — it ships **in the same change**.
4. **`steward_*` event namespace** — mirror `librarian_describe_*`
   (`crates/goose/src/events/mod.rs`): `steward_sweep_started / _ci_red / _proposal_created / _completed /
   _error`, driving the character's visible `working ↔ idle` state on the `/events` WS exactly as the
   Librarian's `*_describe_*` events do. **Honest presence (V1-E principle):** the Steward shows "working"
   only when it is *really* sweeping/detecting — never theater.

### World View presence & relationship to Henry

- **Presence:** a peer inhabitant of the World View alongside Henry, the Librarian, and the Reader —
  rendered by the same `ROSTER` fan-out, no crown (`isHenry: false`), its own trim and zone (V2-H). A
  repo-keeper / CI-guardian character: think a vigilant custodian of the repos, visibly *watching*.
- **Henry relationship — PEER, not subagent.** The Steward **detects and proposes**; **Henry orchestrates**.
  When the Steward finds red CI or a repo-health issue, it emits a **proposal** (card/inbox seam, §2.4/§3);
  **Henry** owns the dispatch (`ExternalCliEngine`) and the review loop (`advance_goal_checked`). The Steward
  **never** calls the fix-dispatcher or the transition guard itself. This is exactly the decomposition §2.4
  already specifies — the character layer changes the Steward's *identity/visibility*, not its authority.

### What the character layer is NOT

It is **pure presentation/identity over the bounded capability.** It grants the Steward **zero new
authority**: no code-writing, no merging, no autonomous repo mutation. Promotion makes the Steward
*visible and nameable as the teammate who owns repo health*; the detector/proposer bounds (§2.2, V2-A) are
untouched. A "character that owns repo health" = a character that **constantly watches and proposes**, with
the user gating every change.

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

## 2.6 Build sequence — from today's recipe to the full character-agent

Today's state (the floor this builds up from): **#552 ships the bounded recipe** (`steward.yaml`
read/propose, `$0` local) + the safety core (`steward/mod.rs`) + the `secret_scan` guard + the in-flight
`git_steward` **worker descriptor** (`steward-output-318`, OPEN, do-not-merge). From there, each slice is
**additive and independently shippable**, ordered cheapest-and-safest first:

| Slice | What it adds | Touches | Risk / routing | Gate |
|---|---|---|---|---|
| **(a) Character layer** | The four roster slots (§2.0) — makes the Steward a **visible peer agent**. Pure presentation, **zero new authority**. | `roster.ts` (+1 `AgentIdentity`), `agent_identity.rs` (+1 `WorkerPersona`), promote `git_steward` into `WORKER_DESCRIPTORS`, `events/mod.rs` (+`steward_*`). | None — no repo mutation, no egress. | **Needs Jesse:** V2-H (placement/trim/zone), V2-I (persona + engine kind). Cheap; can land first. |
| **(b) CI-watch + detect** | Run the project's CI; detect CI-red, dirty tree, stale branch, orphaned worktree, diverged remote. Emits `steward_*` events + **proposals only**. | New detector wired to the scheduler tick; reuses §2.2 routing. | **Autonomous** — effect-free observation; bound by *resource/cost* (disk/concurrency cap), not by approval. | Reasonable as designed; **settle at build**. Depends on (a) for visible presence. |
| **(c) Fix-propose via Henry** | The dispatch seam: a detected failure → **proposal** → **Henry** dispatches a fix-goal (`ExternalCliEngine`, isolated worktree) → Review → **user approves merge**. | No new safety code — reuses card/inbox seam (§2.4/§3), `advance_goal_checked`. Steward **never** dispatches/merges itself. | **Tier-gated** (V2-B) — same gates as any Henry goal. **No shortcut.** | INVIOLABLE boundary (V2-A). Depends on (b). |
| **(d) Multi-repo registry** | `~/.permagent/repos.yaml` (root-folder scan *proposes* additions) + `steward_repos` per-repo state (CI status, clean/dirty, ahead/behind, last sweep). Per-repo sweep under the disk/concurrency cap. | New registry + table; extends the (b) detector to loop repos. | Read-only registry; scan **proposes**, user confirms (explicit-config rule). | V2-C reasonable; **settle exact shape at build**. Depends on (b). |
| **(e) Cross-device Scope-I** | Git-remote-as-authority reconciliation (§2.5): observe local divergence, **propose** local pull/push/stash through `surface_destructive_proposal`; un-fast-forwardable = human `risk_gate`. | Per-device local actions only; reuses the destructive-proposal seam. | Every cross-device *effect* is a local op proposed to the local human. | **BLOCKED on the cross-device sizing spike (§2.5 / V2-D).** Scope-II (working-tree sync) RULED OUT (V2-E). Depends on (d). |

**Critical-path notes:**
- **(a) is the cheap unlock** — it is the only slice with *no* new capability and *no* gate-design work; it
  just makes the existing/imminent Steward a visible, named teammate. It can land right after #552 merges
  (it promotes #552's `git_steward` descriptor), pending only the V2-H/I rulings.
- **(b)→(c)→(d)** are the capability build, each additive and each landing behind a gate that **already
  exists** (§2.2). The "large autonomous actor" is, by construction, a **loud detector** (b) wired to
  **already-bounded actuators** (c).
- **(e) does not start until the sizing spike** (§2.5) sizes Scope-I; it must not block (a)–(d).
- **Self-knowledge ships per slice** (standing rule): every slice that adds user-facing capability extends
  the Steward's `WORKER_DESCRIPTOR` copy in the *same* change.

## 2.7 Decision points — Vision 2 (RULED 2026-06-30)

| # | Decision | Ruling |
|---|---|---|
| **V2-A** | Steward's role: actor that writes/merges, or detector/proposer only? | ✅ **Detector/proposer only — RULED INVIOLABLE.** The Steward writes no code and merges nothing. It detects (CI failures, repo issues, sync divergence) and **proposes** (dispatches a fix goal via Henry, surfaces a card). Every actual change goes through Henry's gated dispatch + Jesse's review. This is the hard cap that makes "an agent that runs CI and resolves errors" safe. |
| **V2-B** | The auto-vs-gated line for **fix-dispatch.** | ✅ **Same gates as any Henry goal** — a Steward-proposed fix is goal → worker (isolated worktree) → Review → Jesse approves. **No special auto-approve, no shortcut.** (CI-run + detection remain autonomous because they are effect-free; the *fix* is gated like all goal work.) |
| **V2-C** | Repo registry source. | ✅ Reasonable as designed (`~/.permagent/repos.yaml`, seeded by a root-folder scan that *proposes* additions for confirmation) — **settle exact shape when V2 is scoped for build.** |
| **V2-D** | Cross-device sync model (scope I). | ✅ **RULED as designed: git-remote-as-authority; no authoritative device; each Steward acts only on its own local checkout; cross-device awareness is read-only; un-fast-forwardable divergence = human `risk_gate`.** Git's remote IS the consensus — this sidesteps distributed state. **Proceed to the sizing spike before building.** |
| **V2-E** | Scope of "in sync": committed git state, or byte-for-byte working tree? | ✅ **Scope I (committed/pushed git state) only.** **Scope II (real-time working-tree sync) is RULED OUT for now** — no primitive, hard distributed-systems problem, no clear payoff over commit-and-push. Its own spike *if ever needed*; not built. Scope I delivers "repos in sync across devices" for committed state — the real want. |
| **V2-F** | Is this one epic or several? | ✅ Reasonable as designed (sliced: CI-runner + detection → multi-repo registry → fix-dispatch wiring → cross-device reconciliation after the spike) — **settle sequence when V2 is scoped for build.** |
| **V2-G** | Proposal surface (shared with V1-D — ruled once below). | ✅ **Target the Decision Inbox; build against the card seam until the orchestrator is enabled; migrate card→inbox when it goes live** (§3). Same approach the Steward output-fix follows now. |
| **V2-CHAR** | Promote the Steward to its own character-agent, or absorb it into an existing one? | ✅ **PROMOTE — RULED 2026-06-30.** Repo/CI/git health is a genuinely distinct role with no existing agent to absorb it (the asymmetry vs the Enricher, §2.0). Fill the four roster slots; peer agent, not a Henry subagent. The capability bounds (V2-A) are untouched. |
| **V2-H** | ⏸️ **NEEDS JESSE** — Steward's World View placement: trim color + home zone. | Like V1-B for the Librarian — a feel/placement call. The Reader took ground floor, the Librarian the mezzanine. Not blocking the *capability* build; blocks slice (a)'s final form. |
| **V2-I** | ⏸️ **NEEDS JESSE** — persona identity (name/traits/tone) **and engine kind.** | The character's voice is yours to design (a repo-keeper/CI-guardian). **Engine:** because the Steward is a read-only detector that proposes + dispatches *via Henry*, it does **not** run `claude`/`codex` itself — recommend `engine: Pending`/a local-detector kind, **not** `ExternalCli`. Confirm. |
| **V2-J** | Sequence of the character build vs the capability build. | ✅ Reasonable as designed (§2.6): **(a) character layer first** (cheap, no new authority, lands after #552), then **(b) detect → (c) fix-propose → (d) multi-repo → (e) cross-device after the spike**. **Settle final ordering at build**; (e) blocked on the sizing spike. |

---

## 3. Cross-vision notes

- **Shared substrate.** Both visions are *presentation/orchestration layers over already-bounded
  workers*: Vision 1 dresses the slice-4 Enricher as a character; Vision 2 dresses CI/repo hygiene as a
  proposing worker. Neither asks for a new safety primitive — both reuse the persona/roster/worker-
  descriptor stack and the steward-safety/decision-inbox/goal-dispatch stack.
- **Same four-slot character pattern, opposite identity rulings.** Vision 1 **absorbs** the Enricher into
  the Librarian (same *kind* of work as an existing agent); Vision 2 **promotes** the Steward to its own
  character (a *distinct* role no existing agent owns) — see the asymmetry in §2.0. Both fill (or extend)
  the identical four slots: `roster.ts` `AgentIdentity` + `WorkerPersona` + `WORKER_DESCRIPTOR` + a
  `*_event` namespace. The pattern is shared; whether to mint a new identity is the per-case ruling.
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
  orchestrates; the existing review gate approves. **Promoting it to a character grants it ZERO new
  authority** — the character layer (§2.0) is identity/visibility only; the detector/proposer bounds (V2-A)
  are inviolable.
- Do **not** design real-time working-tree cross-device sync (scope II). It is named and deferred to its
  own spike, or ruled out.
- Do **not** build anything. This is a design for Jesse to rule on; the slices are authored after the
  rulings, in their own dispatches.

---

## 5. Decision summary (RULED 2026-06-30)

**Vision 1 — Enricher:** **V1-0 = (a) the Librarian absorbs enrichment** (no 4th agent; conditioned on
descriptor line + egress flag + `librarian_enrich_*` events) · V1-A peer-worker (moot, absorbed) · V1-B/C
placement & persona DEFERRED to Jesse (largely moot — the Librarian already has its character) · V1-D
proposal surface → Decision Inbox via card seam (shared ruling) · V1-E presence honesty ENFORCED ·
**slice-4 build requirement: the enrich task's egress flag ships as a CONTROL (disableable + source-boundable),
not just a label.** *All gated behind slice 4 (B + 2b) regardless.*

**Vision 2 — Expanded Steward (PROMOTED to a character-agent):** **V2-CHAR PROMOTE** — the Steward becomes
its own peer character-agent (four roster slots, §2.0), *not* absorbed (asymmetry vs the Enricher) and *not*
a Henry subagent. V2-A **detector/proposer-only — INVIOLABLE** (the character grants zero new authority) ·
V2-B fix-dispatch = **same gates as any Henry goal** (no shortcut) · V2-C repo registry reasonable (settle at
build) · **V2-D cross-device scope I RULED as designed — proceed to sizing spike** · V2-E scope II **RULED
OUT** (own spike if ever) · V2-F/V2-J slice sequence reasonable — **(a) character layer first**, then detect →
fix-propose → multi-repo → cross-device-after-spike (§2.6) · V2-G proposal surface → Decision Inbox via card
seam (shared ruling). **NEEDS JESSE:** **V2-H** (World View placement/trim/zone) + **V2-I** (persona +
engine kind — recommend `Pending`/local-detector, *not* `ExternalCli`). *This is a new epic; the cross-device
sizing spike precedes the (e) build.*

**Shared:** proposal surface ruled once (§3) — target the Decision Inbox, build against the card seam,
migrate when the orchestrator is enabled.
