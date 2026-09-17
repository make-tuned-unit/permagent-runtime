# Lane R56a — `select-failing-test-decompose-before-create` intermittently never reaches Review

Branch `lane/R56a` (from `f9ecdb5a`), worktree `/Users/j/Documents/dev/permagent-worktrees/laneR56a`.
Commit: `754c754a` "Prove disjoint roadmap roots so the second one can start".

Edit-only lane: **no cargo was run.** Gate filters for the coordinator are at the bottom.

---

## What the failing run actually did

Evidence, in order, from
`…/laneM17/dschat-r58/q3-runs/dschat-select-failing-test-decompose-before-create-1789587078715525000-12/`:

1. `logs/harness.log:106` — `decompose_roadmap` produced **two** roots (pricing.py, inventory.py),
   neither with a `write_scope`. Both cards carry the SAME acceptance criterion
   `python3 test_checkout.py exits with code 0`.
2. `logs/harness.log:178` — `goal_advance dispatch` on root 2 refused:
   `write scope conflicts with active goal 'Fix discount calculation in pricing.py': write scope is
   missing or empty for a non-read-only node`.
3. `logs/harness.log:190` — the coordinator `escalate`d the refusal (`escalated=1`) instead of waiting.
4. `data/spectral/permagent.db` card `01a0abb3-ff17…` metadata — root 1 ran **three** worker sessions
   (`20260916_2/_3/_4`), committed `0b7a9b2 "Fix apply_discount…"`, and each attempt ended in
   `hold_done` → `last_error: "Held twice (held twice without verify …)"`, `headless_unblock_count: 1`.
5. Same card, `salvage_receipt` — the run-end salvage sweep's verify **failed**:
   `assert decrement_stock(10, 3) == 7 … AssertionError` in root 1's own worktree.
6. `logs/harness.log:258-267` — `0 goal(s) finished, 1 still running at exit → parked`,
   `park_reason … ready_unconsumed(commit=0b7a9b2)` (root 1) and `park_reason … never_dispatched`
   (root 2), `run-end salvage sweep: 1 goal(s) had unlanded work, 0 landed`.

Correction to the brief: root 1's worker did **not** run clean and the drain did **not** quiesce out
from under a live worker. It burned all three attempts, the drain parked it on the re-dispatch budget,
and salvage then failed its verify. The q2 passing run (`…-1789584939290348000-12`, log:133-140) is a
**one-goal** roadmap: `goals=1 review_reached=1`. The intermittency is planner shape, not timing.

---

## Root cause 1 — nothing in the tree ever stamps a `write_scope`

`crates/goose/src/agents/platform_extensions/write_scope.rs:103-117` — `metadata_conflict` returns
"write scope is missing or empty for a non-read-only node" whenever either side has no declared scope.
That is the correct default (a worktree isolates branches, not merge collisions).

But the key was **never written by anything**. `materialize_roadmap_transaction_with_budget`
(`orchestrator.rs:3577`+) builds each card's metadata from `goal_state::ProposedGoal`
(`crates/goose/src/goal_state.rs:311`), which has only `title/description/acceptance_criteria/tags/
depends_on` — no metadata field a planner could fill in. `crates/goose/src/agents/platform_extensions/
orchestrator.rs:3821` (`cards::create_goal_card_in_tx`) is the only goal-card creation site in the repo.
Grep confirms the pre-fix consumers were the dispatch check alone (`orchestrator.rs:6265`, `:6465`).

Consequence: **every** multi-root roadmap serialized, regardless of file disjointness, and the second
root's dispatch was refused for as long as the first stayed `in_progress`.

Codex's `4aefd588` ("Coalesce overlapping harness goals") only edits the coordinator recipe text plus a
recipe assertion — it nudges the planner toward one card but derives nothing, so the defect survived it.

### Fix

`derive_disjoint_write_scopes` (`orchestrator.rs:1452`), pure:

- uses `goal_named_files` (`orchestrator.rs:1340`) — the same title+description probe
  `merge_overlapping_file_goals` (`:1358`) already uses. Acceptance criteria are excluded, so a shared
  test oracle (`test_checkout.py`) is never claimed as a write;
- all-or-nothing: declines for the whole proposal unless ≥2 goals, non-empty `RepoFacts.files`, every
  goal names ≥1 existing file, each set ≤ `DERIVED_WRITE_SCOPE_MAX_PATHS` (8), every path round-trips
  through the dispatcher's own `write_scope::declared_write_scope`, and the sets are pairwise
  non-overlapping under `write_scope::scopes_overlap`;
