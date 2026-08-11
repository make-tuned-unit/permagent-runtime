# Goal Graph Engineering — deep review and design

**Date:** 2026-08-05 · **Status:** PROPOSED (Phase 0 shipped)
**Trigger:** the overnight goal-dispatch frenzy — three self-recreating cron
recipes, ~$20 of burned API credits, eight goals falsely "abandoned," three
worker commits orphaned on detached HEADs, and worker code pushed to
`origin/main` before any review.

This doc is the synthesis of a three-lane audit (goal lifecycle, scheduler
guardrails, Brain graph capability) with file:line evidence, and the design
that makes goal-based work organized, verified, and context-rich enough to
trust. The organizing idea, per Jesse's direction: **graph engineering** —
the work system's nouns (goals, attempts, commits, sessions, workers, checks,
evidence) become first-class graph nodes with typed, provenance-carrying
edges, and every consumer (dispatch briefs, verification, review, the DAG
driver, Henry's own reasoning) reads from that graph instead of from prompt
folklore and JSON blobs.

---

## 1. What the audit found (condensed; each item verified with file:line)

### Lifecycle defects
- **W1 — Zombie dispatch.** `goal_advance action=dispatch`
  (`orchestrator.rs:1917-1936`) moves Ready→InProgress **without spawning a
  worker** — no session, no tracker, no baseline. The real dispatcher
  (`dispatch_goal_fn:748`) is unreachable from the tool. These zombies are
  what the resume sweep later "abandons."
- **W2 — Push-to-main before verification.** On worker success the engine runs
  credential-scan → **`git push origin HEAD:main`** (`goal_engine.rs:988-1044`)
  → collect evidence. Completion checks and human review run **after** the
  code is on main. `approve` merges nothing (there is nothing left to merge);
  `reject` has no revert path. Push failure is `warn`-logged and swallowed —
  which is the only reason last night's three commits *weren't* on main.
- **W3 — The worker never sees its acceptance criteria.** The whole brief is
  `Goal/Description/Project/root` (`orchestrator.rs:832-836`). Criteria are
  compiled into enforced checks (`checks_from_acceptance:3187`) and fed to the
  verifier — but withheld from the agent being graded. Unmappable criteria are
  silently dropped (`:3204-3210`).
- **W4 — Failure destroys evidence.** `collect_evidence` runs only on success
  (`goal_engine.rs:926`). Timeout/error/credential-block paths keep only a
  `last_error` string; committed work strands in an unreferenced worktree, and
  retries start cold (`requeue_goal` preserves `last_error` but nothing
  re-injects it).
- **W5 — Restart recovery fabricates success.** A re-attached session that
  stops being busy is promoted to Review with *assumed* success and no
  evidence (`resume_single_goal:4885-4890`).
- **W6 — The DAG is declared but never driven.** `promote_eligible_dependents`
  promotes Triage→Ready but never dispatches; the only promote-and-dispatch
  path is the `resume_roadmap` tool (`orchestrator.rs:2474`). After the root
  wave, roadmaps stall — the "dispatches automatically" user message
  (`:2416-2419`) is false. When dispatch does run it fires every Ready goal at
  once, no concurrency cap. **Henry's every-minute `dag-wave-sequencer` cron
  was a workaround for this missing scheduler.**
- **W7 — The verifier grades against an always-empty field.**
  `claimed_evidence` resolves to `"(none provided)"` for every goal
  (`verification/mod.rs:252-265`); the worker's actual stdout summary is
  stored but never shown to the verifier.
- **W8 — Provenance gaps.** LLM-authored goals are stamped `created_by:
  "user"`; the external CLI's actual model is never recorded; root-dispatch
  failures inside `create_roadmap` are swallowed.

### Scheduler/automation defects (Phase 0 — FIXED in cca345f5b)
The self-replication loop had seven conditions: no cron cadence floor; agent
schedule creation unguarded in `Auto` mode; ids minted from titles (near-
duplicates install cleanly); the deletion tombstone read only by the 3
hardcoded starters; scheduled sessions inheriting the **full** extension set
(orchestrator + recipe_author) via the `resolve_extensions_for_new_session`
fallback; `recipe_author` reaching the scheduler through a process-global
static; and mid-stream provider failures returning `Ok` so dead runs showed
`last_status=ok`. Phase 0 shipped: 15-min interval floor (add-time **and**
load-time), agent-created schedules land paused behind user approval,
stream failures propagate, 3-consecutive-failure auto-pause, headless
extension denylist, `run_now` outcome recording.

### Graph capability (what exists to build on)
The spectral store already provides everything a work graph needs and
permagent uses almost none of it: content-addressed entity ids
(`blake3(type:canonical)` — `entity_id("goal", card_id)` needs no registry),
typed ontology-validated confidence-scored edges, **bi-temporal supersession**
(`asserted_at`/`valid_to`/`superseded_by` — built, tested, entirely unused),
BFS `neighborhood()`, and a proven provenance-first mint pattern
(`project_graph.rs:66-113`, `entity_provenance` v23, prune-safe reconciler
`state.rs:409-455`). Meanwhile: goals/sessions/commits/workers exist only in
`cards.metadata_json` blobs; `depends_on` is a JSON array nobody can join or
traverse; `sessions` has no `project_id`/`card_id`; lessons infrastructure is
dead code; goal outcomes never reach the Brain; and the dispatched worker —
the agent doing the actual work — receives zero recalled context of any kind.

---

## 2. Design principles

1. **Deterministic where determinism suffices.** The DAG driver, guards,
   floors, and watchdogs are daemon code — never an LLM cron. (An LLM cron
   *watching* for runaway LLM crons is how we got here.)
2. **Relational tables stay authoritative; the graph is a projection** —
   exactly how `works_on` projects `project_people`, with an idempotent boot
   reconciler. No dual-write consistency problem.
3. **Injected context is data, not instructions** — quoted, provenance-
   carrying, confidence < 1.0, overridable (`FAILURE_LEARNING_LOOP.md` §0;
   the −9.2pp consolidation regression is the standing warning). Flag-gate
   and eval-gate every injection change.
4. **Evidence over assertion.** Every claim in a review decision must cite a
   machine artifact (check output, commit id, diffstat) reachable from the
   goal node.
5. **Nothing lands without a gate.** Work reaches `main` through a branch the
   daemon controls, after checks pass, when the user approves — never before.

## 3. The work graph

**Ontology additions** (`~/.permagent/brain/ontology.toml` — pure TOML, no
code): entity types `goal`, `attempt`, `session`, `commit`, `worker`,
`check`; predicates `depends_on`, `blocks`, `attempted_by`, `produced_by`,
`evidences`, `verified_by`, `supersedes` with matching domain/range.

**Bridge:** `cards.graph_entity_id` (migration, mirroring
`projects.graph_entity_id`), minted `entity_id("goal", card.id)` with
`record_provenance(Runtime)` written first.

**Projections** (each at its existing authoritative seam):
- `depends_on` JSON → `depends_on` triples, at `write_depends_on_audited`
  (`goal_transition.rs:1268-1290`) + boot reconciler.
- Dispatch → `attempt` node + `attempted_by` (goal→worker) + session node, at
  the dispatch transaction. Re-attempts **supersede** the prior attempt edge
  (this is what the bi-temporal columns are for) — the attempt chain becomes
  queryable history instead of an overwritten blob.
- Evidence → `commit` nodes + `produced_by`/`evidences` triples at
  `set_goal_dispatch_evidence` (`cards.rs:1313`); check runs → `check` nodes +
  `verified_by` at the verifier write (`verification/mod.rs:281`).
- Outcomes → `remember_with` on completion/failure/verdict so goal history
  enters recall. Today 70 goal cards contribute nothing to future dispatches.

**Query duty:** the graph answers, deterministically, the questions the
system currently cannot ask: *what did this goal's dependencies produce*
(dep goal → commits + verdicts), *what happened on prior attempts* (attempt
chain), *which in-flight goals touch the same files* (goal → declared/actual
paths via evidence), *which worker/model produced what quality* (worker →
attempts → verdicts).

## 4. The landing path (kills W2, finishes the rescue-branch story)

1. **Branch at dispatch**: `git worktree add -b goal/<card8>-<slug>` from a
   freshly fetched `origin/main` (fallback: current behavior when no remote),
   replacing `--detach`. Branch name + baseline recorded in the dispatch
   transaction — the pointer to the work survives every failure mode.
2. **Daemon pushes the goal branch** (never main) after the credential scan:
   work becomes durable and reviewable immediately. The worker keeps its
   existing "commit, don't push" contract.
3. **Checks gate Review**: completion checks run in the worktree **before**
   the card enters Review; their stdout/exit codes are persisted and cited in
   the `approve_review` decision (fixes W7's empty `claimed_evidence` too —
   pass `worker_summary` + check artifacts to the verifier).
4. **Approve = land**: fast-forward/rebase onto main, or open a PR on
   conflict. **Reject = archive branch with reason**, re-inject
   `review_notes` + prior evidence into the rework brief (fixes the cold
   retry of W4).
5. **Failure preserves evidence**: every terminal path calls
   `collect_evidence` (it only needs the worktree + baseline); requeue stamps
   `last_attempt_work` and retries dispatch onto the same goal branch.

## 5. The deterministic DAG driver (kills W6, replaces Henry's cron)

A daemon loop (tokio interval, ~60s, zero LLM) that: promotes Triage goals
whose deps are Complete (existing predicate), dispatches Ready goals up to a
**concurrency cap** (default 2) with **file-collision serialization** (skip
dispatch when an in-flight goal's evidence paths intersect the candidate's
declared paths), honors budgets, and emits one activity event per action.
`goal_advance action=dispatch` either routes to the real dispatcher or is
removed (W1). Henry's tools shrink to intent: create/approve/cancel/query —
the driver does the driving.

## 6. The dispatch brief contract (kills W3, feeds from the graph)

Every worker brief contains, in order: goal title/description; **acceptance
criteria verbatim** plus the compiled checks it will be graded by (command
strings); dependency context (each dep goal: title, verdict, commits,
diffstat — from the graph); prior-attempt context on retry (last_error,
handoff diff, review_notes); project conventions (CLAUDE.md/AGENTS.md head,
build/test commands); sibling in-flight goals (titles + paths) with a
"do not touch" note; and the landing contract (branch name, push rules,
what happens at Review). Recall-derived additions follow principle 3
(data-not-instructions, flag-gated, eval-gated).

## 7. Phasing

| Phase | Scope | Status |
|---|---|---|
| 0 | Scheduler guardrails + resume-sweep guard | **SHIPPED** (cca345f5b, 76ca931bf) — needs install |
| 1 | Landing path: branch-at-dispatch, daemon pushes goal branch, checks gate Review, approve=land, evidence on every terminal path | next |
| 2 | Deterministic DAG driver + dispatch concurrency/collision gates; delete W1's bare transition | after 1 |
| 3 | Brief contract v1 (criteria + deps + attempts + conventions, no Brain recall yet) | with 2 |
| 4 | Work graph: ontology + bridge + projections + reconciler; verifier reads evidence; goal outcomes → Brain | after 1-3 |
| 5 | Graph-derived brief context + recall unification (`CascadeHits` keeps `neighborhood`), eval-gated | last |

Fix-in-passing anytime: W5 (stop assuming success on resume — route through
the worktree-evidence path), W8a (`created_by: "agent"`), W8c (record the
external CLI's model).
