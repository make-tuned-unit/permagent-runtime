# Lane T7 — the headless dispatch policy becomes a resettable seam

Branch `lane/T7`, based on `0514278c`.

| item | commit |
| --- | --- |
| 1. storage + production API | `e973f608` Make the headless dispatch policy a resettable slot, not a OnceLock |
| 2. test guard + serial key | `e12642de` Guard the one test that installs the headless dispatch policy |
| 3. workarounds removed | `a9218a9e` Drop the workarounds that only existed because the policy could not be reset |

## The defect

`HEADLESS_DISPATCH_POLICY_CONFIG` was a process-global
`OnceLock<HeadlessDispatchPolicyConfig>` in
`crates/goose/src/agents/platform_extensions/orchestrator.rs`, installed via
`let _ = ....set(..)`. That is exactly the contract production wants (first
install wins, a second call is a harmless no-op, a daemon never installs one
at all) but it gave `cargo test -p permagent --lib` a single process-wide cell
with no way to unset. `dispatch_goal_to_and_resume_roadmap_consult_the_installed_headless_policy`
calls the real `install_dispatch_hook_for_headless_session`; it already
restored the *dispatch hook* it took (Lane T1's `RwLock` seam) but nothing
could empty the policy cell, so from that test onward every test in the binary
read "this process is headless" as true. Each full-suite run picked a
different victim.

## The seam

The change mirrors, deliberately and line for line, what Lane T1 already did
to `GOAL_DISPATCH_HOOK_SLOT` for the identical reason — one precedent, one
shape, so the two globals in this file now behave the same way.

```rust
type HeadlessDispatchPolicyConfigHandle = std::sync::Arc<HeadlessDispatchPolicyConfig>;

static HEADLESS_DISPATCH_POLICY_SLOT: std::sync::OnceLock<
    std::sync::RwLock<Option<HeadlessDispatchPolicyConfigHandle>>,
> = std::sync::OnceLock::new();
```

- `headless_dispatch_policy_config() -> Option<Arc<HeadlessDispatchPolicyConfig>>`
  is the single read. It clones the `Arc` out from behind the read lock and
  returns, so a reader never holds a guard across an `.await` — the rule
  `goal_dispatch_hook_snapshot` already documents for the sibling slot. Every
  former `HEADLESS_DISPATCH_POLICY_CONFIG.get()` is now this call; the closure
  arguments of `is_some_and` / `map` see an `Arc`, which derefs identically, so
  no call site changed shape.
- `install_headless_dispatch_policy_config(config)` keeps production's
  first-install-wins contract byte for byte and logs the no-op second call at
  `debug` on `permagentd::brain`, the way `install_goal_dispatch_hook` does.
  `install_dispatch_hook_for_headless_session` is its only caller.
- `HeadlessDispatchPolicyTestGuard` + `take_headless_dispatch_policy_for_test()`
  are both `#[cfg(test)]`. The guard's `Drop` restores the previous value.
  **There is no production path that can uninstall a policy** — the type is not
  constructible outside `cfg(test)`, so this is the "guard type only
  constructible in tests" option, not merely a `cfg`-gated reset function.

Unchanged, on purpose: `current_headless_dispatch_policy`,
`headless_auto_land_status` and `is_headless_dispatch_policy_installed` have
the same signatures and the same observable behaviour. A daemon process still
never installs a policy, so it still reads `None` forever.

Comments in this file and in `crates/goose/src/agents/reply_parts.rs` that
described the old "set once, stays set forever, nothing can reset it" hazard
now describe the seam that exists. Three of them were the *reason* a
neighbouring design choice was made (e.g. `record_escalation` and
`maybe_headless_unblock_verify_gap` taking `headless` as an explicit
parameter); those choices are still right on their own merits and were left
alone, with the justification corrected.

### Why a guard is not enough on its own

`cargo test` runs `#[tokio::test]`s in parallel in one process. A
restored-on-drop slot is still installed *while* the installing test runs, so
the guard bounds the leak in time, not across threads. The key
`serial(headless_dispatch_policy)` bounds it across threads. The convention,
documented on `take_headless_dispatch_policy_for_test`:

- a test that **installs** a policy holds the guard **and** takes the key;
- a test whose assertions assume **no** policy is installed takes the same key
  and needs no guard.

## Converted tests

`dispatch_goal_to_and_resume_roadmap_consult_the_installed_headless_policy` is
the only test in the crate that installs the policy (`grep
install_dispatch_hook_for_headless_session` finds one other reference, the
production call in `crates/goose-cli/src/session/builder.rs`). It now holds
`take_headless_dispatch_policy_for_test()` beside the
`take_goal_dispatch_hook_for_test()` it already held, and is keyed
`#[serial_test::serial(receipts_global_registry, routing_env, headless_dispatch_policy)]`
— one attribute, the new key folded in alongside the existing two, never
stacked.

Tests that assume the policy is absent and now share the key:

| test | previous attribute | now |
| --- | --- | --- |
| `repeated_premature_done_parks_after_max_holds` | none | `serial(headless_dispatch_policy)` |
| `r62g_create_roadmap_refuses_a_blocker_titled_goal_in_a_real_repo` | `serial(receipts_global_registry)` | `serial(receipts_global_registry, headless_dispatch_policy)` |
| `r62g_create_roadmap_lets_a_remove_blocker_flag_goal_through` | `serial(receipts_global_registry)` | `serial(receipts_global_registry, headless_dispatch_policy)` |
| `goals_json_that_differs_from_the_pending_proposal_still_requires_the_correct_hash` | `serial(receipts_global_registry)` | `serial(receipts_global_registry, headless_dispatch_policy)` |

The other two confirmed victims needed no change:
`dispatch_of_a_goal_this_session_never_created_increments_verification_integrity_goals_r41a`
already shares `receipts_global_registry` *and* `routing_env` with the
installer test (commit `0514278c`), so it can never run concurrently with it,
and the guard removes the after-the-fact leak that was still failing it with
`Unknown provider: mock-roadmap`. `a_quiet_boot_still_reports_that_it_looked`
has no policy read on its path at all (`reconcile_in_progress_goals` →
`reconcile_in_progress_goals_with_dispatch` emits its report line
unconditionally) and was left untouched, on the evidence of the suite runs
below rather than on a guess about the mechanism.

## Workarounds removed

- **`e0f12037`, `repeated_premature_done_parks_after_max_holds`** — the
  pre-spent R18g `headless_unblock_count: 1` and its 18-line explanation are
  gone, and the original assertion (`needs_human_attention == true` plus an
  open `unblock` decision) stands unchanged. With no policy installed,
  `maybe_hold_review`'s `HoldOutcome::Park` arm reaches the park instead of
  R18g's headless unblock, deterministically.
- **`98780e3a`, both R62g `create_roadmap` tests** — the
  `if !is_headless_dispatch_policy_installed()` branch and the two-armed
  `match` are gone. The exact refusal prefix, the `` `escalate` `` hint, the
  "a refused create_roadmap must create no goal cards" assertion and the
  hash-check error are asserted unconditionally again.
  **Kept**: the stronger invariant that commit added to the refusal test — no
  BLOCKER-titled card is ever materialized, whatever the outcome. That is not
  a mode workaround, it is a genuinely better assertion, so it stays alongside
  the restored ones.
- **`38a49a95`, `goals_json_that_differs_from_the_pending_proposal_still_requires_the_correct_hash`**
  — the `is_headless_dispatch_policy_installed()` arm asserting the f9ecdb5a
  canonical-proposal recovery is gone; the hash-mismatch error is asserted
  unconditionally. The `serial(receipts_global_registry)` key that commit
  added stays (it was never only about the policy: the recovery path
  materializes 2 goal cards and could bump the R41a snapshot).
  The headless canonical-proposal recovery this arm exercised **is** real
  production behaviour, so it deserves its own test that drives
  `resolve_create_roadmap_proposal_hash` with `headless: true` — the way
  `headless_third_consecutive_mismatch_auto_approves_the_pending_proposal`
  already does — rather than a branch inside a test of the other mode. That
  test is not in this lane; the behaviour is not currently covered end to end
  through an installed policy, and that is a coverage gap this lane records
  rather than closes.

**Deliberately kept**: the R20b-e salvage-ordering tests near
`hold_exhaustion_park_with_uncommitted_work_parks` also pre-spend R18g's
`headless_unblock_count`. Those were not workarounds for the `OnceLock` — the
section's own comment says they pre-spend so each sub-test observes the
salvage-vs-park *ordering* rather than R18g's eligibility math, which its own
test suite already covers. Their comments are corrected to describe the
concurrency hazard that remains rather than the "set once, never reset" one
that does not, and the pre-spends stay.

Nothing else in the file branches on the policy for test-determinism reasons:
`grep is_headless_dispatch_policy_installed` now finds only the production
readers in `after_turn.rs` and `reply_parts.rs`.

## Verification (M1 build host)

All on the M1 build host, `~/build/harness-loop` at `a9218a9e`, with
`ulimit -n 8192`, `CARGO_INCREMENTAL=0`,
`RUSTUP_TOOLCHAIN=1.97-aarch64-apple-darwin`, `CARGO_BUILD_JOBS=2` and a
dedicated `CARGO_TARGET_DIR`. `pgrep -x cargo` was empty and `df -g ~` showed
18 GiB free before each batch.

`cargo test -p permagent --lib`, three times back to back:

```
===== RUN 1 =====
test result: ok. 5646 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 232.14s
===== RUN 2 =====
test result: ok. 5646 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 226.25s
===== RUN 3 =====
test result: ok. 5646 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 221.93s
```

`cargo test -p permagent --lib -- dispatch_of_a_goal_this_session_never_created --nocapture`
(the R41a victim, alone):

```
test agents::platform_extensions::orchestrator::tests::dispatch_of_a_goal_this_session_never_created_increments_verification_integrity_goals_r41a ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5652 filtered out; finished in 1.48s
```

`cargo test -p permagent --lib -- --test-threads=1` (single-threaded ordering
exposes installer leakage differently — this is the ordering under which
`e0f12037` originally reproduced the leak):

```
test result: ok. 5646 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 236.26s
```

`cargo clippy -p permagent --lib -- -D warnings`:

```
(no output: zero errors, zero warnings)
```

The test binary itself built clean (`Finished test profile ... in 2m 42s`),
with one pre-existing, unrelated linker warning about the `__eh_frame`
section size that this branch does not introduce.

## Risks

1. **Concurrency, not order, is now the residual hazard.** The guard cannot
   stop a test running on another thread from observing an installed policy.
   The `headless_dispatch_policy` key is the whole defence, so any future test
   that either installs a policy or assumes its absence must take that key.
   The doc comment on `take_headless_dispatch_policy_for_test` states the
   convention; nothing mechanically enforces it.
2. **A lock where there was a `OnceLock`.** Reads are now an `RwLock::read`
   instead of an atomic load. Every reader is on a dispatch/completion path
   that already does database work, the guard is released within the
   statement, and no guard crosses an `.await`, so deadlock and contention are
   not credible — but it is a real change to a hot-ish global.
3. **`.unwrap()` on a poisoned lock.** `install_headless_dispatch_policy_config`,
   `headless_dispatch_policy_config` and the guard's `Drop` all `unwrap()` the
   lock, exactly as the sibling `GOAL_DISPATCH_HOOK_SLOT` functions do. A
   panic while the write guard is held would poison the slot and every later
   read would panic. The critical sections contain no fallible work beyond an
   `Arc::new`, and consistency with the sibling seam is worth more here than a
   divergent recovery path.
4. **Coverage gap, carried forward.** Removing 38a49a95's headless arm means
   the canonical-proposal recovery under an installed policy is no longer
   touched by that test. See the note above; the seam now makes a proper test
   for it cheap to write (hold the guard, install, take the key).
5. **Lost mode-independence.** The four converted tests now depend on the
   serial key for their determinism where they previously depended on nothing.
   That is the intended trade — assertions that say what they mean, protected
   by an explicit key — but a future edit that drops a key silently
   reintroduces the flake as a *concurrent* one, which is harder to bisect
   than the old deterministic-after-installation leak.
