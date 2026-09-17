# Lane T4 — the 13 `cargo test -p permagent --lib` failures at f9ecdb5a

## Result

```
cargo test -p permagent --lib -j2   (at 6e4d52b7)
test result: ok. 5416 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 142.60s
```

Baseline at `f9ecdb5a` was 5403 passed / 13 failed. 5403 + 13 = 5416, so every
failure turned green and nothing new broke. **Nothing is left red.**

Full log: `/private/tmp/claude-501/.../scratchpad/libtest-t4-final.log`.

## Headline

**None of the 13 was a regression from the last six commits** (`dfdd8111`,
`970cff92`, `85301c84`, `4aefd588`, `e7af2c74`, `f9ecdb5a`). `git log -S` puts
every failing test in an earlier lane, and each failure's code path was compared
against the commit that introduced the test. They are debt that accumulated
because lane gates ran named test filters and never the whole suite.

**Eight of the thirteen passed in isolation** and failed only in the suite —
process-global state (`OnceLock`s, a shared status slot, a registry keyed so ids
collide across test databases) or host state (real `~/.npm`). Only a whole-suite
gate catches that class.

## Classification (final)

| # | Test | Verdict | Commit |
|---|------|---------|--------|
| 1 | `orchestrator::create_roadmap_rewrites_a_harness_derived_ac_on_the_first_call` | DEBT (R52a/R21c), deterministic | `6fedb843` |
| 2 | `orchestrator::goal_status_reports_a_registered_goal_worker_as_alive_and_leaves_it_alone` | DEBT (Y4b), suite pollution | `2e636486` |
| 3 | `orchestrator::send_message_refuses_a_live_goal_worker_and_says_what_can_be_done_instead` | DEBT (Y4b), suite pollution | `2e636486` |
| 4 | `orchestrator::view_session_and_goal_status_agree_that_a_live_goal_worker_is_alive` | DEBT (Y8c), suite pollution | `2e636486` |
| 5 | `orchestrator::repeated_premature_done_parks_after_max_holds` | DEBT (T1), `OnceLock` poison | `e0f12037` |
| 6 | `orchestrator::scoped_child_session_extensions_fn_is_unchanged_without_an_active_coding_roadmap` | DEBT (R27b), deterministic | `25e940a8` |
| 7 | `orchestrator::scoped_child_session_extensions_fn_narrows_to_the_lean_coding_profile_for_the_top_level_coordinator` | DEBT (R27b), deterministic | `25e940a8` |
| 8 | `self_knowledge::tool_descriptions_name_every_callable_tool` | DEBT (R48d) | `64e22a35` |
| 9 | `config::identity_name_guard::shipped_source_never_hardcodes_agent_or_developer_name` | DEBT (V2/R18c/R52–R53) | `a9b915a8` |
| 10 | `dependency_claim_guard::no_shipped_comment_claims_a_reachable_dependency_is_missing` | DEBT, guard false positive | `7de3ab8a` |
| 11 | `providers::retry::a_fake_429_is_announced_while_we_wait_and_cleared_after` | Suite pollution (shared slot) | `406ba73e` |
| 12 | `storage_health::classify::apple_system_caches_are_left_alone` | ENVIRONMENTAL (host paths) | `6e4d52b7` |
| 13 | `storage_health::classify::toolchain_and_package_caches_are_regenerable_not_safe` | ENVIRONMENTAL (host paths) | `6e4d52b7` |

Only #1 touches production behaviour (one classification arm), and #8 changes a
user-facing registry description. Every other fix is test-only.

**Correction to this report's own earlier draft:** #6/#7 were first classified
ENVIRONMENTAL (host config leaking through `Config::global()`), and a `&Config`
seam was written on that theory. Run, it still failed against an empty tempdir
config. The real cause is deterministic and fails on every machine (see below).
The seam was reverted and never committed.

## Commits, per cause

