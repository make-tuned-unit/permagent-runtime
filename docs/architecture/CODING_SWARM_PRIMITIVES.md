# Coding-swarm primitives — the Ruflo review and what Permagent took from it

2026-08-10. Jesse reviewed Ruflo-style multi-agent coding harnesses and asked
what to steal. Verdict: six of its eight primitives already existed here; the
mapping and the three gaps we closed are recorded so this is not re-derived.

## Primitive → existing seam

| Ruflo primitive | Permagent seam |
|---|---|
| plan | `decompose_roadmap` / `create_roadmap` |
| spawn_worker | goal dispatch via the worker registry (#59) |
| assign_task | goal cards |
| share_artifact | goal worktrees + branches, execution receipts, `map_query` |
| evaluate | verification gate (`goose-server/src/verification/`), machine-clamped rubric |
| request_revision | `steer_goal` (live workers) / review reject (completed work) |
| approve | Decision Inbox `approve_review` |
| commit | `land_approved_goal` (FF-only) |

## What was added (this pass)

1. **Consensus review panel** — `verifier.json` gains `panel_models`; the
   verification gate runs every panelist on the same evidence and folds
   per-rubric-question by majority, ties resolving to the worse grade
   (`verifier::fold_panel`). Degraded panelists stay in the denominator as
   Uncertain. Per-panelist verdicts persist in
   `dispatch_evidence.verdict.panel`. Empty config = single-model gate,
   byte-identical.

2. **Specialist role briefs** — `platform_extensions/role_brief.rs`:
   debugger / security / architect mandates, orthogonal to the worker roster
   (any worker can wear any brief). Selected via `goal_advance`'s `role`
   argument; persisted sticky in `metadata_json.dispatch_role`; prepended
   mandate-then-task in the dispatch instructions.

3. **Review-fail → debugger proposal** — a verification FAIL files a `choice`
   decision (payload marker `proposal: "debug_dispatch"`) offering a
   debugger-role re-dispatch with the failing check evidence. Proposal-only:
   the effect arm (decisions_effects) runs only when a human picks the
   option, and it delegates the state change to the existing
   `approve_review`-reject flow (one source of truth for rework, attempt
   budget, and at-cap parking), after persisting the sticky mandate.

4. **Failure-learning return leg** — `decompose_roadmap` injects up to 3 open
   incidents (`incidents::format_incident_context_block`) as raw quoted
   evidence — no distillation (the −9.2pp weak-intermediate lesson from
   FAILURE_LEARNING_LOOP.md). Inert when no incidents are open; the
   `incidents` INFO line is the A/B signal.

## Deliberately not done

- **Lesson pool wiring** — gated by FAILURE_LEARNING_LOOP.md's own paired-eval
  rule ("if they do not beat the raw artifacts, stop and keep Phase 2").
- **Mesh-federated workers** — sound (coding federation is latency-tolerant,
  unlike split inference), but gated on the live full-loop proof.
- **Topology zoo / permanent specialist roster** — roles are briefs, not
  agents; Ruflo's own caveat.
