# Lane T4 — the 13 `cargo test -p permagent --lib` failures at f9ecdb5a

**Status: BLOCKED on the disk gate, with a short window used.** The lane rule is
"before every cargo invocation check `df -g /` and STOP if free space is under
9 GB". Free space on `/` was 8.42 GB at the start of this lane, rose to 10.27 GB
when the paid measurement round released space, and fell back to ~7.7 GB after
two builds. Three of the thirteen are fixed; the rest are diagnosed to a named
function and line with the fix specified, but could not be built or gated.

Two cargo runs happened inside the 10.27 GB window and diagnosed and then
verified failure #1. Everything else below is from source reading, `git log -L` /
`git show` archaeology and the failure log at
`/private/tmp/claude-501/.../scratchpad/libtest-f9ecdb5a.log`.

Two further fixes were landed without a build, because they are comment-only
edits against text-scanning guards and carry no compile or behaviour risk. The
other ten need a build to verify and are specified, not applied.

## Classification

| # | Test | Verdict | Cause |
|---|------|---------|-------|
| 1 | `orchestrator::create_roadmap_rewrites_a_harness_derived_ac_on_the_first_call` | DEBT (Lane R52a/R21c) — **not** a regression from the last six commits | The R52a rewrite fires, then the harness re-rejects its own output. **FIXED — `6fedb843`, verified green** |
| 2 | `orchestrator::goal_status_reports_a_registered_goal_worker_as_alive_and_leaves_it_alone` | DEBT (Lane Y4b) — test pollution | Session ids are `YYYYMMDD_N`, sequential per DB, not unique |
| 3 | `orchestrator::send_message_refuses_a_live_goal_worker_and_says_what_can_be_done_instead` | DEBT (Lane Y4b) — same cause as 2 | " |
| 4 | `orchestrator::view_session_and_goal_status_agree_that_a_live_goal_worker_is_alive` | DEBT (Lane Y8c) — same cause as 2 | " |
| 5 | `orchestrator::repeated_premature_done_parks_after_max_holds` | DEBT (Lane T1) — process-global `OnceLock` poison | `HEADLESS_DISPATCH_POLICY_CONFIG` set by another test in the same binary |
| 6 | `orchestrator::scoped_child_session_extensions_fn_is_unchanged_without_an_active_coding_roadmap` | ENVIRONMENTAL | Reads this machine's real `~/.config` through `Config::global()` |
| 7 | `orchestrator::scoped_child_session_extensions_fn_narrows_to_the_lean_coding_profile_for_the_top_level_coordinator` | ENVIRONMENTAL — same cause as 6 | " |
| 8 | `agents::self_knowledge::tool_descriptions_name_every_callable_tool` | DEBT (Lane R48d) | `routing_roster` shipped without being named in the registry description |
| 9 | `config::identity_name_guard::shipped_source_never_hardcodes_agent_or_developer_name` | DEBT (Lanes V2/R18c/R52–R53) | **FIXED — `a9b915a8`** |
| 10 | `dependency_claim_guard::no_shipped_comment_claims_a_reachable_dependency_is_missing` | DEBT (guard false positive) | **FIXED — `7de3ab8a`** |
| 11 | `providers::retry::a_fake_429_is_announced_while_we_wait_and_cleared_after` | ENVIRONMENTAL (suspected) | Process-global `wait_status` slot + a real 400 ms sleep, not serialized |
| 12 | `storage_health::classify::apple_system_caches_are_left_alone` | ENVIRONMENTAL | Asserts on real host paths |
| 13 | `storage_health::classify::toolchain_and_package_caches_are_regenerable_not_safe` | ENVIRONMENTAL — same cause as 12 | " |

**No failure traces to any of the last six commits** (`dfdd8111`, `970cff92`,
`85301c84`, `4aefd588`, `e7af2c74`, `f9ecdb5a`). `git log -S` shows every failing
test was introduced by an earlier lane, and the specific code paths involved were
compared against their introducing commits and found byte-identical (details
below). The suspicion in the lane brief that `f9ecdb5a`'s `goals` metadata key or
`e7af2c74`'s goal coalescing broke #1 does not hold up: `canonical_pending_goals`
returns `None` for this fixture (`install_answered_roadmap_proof` writes no
`goals` key) and `merge_overlapping_file_goals` is guarded by `goals.len() > 1`
against a one-goal fixture.

## Commits landed

### `a9b915a8` — Cite the rulings by date instead of naming the developer (#9)