- `handle_create_roadmap` calls it on the FINAL goals against the `facts` it already gathered
  (`orchestrator.rs:10765`), and logs the count to tracing only (never into a user-visible note);
- materialization stamps it only when the card has no scope of its own (`orchestrator.rs:3813`), so an
  existing declared scope can never be widened.

---

## Root cause 2 — a root whose FIRST dispatch is refused is invisible forever after

`run_scoped_ready_goals` (`orchestrator.rs:16371`) kept only cards already awaited by this run, or
cards that `depends_on` one of them. A sibling root has **no worker session** (never dispatched, so
`descendant_session_ids` → `goal_card_for_worker_session` never maps it) and **no `depends_on`** — so
it matched neither test and the drain's sweep (`redispatch_ready_goals`, `:16413`) could not see it on
any poll. Same blind spot in `run_end_salvage_sweep` (`:17123`), which also walks descendant sessions —
hence "1 goal(s) had unlanded work" when two goals were open.

### Fix

- `run_scoped_ready_goals` also accepts a card **this run's own `create_roadmap` minted**
  (`metadata_json.created_by_session_id`, stamped at `orchestrator.rs:3754`). That is the same "same
  roadmap" identity `roadmap_sibling_goals` (`:18460`) and `cards::goal_cards_created_by_session`
  already use — not a new notion of ownership, and a stranger card or another session's card stays out
  of scope exactly as before.
- A scope-conflict refusal is treated as a **WAIT**: `WRITE_SCOPE_CONFLICT_REFUSAL` (`:6238`) is now the
  one const shared by the refusal producer (`write_scope_conflict`, `:6284`) and the matcher
  `refusal_is_scope_wait` (`:6248`); on that shape the drain refunds the re-dispatch attempt
  (`:16475`), because nothing was started and no worker/model budget was spent. At the default budget
  of 2 (`DEFAULT_HEADLESS_MAX_REDISPATCH`), refusals alone could otherwise exhaust the card's whole
  allowance before the conflict cleared. It still reports 0 dispatches, so a pass made only of these
  can quiesce; the retry is bounded by the drain cap and poll interval.
- Coding recipe (`crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`, rule 4): a write-scope
  refusal is a WAIT, never an escalation and never a re-issue of the identical dispatch. Pinned by
  `crates/goose-cli/src/recipes/search_recipe.rs` (same assertion style Codex added in `4aefd588`).

---

## Root cause 3 — NOT FIXED, and it is the one that kept `review_reached` at 0

**Both cards carry the same whole-suite oracle, which neither can satisfy in its own worktree.**

`test_checkout.py` asserts `apply_discount(...) == 90.0` AND `decrement_stock(10,3) == 7`. Each goal
runs in an isolated worktree off trunk with only its own fix, so `python3 test_checkout.py` fails for
BOTH of them — serially (what happened: hold, hold, headless unblock, hold, park, salvage verify
failed) and in parallel (what happens after fix 1: both roots commit, both fail the same oracle, both
park). Fixes 1 and 2 remove the `never_dispatched` park and the wasted escalation, but on their own
they will **not** turn this fixture green; they may turn one parked goal into two.

The q2 run passed only because the planner emitted ONE goal that fixed both files.

I did not fix this because the obvious fix collides with an explicitly protected invariant:
`ground_roadmap_goals_keeps_a_split_when_every_goal_names_a_real_file` (`orchestrator.rs:38937`) pins
that two file-disjoint goals sharing the identical criterion "`cargo test` passes" **must stay split**
("a genuinely-justified split … that right-sizing itself already proved correct to keep"). Any
"merge goals that share one oracle" rule breaks that test by construction. Options for the coordinator
to rule on, cheapest first:

1. **Planner prompt** (`orchestrator.rs:10040`+ system text): "when one check can only pass after
   several cards land, either put them in ONE card or chain them with `depends_on` and give every card
   but the last a criterion it can satisfy alone." Zero code risk, model-dependent.
2. **Chain + defer the shared oracle** in `ground_roadmap_goals`: when file-disjoint goals share an
   identical grounded criterion, add `depends_on` in order and keep the shared oracle only on the last
   card, leaving each earlier card its own R43b-derived behavioural criterion. This is the shape that
   actually reaches Review twice. It is a real semantic change (ordering + AC removal) and interacts
   with `ground_acceptance_criteria`'s empty-list fallback, which re-adds a detected check command —
   it needs its own lane, not this one.
