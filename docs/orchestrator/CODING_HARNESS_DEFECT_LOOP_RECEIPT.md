# Coding harness continual-defect controller receipt

**Date:** 2026-09-04 (America/Halifax)  
**Scope:** master-program control plane and evaluator CLI only  
**Status:** focused gates passed; P4 remains active

## Defect

The improvement program documented a forward path to qualification, but its
controller could only transition an active node to passed. An integrated or
held-out regression therefore had no machine-enforced way to reactivate the
earliest owning node and invalidate downstream state.

## Repair

- Added the M0-M7 exceptional-state defect graph to
  `CODING_HARNESS_EVAL_LOOP.md` and made it an overlay on every program node.
- Added `p7b_continual_defect_loop` before held-out qualification so the loop
  itself must retain three clean frozen-slice iterations.
- Added `ProgramDag::reopen_for_regression`, which requires a retained reason,
  atomically reactivates a passed owner, resets all descendants to `planned`,
  preserves passed prerequisites and independent branches, and cannot bypass
  human or spend-cap approval.
- Added `permagent-eval program reopen` with read-only-by-default behavior and
  atomic `--out` / `--in-place` persistence.

This adds no scheduler, ledger, or memory store. Runtime execution remains the
approved Permagent roadmap and evidence remains in the existing
Spectral/session path.

## Verification

| Gate | Result |
|---|---|
| Master manifest validation/frontier | passed; active node remains `p4_task_budget_boundary` |
| Program controller tests | 13 passed, 0 failed |
| Program CLI tests | 5 passed, 0 failed |
| CLI surface | `program reopen --help` exposes required manifest, node, and reason plus explicit persistence options |
| Formatting | `rustfmt` and `git diff --check` passed |

The subsequent full `permagent-eval` suite observed 158 passes and one
in-flight dispatch-inventory test failure while the inventory worker was
changing that file. That failure is retained as a separate M4 defect and was
routed to its owning B4.8A worker; this receipt does not conceal or claim the
integrated suite as green.
