# Lane R62cd: drain budget, shared oracle, bytecode, already-satisfied goals

Branch `lane/R62cd` in `/Users/j/Documents/dev/permagent-worktrees/laneR62cd`. It starts at `53de25c5` with `lane/R56a` (754c754a + report) merged in.

This lane only edited files. **No cargo was run.** Every changed file passes `rustfmt --edition 2021 --check`. The coordinator runs the gate filters listed at the bottom.

Line references are at `e640ac61` unless a line says otherwise. Evidence comes from `loop-scratch/A61_R60_MISSES.md` (Misses 1, 4 and 5), from the R56a report, and from M46 r61 s1.

## Commits

| sha | piece |
|---|---|
| `17dc117c` | Merge of `lane/R56a`. One conflict, in `crates/goose-cli/src/recipes/search_recipe.rs`: HEAD's R57a/R59a recipe assertions and R56a's write-scope WAIT assertion were added side by side. I resolved it by keeping all three. The yaml auto-merged, and every asserted phrase is present. |
| `910ab3f7` | R62c: the headless-unblock continuation gets its own dispatch, and a never-dispatched goal gets a first dispatch before the drain quiesces. |
| `aeaf0936` | R62c: the drain adopts a Review goal that no completion handler surfaced. This covers the r61 s1 evidence the coordinator added. |
| `709f2f28` | R62c / R56a root cause 3 option (b): goals that share a whole-suite oracle are chained, and the oracle is kept only on the last goal. |
| `94cd6e3f` | R62d: bytecode and caches are kept out of goal commits and out of the uncommitted-work check. |
| `e640ac61` | R62d: a goal whose work is already satisfied at its baseline is completed instead of held forever. |

## R62c: the drain

### Confirmed: R56a's refund covers the scope-refusal drain path
The drain's redispatch attempts go through the installed headless hook to `dispatch_goal_fn`. When a write scope conflicts, that function returns `"Goal '…' remains Ready and was not dispatched: write scope conflicts with active goal …"` (orchestrator.rs `dispatch_goal_fn`, the `write_scope_conflict` call). The hook passes the error through unchanged. `redispatch_ready_goals` matches the error with `refusal_is_scope_wait` and refunds the attempt. That was A61 Miss 1's drain on e400/e406, and the fix is already in the merge.

### Root cause 1: the headless-unblock continuation lost to the redispatch cap (A61 Miss 4)
- R18g (`maybe_headless_unblock_verify_gap`, :23328) fires only on a goal's second hold. By then the drain had already re-dispatched the card `max_redispatch` (2) times.
- `next_drain_action` (:16730) saw a Ready card with its budget spent and returned Park. The old `next_dispatch_actions` also skipped the card on every later sweep.
- Result: no fifth session ran, and the run ended 157 s into a 500 s timeout.

**Fix:**
- `DrainSweepLedger` replaces the plain count map. `next_drain_dispatches` (:16872) picks a `DrainDispatchKind` for each card:
  - `UnblockContinuation` neither checks nor spends the budget.
  - `Budgeted` is the unchanged Y6 behaviour.
  - `FirstDispatch` is described under root cause 2.
- "Continuation not yet used" is tracked durably by `headless_unblock_continuation_pending` (:16823). It is true while there is a `resumed_by_headless_unblock_at` stamp and no `dispatched_at` after it. Every dispatch stamps `dispatched_at`, so the continuation counts as used no matter who starts it.
- R18g's own counter keeps the stamp to once per goal. The `continuation_tried` set limits a refused continuation to one try per drain.
- The poll arm treats an unused continuation as budget left, so the card is Requeued instead of parked.

### Root cause 2: the drain quiesced with its wall budget unspent (A61 Miss 1)
A Ready goal of the run's own roadmap had its budget spent by refusals and was never dispatched. After two idle passes the drain exited with 480 s of its 540 s cap left.

**Fix:**
- On a quiescing pass (`let quiescing =`, :17532: nothing pending, nothing newly discovered, no completion in flight), a run-scoped Ready card that `goal_never_dispatched` (:16843) accepts gets one `FirstDispatch` per drain. `goal_never_dispatched` requires no dispatch stamp, no receipt, no attempt, and that the card is not awaited.
- The headroom gate and the completion gate still apply.
- A scope wait refunds whichever allowance it used.
- `max_redispatch = 0` still turns off the whole sweep.

### Root cause 3: a Review card nobody would ever land (M46 r61 s1, `harness-testfix-inventory-suite-green`)
What happened:
- The coordinator called `goal_advance{action: review}` while the worker was still running (audit seq 5, 02:49:37.878). The worker session ended at 02:49:49 and its checkpoint was at 02:49:50.
- The manual Review arm (:9559) moves the card to Review but creates no `approve_review` decision and never auto-lands.
- The worker's `handle_goal_completion` then saw "not in_progress — manual intervention assumed" and skipped.
- The drain Parked, and `handle_goal_timeout` does nothing for a card in Review. The drain went idle and exited: `0 finished, 1 still running`, `ready_unconsumed(commit=0669b43)`.