All 21 hits were comments. Each said "Jesse's ruling" / "Jesse ruled" / "a design
decision for Jesse" / "Jesse's own dev machine"; each is reworded to the shape the
guard's own failure message asks for ("the user's ruling", "the user ruled", "for
the user"). The quoted HARNESS_LOOP_LOG.md heading `"JESSE RULING (2026-09-08
23:55)"` is a document title and is matched case-sensitively by the guard, so it
is left verbatim and the ruling stays lookup-able by date.

Files: `permagent-eval/src/{runner,selection,qualification}.rs`,
`goose-cli/src/session/mod.rs`, `goose/src/{decisions,goal_transition,
decisions_effects}.rs`, `goose/src/cost_router/capability_directory.rs`,
`goose/src/agents/platform_extensions/{summon,orchestrator}.rs`.

Verified without cargo by re-running the guard's own scan predicate
(whole-word, case-sensitive, `[A-Za-z0-9_]` boundaries) over `crates/*/src` and
`ui/command-center/src`: 0 remaining hits outside the allowlisted guard file.

### `7de3ab8a` — Stop a review-gate test comment reading as a dependency claim (#10)

`cost_router/review_gate.rs` tripped the guard's `when/once <X> lands` pattern
via "…when a fallback **lands** on the author's pair…", with the required
`DEPENDENCY_MARKERS` mention supplied by `pin` as a **substring of "loo(pin)g"**
in the preceding sentence. It is a false positive: the comment describes this
crate's own independence accounting, not a dependency's availability. Guard rule
(a) therefore applies — fix the comment, do not add a `Claim` (a `Claim` would
record an unavailability that does not exist and become a dead entry the guard
fails on later). Reworded to "for a fallback that resolves to the author's pair";
the word `lands` no longer appears anywhere in the file.

The substring-marker looseness (`pin` inside an ordinary English word) is left
alone: widening `DEPENDENCY_MARKERS` to word boundaries is a separate change with
its own blast radius across the CLAIMS allowlist.

## Root cause and fix for the rest

### 1 — `create_roadmap_rewrites_a_harness_derived_ac_on_the_first_call` — FIXED in `6fedb843`

Run alone at `f9ecdb5a` it fails identically, so it is a deterministic defect,
not pollution. The fix below is verified: the test is green after `6fedb843`.

The fixture's project has
`root_path = NULL` (the personal project is seeded that way in
`spectral_schema.rs`), so `facts_dir` falls back to the session's tempdir
working dir and `RepoFacts::gather` detects **no** check commands — which the
rejection text itself confirms ("none detected in this repo").

`validate_roadmap_ac_or_rewrite` then does exactly what R52a intends: the
criterion's hash is in the recorded `derived_ac_hashes`, so
`rewrite_selected_ungrounded_criteria` rewrites it. With no detected check, the
rewrite takes the middle branch and produces a structured probe marker:

```
probe:command cmd="node tests/run.ts" — running `node tests/run.ts` produces …
```

The function then re-checks with `ungrounded_named_criteria_in_goals`. And
`criterion_names_a_check_or_file` exempts `UNVERIFIED (…)` markers **but not
`probe:` markers** — so the harness flags its own freshly-compiled probe as an
ungrounded model-authored criterion and returns the R21b rejection. That is
precisely the reject-then-identical-resubmit round trip R52a exists to prevent,
firing on R52a's own output.

Archaeology: `validate_roadmap_ac_or_rewrite`,
`rewrite_selected_ungrounded_criteria`, `criterion_names_a_check_or_file`,
`is_grounded_criterion`, `classify_probe_shape`, `extract_quoted_command`,
`PROBE_COMMAND_PROGRAMS`, the `facts_dir` resolution and the test body itself are
all byte-identical to `86facc0c`, the commit that introduced the test — so it
looks as though it was never green in this configuration and Lane R52a's gate did
not cover it.

**Fix applied:** exempt the three `PROBE_*_PREFIX` markers in
`criterion_names_a_check_or_file`, exactly as `UNVERIFIED_CRITERION_PREFIX`
already is. A criterion the harness has already compiled into a structured probe
is the most grounded form available — `verify` can execute it — so re-flagging it
is wrong independent of this test, and the rewrite becomes idempotent. No test
pins the old classification for a probe-prefixed criterion (the
`rewrite_ungrounded_named_criteria_emits_probe_*` tests assert on the *output* of
a rewrite over plain criteria).

