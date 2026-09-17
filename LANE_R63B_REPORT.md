# Lane R63b: parallel sibling goals can land after the first one lands

Base: M47 `e03e6d46`. Branch: `lane/R63b`. Edit-only: no cargo was run. `rustfmt --edition 2021 --check` passes on all three files.

## Defect (M47 record r63 s2, `harness-cli-argparse-log-filter`)
The planner made 3 goals. R56a's derived disjoint `write_scope` (`logfilter/cli.py`, `logfilter/filtering.py`, `logfilter/datespec.py`) let all three run in parallel from `d930d13`. All three reached review with a `pass` verdict. Goal 2 fast-forwarded `main` to `e1cbbca`. Goals 3 (`9fe6cce`) and 4 (`823f9c9` plus the checkpoint `7ab4242`) were refused with "needs merge … cannot fast-forward". The oracle then failed on 4 checks those goals would have fixed. Receipts: `goals=3 review_reached=3`. Both worktrees were clean, and the trunk changes did not overlap either goal's scope or changed files.

## Root cause
- `crates/goose/src/goal_landing.rs:401-406` (base): `land` only fast-forwards. It returns `NeedsMerge` whenever trunk HEAD is not an ancestor of the goal head.
- `crates/goose/src/decisions_effects.rs:878` (base): `land_approved_goal` accepted that outcome with no retry. That was fine while siblings ran one at a time. It stopped working once R56a let them run in parallel.

## Change
1. `orchestrator.rs:21002` adds `rebase_goal_branch_for_headless_land`, with the helper `paths_changed_since` at :20950 and the `HeadlessRebase` enum. It runs only when the caller passes the headless flag. It refuses, with a stable reason key and the branch left untouched, unless all of these hold:
   - The worktree exists, is not the trunk checkout, and is on `goal/<run>`.
   - The worktree is clean. Regenerated artefacts such as `__pycache__` are ignored, the same filter R62d uses.
   - The reviewed `head_commit` is still on the branch. Checkpoint commits after it are allowed.
   - The goal has a non-empty declared or derived `write_scope`.
   - `git diff --name-only -z <fork> <trunk>` overlaps neither the scope nor the goal's own `<fork>..HEAD` files (`write_scope::scopes_overlap`). The second check is stricter than the brief asked. It means a clean rebase only ever combines separate files and never merges text inside one file, so goals that write outside their scope are also caught.
   - Then `git rebase --no-autostash <trunk>` runs. If it fails, `git rebase --abort` runs, and the reason is `rebase_conflict`.
   
   All git calls go through `git_in_worktree` / `git_bool_in_worktree`, which have a 20 s timeout. After a clean rebase:
   - `run_fresh_verify_for_hold` re-runs the worker's recorded genuine command (R62e) on the rebased head. A vacuous pass is rejected.
   - Card metadata is re-pointed in one `persist_card_meta` write: top-level and `dispatch_evidence.baseline_commit` become the trunk head (same as R22h), plus `dispatch_evidence.head_commit`, `commits` (`%h %s` of `onto..HEAD`, never blanked), `work_base_commit` if present, and a `rebased_onto_trunk` receipt (`from_head`, `onto`, `head`, `trunk_changed_files`, `reverify`).
   - If the re-verify fails, a `fail` verdict (`source: headless_auto_rebase_reverify`) is written, so R43a's recorded-pass-at-head check can never credit the old pass to the rewritten head.
2. `decisions_effects.rs:489`: `land_approved_goal(.., headless_auto_land: bool)`. At :885, after `land` returns `NeedsMerge`, headless auto-land loops at most `HEADLESS_REBASE_MAX_ATTEMPTS = 3` times (:994), because a sibling can land while this one re-verifies:
   - Verified: set `req.head_commit` to the rebased head and call the same `goal_landing::land` again. The existing `persist_landing_outcome`, needs-merge decision, post-landing hook, landed notes and R43a receipts all apply unchanged. The card is re-read after the helper so the landing receipt never writes stale evidence back.
   - NotRebased: keep the existing `NeedsMerge`, which still persists and creates the manual-merge unblock. `landing.refusal_reason` now carries the reason key (`rebase_trunk_overlap`, `rebase_conflict`, `rebase_no_write_scope`, `rebase_dirty_worktree`, `rebase_branch_mismatch`, `rebase_head_moved`, `rebase_not_diverged`, `rebase_no_worktree`, `rebase_git_error`, `rebase_retries_exhausted`).
   - ReverifyFailed: `Skipped(note)` with `refusal_reason=rebase_reverify_failed`, then `park_refused_goal`, the same rail R43a uses. The goal becomes Failed with an unblock, and the rebased branch stays intact.
3. The flag is the `auto_land_allowed` that `apply_decision_effect_with_auto_land` already receives. Only `auto_land_headless_goal` / credited siblings pass `true`, and that path is gated by `PERMAGENT_HEADLESS_AUTOLAND`. `apply_decision_effect`, used for interactive approvals, still passes `false`, so behaviour there is byte-identical. `ensure_registered_goal_landed` (the outbox retry for a Complete registered goal) now passes the flag through too. No env var or OnceLock is read in tests: they call `land_approved_goal` with `true`/`false` directly.
4. `goal_landing.rs` gets a doc note only: `land` stays fast-forward only.