### `a9b915a8` — #9 identity-name guard
21 comments named the developer ("Jesse's ruling", "a design decision for
Jesse"…). Reworded to "the user's ruling" etc. The quoted HARNESS_LOOP_LOG.md
heading `"JESSE RULING (2026-09-08 23:55)"` is a document title, matched
case-sensitively by the guard, and is kept verbatim so the ruling stays
lookup-able by date. Comments only.

### `7de3ab8a` — #10 dependency-claim guard
A false positive. "…**when** a fallback **lands** on the author's pair…" matched
the `when/once <X> lands` pattern, and the required dependency marker `pin` was
supplied as a substring of "loo**pin**g". Guard rule (a) applies: the comment was
reworded, and no `Claim` was added, because a `Claim` would record an
unavailability that does not exist. Comment only.

### `6fedb843` — #1 the AC validator re-flagging its own probe marker
Deterministic; it fails alone. The fixture repo has no detected check command, so
R52a's first-call rewrite compiles the criterion into
`probe:command cmd="node tests/run.ts" — <original text>`. The immediate re-check
then called it ungrounded again: `criterion_names_a_check_or_file` exempted the
`UNVERIFIED (…)` marker family but not the `probe:*` one, and the original text
kept after the marker still names `node tests/run.ts`. The harness was rejecting
its own output, the exact round trip R52a exists to prevent. The fix exempts the
three `PROBE_*_PREFIX` markers the same way.

### `2e636486` — #2–#4 session-id collision in `GOAL_WORKERS`
Session ids are `YYYYMMDD_N` with `N` numbered **per database**, so every
fresh-tempdir fixture mints `20260916_1`. `GOAL_WORKERS` is process-global and
keyed by card id, so two tests put one session id under two cards, and
`live_goal_worker_card_for_session` returned whichever entry iterated first. The
failure printed two card UUIDv7s from the same millisecond. The existing
`goal_workers` serial key cannot help, because the production registrar in
`dispatch_goal_fn` is reached by dispatch tests outside that group. The
`dead_silent_worker_goal` fixture now renames its fresh session row to
`<id>_<uuid>`. Verified with all 8 `goal_status*` tests (its other users) plus
the 2 `live_goal_worker` tests.

### `e0f12037` — #5 `HEADLESS_DISPATCH_POLICY_CONFIG` poison
`dispatch_goal_to_and_resume_roadmap_consult_the_installed_headless_policy` calls
the real headless installer, which fills a process-global `OnceLock` that nothing
can empty. Every later test reads headless as true, so R18g's headless unblock
resumes the goal instead of parking it. This is the M24 failure Lane R20b-e
diagnosed, recurring from a different test. A `OnceLock` cannot be reset from test
code, so the fix is on the victim's side: the test seeds
`headless_unblock_count: 1` (the technique the R20b-e tests already use), so R18g
refuses either way. **Reproduced both directions** with `--test-threads=1` and the
installer test run first: without the fix it fails with `left: Null, right: true`,
the exact suite failure; with the fix both tests pass.

### `25e940a8` — #6/#7 scoped child extensions
Deterministic on every machine. `EnabledExtensionsState::extensions_or_default`
additively merges config-enabled extensions onto the stored set, **and**
`config::migrations::migrate_platform_extensions` seeds every default-enabled
platform extension into *any* config on load, even an empty one. So the parent
set is always the fixture plus the whole platform roster, and the tests pinned
the fixture. The unchanged case now asserts what its own message says: its keys
equal `parent_extensions_fn`'s keys. The narrowing case now compares `key()`
(what `narrow_extensions_for_agent` matches grants on) instead of `name()`. The
platform "Extension Manager" and the fixture's builtin "extensionmanager" share
that key, and both are correctly granted.

### `64e22a35` — #8 `routing_roster` named in the description
The workers clause now reads "inspect available workers and the routing roster
each worker tier resolves to (list_workers, check_worker, routing_roster)".
Snapshots were regenerated with `INSTA_UPDATE=always` and **checked mechanically**:
- each of the four prompt snapshots changes exactly one line, and that line
  equals the old one with only this substitution applied;
- `family_prompt_size_table.snap` moves only the unscoped section, by +68 bytes on
  every row (the added text's length), with tokens = ceil(bytes/4). The scoped
  GooseCli section is unchanged, as it should be.

insta also dropped each snapshot's `assertion_line` metadata header; that header
was restored so the diff stays within scope. No other copy of the old
description exists in the repo.

### `406ba73e` — #11 `provider_wait_status` serial group
Passes alone. `wait_status` is one process-global slot. The test polls it for a
400 ms announcement, while every retry or contended-slot test publishes and then
**clears** that slot, within microseconds for a zero-delay retry. A single failure
is not enough to name the specific neighbour, so every slot mutator in the lib
binary now shares the `provider_wait_status` serial key. That is 10 tests across
`retry.rs` (the 429 test and the 4 reservation tests), `inflight.rs` (3),
`wait_status.rs` (`publish_then_clear_round_trips`) and `session_manager.rs`
(`b6_bounded_retry_storm_…`).

### `6e4d52b7` — #12/#13 hermetic classify paths
`classify()` step 1 stats the real path before any rule runs, and `~/.npm` really
was in use on this host. The `classify_path` helper now prefixes each case with a
root that cannot exist. All the rules under test read only path shape (the
`/Library/Caches/` substring, component runs, file and parent names), so they are
unaffected. This also makes four sibling tests hermetic that asserted on real
paths such as `~/Documents/dev/spectral/target`. The genuine in-use tests build
their own tempdir and still exercise the live probe.

Report-only commits: `c9d8224c`, `24f8f4bf`, `1eeea554`, plus this update.

## Disk

The first two windows were blocked by the free-space floor. After the target prune,
the next build was a cold 9-minute rebuild that used ~9 GB. The third window
(floor lowered to 8 GB, bench binaries removed instead of the target) had 10–13 GB
free throughout. `df -g /` was run as its own command before every cargo call.
One process breach happened in the first window: `df` was chained with `cargo` in
a single command, so a 7.84 GB reading could not stop that build. It was not
repeated.

## Follow-ups (not done, outside this lane)

- `HEADLESS_DISPATCH_POLICY_CONFIG` is still a `OnceLock` set by a real test.
  `e0f12037` isolates the one victim, but any future test that assumes headless
  is off will hit the same trap. Making the policy injectable (or a resettable
  lock) is the durable fix, and it is a production change.
- `dependency_claim_guard` matches `DEPENDENCY_MARKERS` as raw substrings (`pin`
  inside "looping"). Word-boundary matching would prevent the next false positive.
- Lane R57a (goal_advance dispatch path) was not touched or merged.