### 2, 3, 4 — the GOAL_WORKERS registry liveness tests

Not a regression in `970cff92`/`dfdd8111`. `SessionManager`'s storage layer mints
session ids as `YYYYMMDD_N` with `N` from `MAX(...)+1` **within that database**
(`session_manager.rs:~2100`). Every test with a fresh tempdir DB therefore
produces the *same* id, typically `20260916_1`. `GOAL_WORKERS` is process-global
and keyed by card id, and `live_goal_worker_card_for_session` scans for the first
entry whose `session_id` matches — so a concurrent test that registered the same
synthetic id under its own card wins the scan. The failure shows exactly this:
two card-id UUIDv7s minted in the same millisecond.

All four tests that call `register_goal_worker` directly are already
`#[serial_test::serial(goal_workers)]`. The uncovered source is the **production**
registration at `orchestrator.rs:7317` inside `dispatch_goal_fn`: any dispatch
test can register `20260916_1` under its own card and is not in that serial
group.

**Fix (smallest hermetic one):** make the fixture's worker session id unique
rather than trying to enumerate every dispatch test. In `dead_silent_worker_goal`
and the Y8c fixture, after `create_session`, rewrite the row's id to
`format!("{}_{}", id, uuid::Uuid::new_v4().simple())` and stamp that on the card,
so the card stamp, the registry entry and the session row all agree on an id no
other test can mint. Broadening the serial key instead would serialize a large
slice of the suite for no benefit.

### 5 — `repeated_premature_done_parks_after_max_holds`

`needs_human_attention` read `Null` because the card was never parked at all: in
`maybe_hold_review`'s `HoldOutcome::Park` arm, `maybe_headless_unblock_verify_gap`
is tried first and returns early on success, resuming the goal instead.

This is a *recurrence of the documented M24 failure*. The comment at
`orchestrator.rs:28219-28245` describes it exactly — a test setting
`HEADLESS_DISPATCH_POLICY_CONFIG`, a process-global `OnceLock` that can only ever
be set once per test binary, makes `headless` read `true` for every other test in
the process, and this test "assumes headless is OFF". Lane R20b-e removed the
offending test. **A different test still does it:**
`dispatch_goal_to_and_resume_roadmap_consult_the_installed_headless_policy`
(`orchestrator.rs:~31906`) calls the real
`install_dispatch_hook_for_headless_session`, which sets that `OnceLock` at
`orchestrator.rs:12666`. It carefully restores the *dispatch hook* via
`take_goal_dispatch_hook_for_test()` — but a `OnceLock` cannot be restored, so
the headless policy leaks for the rest of the run.

**Fix:** the lane rule ("tests must never set process-global `OnceLock`s") says
this test may not call the installer. Give it the same treatment the safe R18g
tests already use — inject the policy explicitly instead of installing it — or,
if exercising the real installer is the point, move the assertion behind a
`headless` parameter the way `maybe_headless_unblock_verify_gap`'s own tests do.

### 6, 7 — `scoped_child_session_extensions_fn_*`