So the drain did quit with wall budget left while the goal sat in Review. **But no reviewer or auto-land was running.** Waiting longer could not have helped; it would only have spent the cap. I therefore implemented adoption rather than a wait.

**Fix:**
- `mint_approve_review_decision` (:15010) is extracted from the completion handler's Review arm, and the handler now calls it.
- `adopt_orphaned_review` (:15136) acts only on a goal that has never had an `approve_review` decision. It creates the decision, bound to this coordinator, and runs `auto_land_headless_goal` with all of its existing gates.
- The drain adopts once per card, only when the receipt is terminal, no completion is in flight, and headless auto-land is on for this coordinator (`DrainRedispatch.auto_land`, which every initializer now sets).
- It then polls the card again. Landed or parked reads as Finished; rejected back to Ready reads as Requeued and is re-dispatched. A card still in Review falls through to the old Park.

## R56a root cause 3: a shared whole-suite oracle
**Root cause:** file-disjoint roots all carried the same criterion, `python3 test_checkout.py`. Each root runs in its own worktree with only its own fix, so none of them can ever pass that criterion (A61 Miss 5 msg 40-41/55; r58 q3).

**Fix:** `chain_goals_sharing_a_whole_suite_oracle` (:2218) runs at the end of `ground_roadmap_goals`, after the per-goal rewrites, so the empty-list fallback cannot add the oracle back.
- **What counts as a shared oracle:** a criterion that is both command-shaped and test-shaped (`is_whole_suite_command_criterion`, :2103) and appears identically on two or more goals. The comparison ignores case, whitespace and a trailing full stop.
- **How the goals are chained:** in the roadmap's own topological order, each goal gains a dependency on the one before it. The oracle stays only on the last goal. Because every new edge points forward in that one order, no cycle can form.
- **What earlier goals keep:** their other criteria. A goal that would otherwise be left with no criterion gets a `probe:*` check built from its own named non-test files (`scoped_probes_for_chained_goal`, :2134):
  - `py_compile` for Python
  - `node --check` for JavaScript
  - `bash -n` for shell scripts
  - `cargo check` for Rust when there is a `Cargo.toml`
  - otherwise a file-exists probe
- **When no probe can be built:** the group is left as proposed, with a `decline` receipt.

The protected invariant `ground_roadmap_goals_keeps_a_split_when_every_goal_names_a_real_file` still holds: both goals are kept, they gain an order, and the first gets `cargo check`.

## R62d

### Bytecode and caches (A61 Miss 5)
**Root cause:**
- `checkpoint_uncommitted_changes_in` (:19719) runs `git add -A`, and so do the workers. Together they committed `__pycache__/*.pyc` (d190c7c).
- Test runs then rewrote the now-tracked .pyc files. `worktree_has_uncommitted_work` (:19606) runs `status --porcelain --untracked-files=no`. I confirmed it does **not** pass `--ignored`, so ignored files were never the problem; tracked artefacts were. The check read the rewritten .pyc files as uncommitted work and held a goal whose verify had passed.

**Which exclude file git reads:** I checked this against git 2.55. A linked worktree's `.git/worktrees/<id>/info/exclude` is **ignored**. `git rev-parse --git-path info/exclude` resolves to the common dir's file, and that is the only exclude file git consults for the worktree.

**Fix:**
- `install_generated_artefact_excludes` (goal_engine.rs:1244) is called from `create_goal_worktree` (:1133), which every creation site goes through. It is best-effort, idempotent and append-only. It adds `__pycache__/`, `*.pyc`, `.pytest_cache/` and `node_modules/.cache/` to the file `--git-path` reports. It adds `target/` only when nothing under a `target/` directory is tracked.
- `is_generated_artefact_path` (:1209) matches the same set of paths.
- `worktree_has_uncommitted_work` now drops artefact entries (`without_generated_artefact_entries`, :19642). That filter reaches the hold decision, salvage's uncommitted-extra gate, and the read-only `worktree_mutated` check together.
- The checkpoint makes no commit when only artefacts are dirty, and it unstages tracked artefacts.

### Goal already satisfied at its baseline (A61 Miss 5, inventory root `_5/_6/_7`)
**Root cause:**
- A sibling's commit had already put the fix on trunk. The inventory worker got a genuine verify PASS (msg 139-140) and had nothing to commit.
- `decide_goal_done_hold` (hold_done.rs:437) and `decide_goal_hold` (:145) only allow work that is both verified and committed, so the goal was held forever.
- Even an Allow would have been parked at approval: `land_approved_goal` (decisions_effects.rs:484) refuses an empty rolling gate with `nothing_unlanded`.