## Consumers checked
- **Salvage sweep:** `maybe_salvage_committed_work` → `land_salvage_approved_by_reviewer` → `auto_land_headless_goal(true)` gets the rebase. Its post-land check reads `landing.outcome` / `refusal_reason`, which are persisted from the fresh card. A later salvage uses `baseline_commit` = trunk head, so `baseline..HEAD` is exactly the goal's commits.
- **Drain adoption (R62cd):** orphaned Review → `auto_land_headless_goal(true)`. Same path, consistent.
- **Already-satisfied completion (R62d):** returns before landing (nothing unlanded), so it never reaches a rebase. Its receipt `head` binding is untouched.
- **R43a trunk-relative override / R43a-b recorded pass at head:** head is re-pointed and the verdict is set to `fail` if the re-verify fails, so a stale pass is never credited.
- **H1 `verify_worktree_landing_target`:** the new baseline (trunk head) is an ancestor of the root HEAD, so it still passes.
- **`goal_landed_head_commit` / `landed_goals_for_session` / `needs_merge_goals_for_session`:** `commits` stays non-empty and `landing.landed_head` is the rebased head. The CLI "landed"/"needs merge" lines are unchanged in format.
- **L2 verification (`goose-server/src/verification`):** runs at Review, before approval. It anchors on `work_base_commit`, which is re-pointed when present.
- **Credited-sibling / promoted-dependent shapes (no `head_commit`):** return before `land`, so they are unaffected.

## Gate filters
```
cargo test -p permagent --lib decisions_effects::tests::headless_landing_
cargo test -p permagent --lib decisions_effects::tests::interactive_landing_still_reports_needs_merge_without_rebasing
cargo test -p permagent --lib decisions_effects::tests::land_
cargo test -p permagent --lib decisions_effects::tests::approv
cargo test -p permagent --lib decisions_effects::tests::ensure_registered
cargo test -p permagent --lib goal_landing::
cargo test -p permagent --lib orchestrator::tests::run_fresh_verify_for_hold
cargo test -p permagent --lib orchestrator::tests::auto_land
cargo test -p permagent --lib orchestrator::tests::try_credit_sibling_goals
```
New tests, all in `decisions_effects::tests` with real temp repos and linked worktrees:
- `headless_landing_rebases_a_disjoint_sibling_and_lands_it_after_reverify`: first sibling fast-forwards. The second rebases, re-verifies with `test -f a.txt`, which only passes on the rebased head, and lands. History stays linear (`main^` = goal a). Metadata is re-pointed.
- `headless_landing_does_not_rebase_when_the_trunk_touched_the_goal_scope`: scope `docs`, trunk added `docs/a.md`. The branch is untouched, the result is `needs_merge` with `rebase_trunk_overlap`, and the unblock exists.
- `headless_landing_aborts_a_rebase_that_cannot_apply_and_leaves_the_branch_intact`: the trunk tracks `stray.pyc`, which is untracked in the worktree. The rebase fails and is aborted, HEAD and branch are intact, and the reason is `rebase_conflict`.
- `headless_landing_parks_when_the_rebased_head_fails_reverify`: trunk does not move, the goal is `failed`, the result is `skipped` with `rebase_reverify_failed`, verdict `fail`, and the rebased branch is kept.
- `interactive_landing_still_reports_needs_merge_without_rebasing`: flag `false` gives "cannot fast-forward", no rebase and a null reason.

The two tests that spawn verify are `#[serial_test::serial(receipts_global_registry)]`. The fixture pins `core.excludesFile=/dev/null` and `commit.gpgsign=false`, so a machine's global git config cannot change what is tracked or ignored.

## Risks
- A true content conflict cannot happen under the path-disjoint rule, so the abort test uses git's untracked-file overwrite refusal. A rebase stopped mid-way with state in progress is covered by the same `--abort` call, but no test drives it.
- The re-verify is bounded by `min(90 s, remaining wall budget)`. With no wall budget left it counts as a failed re-verify, so the goal is parked (Failed plus unblock) instead of staying "needs merge" (Complete plus unblock). Nothing lands in either case.
- The rebase rewrites the goal worktree in place. If an L2 verification or reviewer process were still reading that worktree at landing time, it would see files change. Today L2 runs before approval.
- Each landing does at most 3 rebase and re-verify rounds, so a fast stream of sibling landings can still end in `rebase_retries_exhausted` (NeedsMerge).
- Rebasing lands the worktree HEAD, including any R28a checkpoint commits after the reviewed head (r63 goal 4's `pytest.ini`). They are covered by the fresh re-verify, the same thing R43a's override already does.
- Goals without a derived or declared `write_scope` never rebase. That is intentional.
- The regression was not re-run end to end: no cargo, no harness run. The coordinator needs to gate it.
