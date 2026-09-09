# Coding-harness excellence iteration DAG

**Date:** 2026-09-09 (America/Halifax). **Planner/verifier:** Fable (session permagent-runtime-b8). **Workers:** Sonnet lanes
(and Opus/Codex/Cursor where cheaper is not capable), one node per lane, disjoint write scopes, evidence-not-assertion.
**Parent:** `CODING_HARNESS_MASTER_PROGRAM_DAG.yaml` → `CODING_HARNESS_EXCELLENCE_DAG.md` (the contract and the nine areas).
**Evidence log:** `permagent-worktrees/HARNESS_LOOP_LOG.md`. **Branch:** `fix/harness-loop-r1` (worktree `harness-loop`).
**Jesse's mandate (2026-09-09):** iterate until all nine areas are Excellent; Fable plans and verifies, never implements.

## Scoreboard (retained runs only; unknown is Unrated, never a pass)

| Rank | Area | 2026-09-08 23:50 | Blocking defect(s) | Owning nodes |
|---|---|---|---|---|
| 1 | Observability | Good | no three-run qualifying record | X-obs |
| 2 | Safety and trust | Good | reviewer shell residual; `delegate` roadmap bypass; unguarded `run_executable_skill` | X-dag, X-skill |
| 3 | Token efficiency | Unrated | no graduated treatment on the DAG harness | X-ab |
| 4 | Cost routing | Unrated→Poor | goal dispatch ignores PACK pins; reviewer bills same-family | X-cost |
| 5 | DAG enforcement | Poor | `delegate` side door; false "dispatched" claim | X-dag |
| 6 | Verification integrity | Poor | reviewer repairs unmeasured live; reviewer shell mutation undetected | X-verify |
| 7 | Cheap-model usability | Poor | no role graduation; protocol turn cost | X-grad |
| 8 | Latency | Poor | harness overhead not separated from model time | X-lat |
| 9 | Coding success | Poor (0/10) | landing (V2, in flight); then per-fixture worker failures | X-land, rounds |

Graduation is the contract table in the parent: Excellent = every required metric on three consecutive qualifying runs, no
open P0/P1, no missing evidence. Moving a threshold after a result needs a recorded Council decision.

## The loop (one round ≈ one build slot)

```text
R0 measure (M-lane: frozen 10-slice + 17-slice, kept scratch, cost ledger)
 └─ R1 classify (M-lane: causal chain per failure, owning area, smallest repair, pinning test)
     └─ R2 repair in PARALLEL, one lane per defect, each in its OWN worktree off the branch tip,
        edit + tests written, then queued for the single build slot in merge order
         └─ R3 review (read-only lane per repair commit; findings must be closed before merge)
             └─ R4 merge order → release build → sign → back to R0
Gate per round: pass-rate not down, USD/solved and tokens reported paired, no trust regression;
a repair that does not move its area's metric on the next R0 is reverted, not kept.
```

Throughput rules (tonight's machine): one compile at a time at -j1, tripwire on df < 25 GiB or swap free < 400 MB;
research, design, forensics, review and test-writing lanes run in parallel without limit; when a second build
host or a remote worktree is available, R2 lanes build there and only the merge build runs here.

## Nodes (one lane each; all can start now except where noted)

### X-land — deliver work (Coding success) — IN FLIGHT (Lane V2, Jesse's option 3)
Policy switch `PERMAGENT_HEADLESS_AUTOLAND` default off; Tier-1 proof accepts the auto-launch actor only when on,
session-bound, single-use. Eval driver sets it (Lane K3). Gate: the slice run lands ≥1 fixture's edits into the
oracle-visible tree; first measurable coding-success number.

### X-dag — close the side door (DAG enforcement, Safety)
While a roadmap is active in a session, a coordinator `delegate` may only run a worker whose tool surface is
non-mutating (reuse the reviewer allowlist mechanism); mutating work goes through goal cards. A coordinator turn
that created a roadmap cannot end while a root goal is Ready-undispatched or a claimed dispatch has no child session
(closes the false "dispatched" claim). Gate: adversarial tests for both; a slice run shows 0 roadmap-bypass passes.

### X-cost — route by pins and role (Cost routing)
Goal dispatch consults `PERMAGENT_PACK_*` and `PERMAGENT_ROLE_*` through the same precedence `delegate` uses; the
reviewer is dispatched to a genuinely different family when one is configured, else the label says so and a receipt
records it. Gate: routing matrix test (deterministic cheapest graduated worker per fixture); paired slice shows
USD/solved not worse; reviewer ledger rows show the intended family.

### X-lat — separate harness time from model time (Latency)
Per turn: time-to-first-tool, tool execution time, model time, harness overhead (everything else), persisted in the
run record and reported by the eval. Gate: overhead p50/p95 reported on every run; first number retained as baseline.

### X-verify — prove the reviewer live (Verification integrity)
Reviewer mutation detector: tree hash before/after the reviewer delegate; a mutating reviewer's verdict is discarded
with a receipt. Vacuous-pass counter in the run record. Gate: slice run reaches review on ≥ half the fixtures; zero
vacuous passes; zero reviewer mutations.

### X-grad — role graduation (Cheap-model usability)
A capability card per provider/model built from retained eval results (micro-suite pass %, tool/DAG-selection %),
consulted at dispatch: ungraduated coordinators are refused, graduated mechanical roles allowed. Gate: the
7B local model is refused as coordinator and accepted for bounded mechanical nodes; glm-4.7's card reflects the
retained runs.

### X-ab — knob measurement (Token efficiency, Cost routing)
When the slice clears 6/10: `ab --knob reviewer-trivial-skip` (approved $4), then repo-map and the premature-done
guard on the DAG harness. Gate: 30-run bar, effect size read per fixture, verdict recorded in evidence.

### X-obs — three-run records (Observability)
Every M-lane run writes the qualification record the parent's `permagent-eval qualify` reads; three consecutive
complete records graduate the area. Gate: `qualify` renders all nine areas from evidence with no Unrated field
caused by missing telemetry.

### X-skill — guard `run_executable_skill` (Safety)
The approval ladder specified in PRIME_RLM_AND_SKILLS.md Part B; no arbitrary shell without a proof. Gate: adversarial
test; recipe cannot reach shell through a skill without approval.

## Rounds

- **Round A (now):** X-land build → M5 measure → classify. In parallel: X-dag, X-cost, X-lat, X-verify design+edits
  in their own worktrees; X-grad and X-skill design docs; X-obs qualification wiring.
- **Round B:** merge order X-dag → X-verify → X-cost → X-lat (each rebased, built once, reviewed) → M6 → X-ab if ≥6/10.
- **Round C+:** per-fixture worker failures from M6, one lane each; X-grad implementation; repeat until three
  qualifying runs per area.

Stop conditions: all areas Excellent; a P0/P1 needing Jesse; spend cap (per round $1.50 measurement, $4 per A/B,
Jesse approves beyond); no candidate improves the Pareto frontier for two rounds.