**Fix:**
- `goal_already_satisfied_at_baseline` (hold_done.rs:497) is pure. It requires every one of these:
  - the goal is a mutating goal
  - a GENUINE verify in this attempt's own worker session, never absent or vacuous
  - no commits
  - a clean tree, untracked files included, with artefacts set aside
  - a trusted trunk with nothing unlanded
  - the goal's `write_scope` files, or the whole tree when none is declared, are identical between HEAD and trunk HEAD
  - no live worker
- `maybe_hold_review` (:22590) consults the rule only when the goal would otherwise be held as verified-but-uncommitted (`already_satisfied_at_baseline_receipt`, :22466). On Allow it records a protected `already_satisfied` receipt.
- `land_approved_goal` changes only on the `nothing_unlanded` refusal: a receipt that still names the current worker session at the current head (`already_satisfied_receipt_still_holds`, :22549) completes the goal with the receipt's note and **no landing outcome**.
- The way out of every other shape is unchanged. A stale or mismatched receipt falls back to the existing refusal and unblock decision.

## Gate filters (the coordinator runs these; I did not)
```
# R56a (merged)
cargo test -p permagent --lib derive_disjoint_write_scopes
cargo test -p permagent --lib materialized_goals_carry_the_derived_write_scope
cargo test -p permagent --lib run_scoped_ready_goals
cargo test -p permagent --lib drain_sweep_
cargo test -p permagent --lib scope_conflict_refusal_reads_as_a_wait
cargo test -p permagent --lib materialize_roadmap
cargo test -p permagent --lib write_scope
cargo test -p permagent-cli --lib recipes::search_recipe
# R62c drain
cargo test -p permagent --lib next_dispatch_actions_respects_liveness_and_the_per_card_budget
cargo test -p permagent --lib next_drain_dispatches_grants_the_continuation_and_first_dispatch_outside_the_budget
cargo test -p permagent --lib headless_unblock_continuation_pending_and_never_dispatched_read_card_metadata
cargo test -p permagent --lib drain_sweep_dispatches_a_headless_unblock_continuation_after_budget
cargo test -p permagent --lib drain_gives_a_never_dispatched_goal_its_first_dispatch_before_quiescing
cargo test -p permagent --lib drain_adopts_a_review_goal_no_completion_handler_surfaced
cargo test -p permagent --lib drain_                 # every drain regression
cargo test -p permagent --lib next_drain_action
cargo test -p permagent --lib handle_goal_completion # mint_approve_review_decision extraction
cargo test -p permagent --lib headless_auto_land
cargo test -p permagent --lib headless_unblock
# shared oracle
cargo test -p permagent --lib ground_roadmap_goals   # includes the protected invariant + 3 new tests
# R62d
cargo test -p permagent --lib create_goal_worktree
cargo test -p permagent --lib is_generated_artefact_path_matches_the_exclude_set_only
cargo test -p permagent --lib r28a_checkpoint_tests
cargo test -p permagent --lib hold_verified_goal_until_work_is_committed
cargo test -p permagent --lib maybe_hold_review
cargo test -p permagent --lib hold_allows_a_goal_satisfied_at_baseline
cargo test -p permagent --lib goal_already_satisfied_at_baseline_requires_every_fact
cargo test -p permagent --lib approving_an_already_satisfied_goal_completes_it_without_a_landing
cargo test -p permagent --lib land_approved_goal
cargo test -p permagent --lib hold_exhaustion_park
cargo test -p permagent --lib salvage
cargo test -p permagent --lib cost_router::hold_done
```

## Risks
- **Not compiled.** These are the spots most likely to trip `cargo`/clippy:
  - `redispatch_ready_goals` now takes 8 arguments; `#[allow(clippy::too_many_arguments)]` is added.
  - Nested async fns in tests.
  - `json!` fixtures.
  - `rustfmt` may reflow the long test calls.
- **The shared exclude file is also the main checkout's.** An exclude rule only hides untracked artefacts; it never touches tracked files. It does, however, silently add lines to the user's `.git/info/exclude`. The alternative, `extensions.worktreeConfig`, would change the repository format for every tool that opens the repo.
- **Orphaned-Review adoption only runs with auto-land on.** A goal the coordinator moved to Review by hand in a run with auto-land off is still left for a human. The source of the problem, a manual `goal_advance review` while the worker is live, is not refused; that belongs in a separate lane.
- **Chaining serializes goals that share a test oracle.** Build-only shared checks are not chained. The earlier goals' `py_compile` / `cargo check` probes are weaker oracles; the chain's last goal carries the real suite. When the model re-authors `goals_json` at `create_roadmap` time, it has to carry the new `depends_on` along.
- **FirstDispatch can start one extra worker per never-dispatched card per drain.** It stays behind the headroom and completion gates.
- **Already-satisfied goals depend on bug (A).** The verify-evidence row-order bug is in lane R61a/R62ab. Until it is fixed, a genuine PASS stored out of order still reads as "no verify", and the rule will not fire. A registered program goal that completes as already satisfied has no landing outcome, so `ensure_registered_goal_landed` still blocks its successors; that path is retryable.