`parent_extensions_fn` calls
`EnabledExtensionsState::extensions_or_default(extension_data, Config::global())`,
and that function performs an **additive refresh**: it merges every extension
enabled in the config into the session's stored set
(`session/extension_data.rs:~145`). So the fixture's 9 stashed extensions become
9 ∪ (this machine's enabled extensions) = 31, and the host's `Extension Manager`
entry (keyed by display name) survives narrowing alongside `extensionmanager` —
exactly what the failure prints.

The tests' `lock_env([("PERMAGENT_PATH_ROOT", tmp)])` only isolates them if
`Config::global()` has not already been initialized. It is a process-global
`OnceLock`, so in a full-suite run whichever test touches it first wins, and the
real `~/.config` leaks in. These pass alone and fail in-suite.

**Fix:** add a `&Config` seam rather than fighting the global. Introduce
`parent_extensions_with_config(context, config)` and
`scoped_child_session_extensions_with_config(context, persona, config)`; the
existing free functions delegate with `Config::global()` so production is
unchanged, and the two tests pass a `Config::new(tmp.join("config.yaml"), …)`.
`Config::new` already exists (`config/base.rs:438`).

### 8 — `tool_descriptions_name_every_callable_tool`

`routing_roster` is registered as a callable orchestrator tool
(`orchestrator.rs:11704`, dispatched at `:11955`) but the extension's registry
`description` (`platform_extensions/mod.rs:561`) never names it, so it renders
into no line of the `permagent_self` brief. Straight Lane R48d debt.

**Fix:** name it in the description — the natural spot is the workers clause:
`inspect available workers and the routing roster each tier resolves to
(list_workers, check_worker, routing_roster)`.

**Why it was not landed blind:** that description is copied verbatim into four
insta snapshots
(`agents/snapshots/permagent__agents__prompt_manager__tests__{basic,typical_setup,
one_extension,all_platform_extensions}.snap`) and its byte length feeds
`…__family_prompt_size_table.snap`, which records `shared body` bytes, per-family
`total_bytes` and `approx_total_tokens = ceil(bytes/4)`. Hand-editing five
snapshots without being able to run `cargo insta` would risk trading one red test
for several. This is a two-minute fix the moment a build can run.

### 11 — `a_fake_429_is_announced_while_we_wait_and_cleared_after`

Not reproduced (no cargo). The test polls `wait_status::current()` for up to 2 s
against a 400 ms real sleep, so a plain scheduling delay is an unlikely miss. The
likelier cause is that `wait_status` is a process-global slot the test opens with
`wait_status::clear()` and no serial key — a concurrent provider test that clears
or overwrites it would make the announcement invisible. **Needs the three
solo runs the brief asks for before choosing between "serialize it" and "it is a
real regression".**

### 12, 13 — `storage_health::classify` tests

`classify()` step 1 calls `inuse::in_use_reason(path, …)`, which stats the real
path. The test helper `classify_path` (`classify.rs:331`) passes literal host
paths, so on this machine `/Users/j/.npm` and
`/Users/j/Library/Caches/GeoServices` are genuinely recently written and classify
as `in_use` before the macOS/costly-cache rules are ever reached. Purely
environmental; these tests are green on a machine that has not touched npm today.

**Fix (small and hermetic):** prefix the path inside `classify_path` with a root
that cannot exist, e.g. `/permagent-classify-fixture` + the literal path. Every
rule these tests exercise is path-shape-based and survives the prefix —
`is_macos_managed` matches the `/Library/Caches/` substring plus the file name,
and `costly_cache_label` matches runs of consecutive components — while
`in_use_reason` can never find anything to stat. One line in the test helper,
and it makes every other `classify_path` test hermetic too.

## Second window (coordinator pruned to 18 GB) — what happened

The prune freed the target dir, so the next `cargo test` was a **cold 9-minute
rebuild that put 9 GB straight back**. Free space went 17 GB → 8.5 GB on that one
build and has stayed there. Only one cargo command fitted in the window:

- **#11 `a_fake_429_is_announced_while_we_wait_and_cleared_after` passes alone.**
  So it is a suite-only failure, consistent with the `wait_status` diagnosis
  below. Not fixed — the remaining two solo runs and the serialization fix both
  need a build.

`wait_status` has exactly two writers outside the test
(`providers/inflight.rs:85,97` publish/clear for `ConcurrencyLimit`, and
`retry.rs:188,197` for the retry path). The test calls `wait_status::clear()` with
no serial key, so any concurrent provider test that publishes or clears can make
the 400 ms announcement invisible to its poll loop. **Fix:** put this test and
`inflight`'s own tests in a shared `#[serial_test::serial(provider_wait_status)]`
group.

## Uncommitted work in the tree

Fixes for **#2–#8 and #12/#13 are written and sitting in the working tree,
deliberately uncommitted**, because the disk floor closed before any of them
could be built. Nothing unverified goes into this branch's history — that is the
failure mode that produced these thirteen in the first place. `git status` is
dirty by design; the changes are:

`crates/goose/src/agents/platform_extensions/orchestrator.rs`

1. **#6/#7 — a `&Config` seam.** `parent_extensions_with_config` and
   `scoped_child_session_extensions_with_config` added; the existing free
   functions delegate to them with `Config::global()`, so production is
   unchanged. The two tests call the `_with_config` form with
   `r27b_empty_config(tmp.path())` — a `Config::new_with_file_secrets` over two
   paths that do not exist, so no keyring is touched and the additive refresh
   contributes nothing. Confirmed by reading `get_extensions_map_with_config`:
   it reads the config file and nothing else, with **no built-in defaults**, so
   an empty config really does yield an empty merge.
2. **#2–#4 — a unique worker session id.** `dead_silent_worker_goal` now renames
   the freshly created session row to `<id>_<uuid>` and stamps that on the card,
   so no other fixture in the binary can mint the same id. The row has no
   children, and the `YYYYMMDD_` prefix is preserved so the id sequencer's
   `SUBSTR(id, 10)` still parses.
3. **#5 — spend R18g's budget up front.** `repeated_premature_done_parks_after_
   max_holds` seeds `headless_unblock_count: 1` on its card, so the headless
   unblock refuses on `once_per_goal_spent` whichever way the
   `HEADLESS_DISPATCH_POLICY_CONFIG` `OnceLock` landed. Inert when headless is
   off, and the same technique the R20b-e tests beside it already use. Chosen
   over making the offending test stop calling the real installer, per the
   coordinator's "prefer per-test isolation over process-global resets" — a
   `OnceLock` cannot be reset at all, so isolating the victim is the only
   test-only move available.

`crates/goose/src/agents/platform_extensions/mod.rs`

4. **#8 — name `routing_roster`** in the orchestrator's registry description:
   "inspect available workers and the routing roster each worker tier resolves to
   (list_workers, check_worker, routing_roster)". **The four prompt snapshots and
   `family_prompt_size_table.snap` are now stale** and must be regenerated with
   `INSTA_UPDATE=always` before this can be committed; the only legitimate diff
   is that sentence plus the byte/token counts.

`crates/goose/src/storage_health/classify.rs`

5. **#12/#13 — hermetic paths.** The `classify_path` test helper prefixes every
   case with `/permagent-classify-fixture-does-not-exist`. All the rules under
   test read path shape only (`is_macos_managed` wants the `/Library/Caches/`
   substring plus the file name; `costly_cache_label` matches component runs;
   `local_rebuild_consequence` reads the file and parent names), so they are
   unaffected, while `inuse::in_use_reason` can no longer stat anything. This
   also de-mines four sibling tests that assert on real paths like
   `/Users/j/Documents/dev/spectral/target`.

**None of the five has been compiled.** Review them as a patch, not as a result.

## What is still red

The whole suite was never re-run, so there is no `test result:` line for this
lane. Per-item:

- **#1 — green**, verified by `cargo test -p permagent --lib
  create_roadmap_rewrites_a_harness_derived_ac_on_the_first_call` after
  `6fedb843`.
- **#9, #10 — expected green**, landed without a build. Both tests are pure text
  scanners over shipped source, and each fix was checked by re-running that
  guard's own scan predicate by hand; the edits are comments only and cannot
  affect compilation or behaviour. Still unproven by execution.
- **#11 — passes alone**, so it is a suite-only interaction; the serialization
  fix is specified but not written.
- **#2–#8, #12, #13 — still red in the suite, fixes written but uncommitted and
  uncompiled** (see "Uncommitted work in the tree").

Nothing on this branch was left in a worse state than it was found: the three
landed commits each remove a failure and none of them touches production
behaviour beyond #1's one-line classification arm.

## What the next window needs

The target dir is **warm now** (48 GB), so the 9-minute cold rebuild will not
repeat — the remaining work is a handful of fast incremental builds. In order:

1. `cargo test -p permagent --lib scoped_child_session_extensions -j2`
2. `cargo test -p permagent --lib goal_worker -j2` and the three named registry
   tests
3. `cargo test -p permagent --lib repeated_premature_done_parks_after_max_holds -j2`
4. `cargo test -p permagent --lib storage_health -j2`
5. `INSTA_UPDATE=always cargo test -p permagent --lib prompt_manager -j2`, then
   read the five `.snap` diffs — only the description sentence and the size-table
   numbers may move — then `tool_descriptions_name_every_callable_tool`
6. add the `provider_wait_status` serial group for #11
7. one full `cargo test -p permagent --lib -j2`

Each of those is a separate cause and wants its own commit. The blocker is
purely free space on `/`: it sits at ~8 GB and the floor is 10 GB. Freeing space
anywhere other than the target dir (which must stay warm) is what unblocks this.

## Follow-on note

Nine of the thirteen are process-global state (`Config::global()`,
`HEADLESS_DISPATCH_POLICY_CONFIG`, `GOAL_WORKERS`, `wait_status`) or host state
(`~/.config`, `~/.npm`, `~/Library/Caches`) leaking into tests that pass in
isolation. That is the same class of defect the lane rule about `OnceLock`s was
written for, and it is why named-filter lane gates kept reporting green. A
whole-suite gate is the only thing that catches it.