3. Accept the merge and retire the protected invariant above — needs an explicit ruling.

---

## Receipt honesty

Nothing here writes or adjusts `verification_integrity`. No goal is marked as having reached Review
that did not; `still_running_goal_reasons_for_session` (`:17496`) and its `ready_unconsumed` /
`never_dispatched` vocabulary are untouched. With fix 2 the honest outcome simply changes from
"never_dispatched" to whatever the sibling actually does.

---

## Gate filters (the coordinator runs these; I did not)

```
cargo test -p permagent --lib derive_disjoint_write_scopes
cargo test -p permagent --lib materialized_goals_carry_the_derived_write_scope
cargo test -p permagent --lib run_scoped_ready_goals
cargo test -p permagent --lib drain_sweep_
cargo test -p permagent --lib scope_conflict_refusal_reads_as_a_wait
cargo test -p permagent --lib ground_roadmap_goals          # regression: grounding untouched
cargo test -p permagent --lib materialize_roadmap           # regression: the new param
cargo test -p permagent --lib write_scope                   # regression: the pure module
cargo test -p permagent-cli --lib recipes::search_recipe    # the recipe contract (different crate)
```

New/changed tests:

| test | line | proves |
|---|---|---|
| `derive_disjoint_write_scopes_stamps_one_real_file_per_disjoint_goal` | `orchestrator.rs:39024` | the fixture's two roots get `["pricing.py"]` / `["inventory.py"]` and `metadata_conflict` then reads them as independent |
| `derive_disjoint_write_scopes_declines_when_any_goal_is_unproven` | `:39074` | declines on an unnamed goal, an overlapping pair, a single goal, and empty facts |
| `materialized_goals_carry_the_derived_write_scope` | `:38605` | the key reaches the card metadata the dispatcher actually reads (`#[serial_test::serial(receipts_global_registry)]`, matching its neighbour) |
| `run_scoped_ready_goals_keeps_a_never_dispatched_sibling_root_of_this_run` | `:34209` | sibling root in; stranger and another run's root out |
| `run_scoped_ready_goals_keeps_awaited_cards_and_their_dependents` | `:34157` | unchanged behaviour, updated for the new argument |
| `drain_sweep_dispatches_a_never_dispatched_sibling_root_of_this_runs_roadmap` | `:34319` | durable: the sweep starts it, the stranger is untouched |
| `drain_sweep_does_not_dispatch_a_ready_card_outside_this_runs_dag` | `:34233` | unchanged (its stranger has no `created_by_session_id`) |
| `scope_conflict_refusal_reads_as_a_wait_and_other_refusals_do_not` | `:34413` | the matcher against the exact text the producer builds |

---

## Risks

- **Under-claimed scope (the real one).** A derived scope names only files the card's own text mentions.
  If a worker also edits a file the card never named and a parallel sibling touches it too, they can
  now collide where they previously serialized. Mitigations: the derivation declines unless every goal
  names a file and all sets are disjoint, and `merge_overlapping_file_goals` has already coalesced
  same-file cards. The blast radius is a merge conflict on a goal branch, not lost work.
- **Conservative by design.** A planner that names the shared test file in every description (common)
  produces overlapping sets and the derivation declines for the whole roadmap — i.e. today's behaviour.
  That is intentional; widening it (e.g. dropping files every goal names) would be exactly the
  under-claim above.
- **Sweep breadth.** The drain can now start a Ready card of this run's roadmap that it never awaited.
  Bounded by `max_redispatch`, the sweep headroom, `eligible_ready_goals`' pause/column gates, and
  ownership by this coordinator's session id. A human's card and another session's card are unaffected.
- **Refund loop.** A card refused on scope every poll retries every poll (one cheap DB query) until the
  cap. It never counts as a dispatch, so quiescence is unaffected, and a genuinely stuck conflict just
  ends as `never_dispatched` exactly as today.
- **Signature churn.** `materialize_roadmap_transaction_with_budget` gained a 12th argument and
  `run_scoped_ready_goals` a 3rd; all four call sites and both existing tests are updated in the same
  commit (grepped: no other callers in the workspace).
- **Not compiled.** Edit-only lane. `cargo fmt` may want to reflow one or two of the new test calls.
