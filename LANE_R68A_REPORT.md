# Lane R68a: the independent-review timeout must be an infrastructure retry, not a verdict

Branch `lane/R68a` (base `b657a1c0`). Edit-only — cargo was not run. `rustfmt --edition 2021 --check` passes on the one touched file.

## Evidence (M53, `b657a1c0`, r68 s2, `dschat-harness-cli-argparse-log-filter`)

The goal card's `auto_land_review.refusal`:

> minimax/MiniMax-M2.5 was UNCERTAIN (review verdict is UNCERTAIN — default-to-reject; do not
> declare done until an independent reviewer approves) and no escalation review could run: the
> independent reviewer did not respond within 120s

The cost ledger shows `anthropic/claude-fable-5` (the escalated reviewer) was charged $0.14 for
that call — it was working, not absent. The single 120s `REVIEWER_GATE_TIMEOUT` constant in
`crates/goose/src/agents/platform_extensions/orchestrator.rs` cut it off before it could answer,
and the goal parked with nothing landed.

## Root cause

`REVIEWER_GATE_TIMEOUT` (120s) bounded every reviewer completion call — the salvage path AND the
auto-land/escalation path share the same `call_reviewer_provider` → `call_reviewer_provider_checked`
plumbing. On timeout, the old code returned `ReviewerCallFailure::Unavailable("...did not respond
within 120s")` — the SAME classification as a billing/auth/outage error — so it fell straight into
`finish_goal_review_with_escalation`'s `Err(why)` arm, which worded the park as if the FIRST
reviewer's genuine UNCERTAIN was the whole story and treated the timed-out escalation as simply
"could not run", with no retry at any layer.

## Changes

All in `crates/goose/src/agents/platform_extensions/orchestrator.rs`.

### 1. Two independent, named, documented timeout constants

- `REVIEWER_GATE_SALVAGE_TIMEOUT_SECS: u64 = 120` — the unverified salvage path's timeout,
  **unchanged from the original value**. Chosen because salvage already runs after the goal's own
  verify budget and attempt cap are spent; with the new one-retry policy it now gets up to 240s of
  wall time before it errors, which is proportionate to a goal that was already failing
  verification. It does not read from the env knob.
- `DEFAULT_REVIEWER_GATE_TIMEOUT_SECS: u64 = 300` — the auto-land/escalation review's default
  (2.5x the old bound), because that review is the only gate between "nothing verified this by a
  human" and a goal landing unattended, and the evidence shows a stronger, slower reviewer can
  still be doing real, billed work past 120s.
- `PERMAGENT_REVIEWER_GATE_TIMEOUT_SECS_ENV = "PERMAGENT_REVIEWER_GATE_TIMEOUT_SECS"` — the env
  override for the auto-land default, following the exact `PERMAGENT_<NAME>_ENV` /
  `DEFAULT_<NAME>` / `<name>_from_env()` idiom already used by
  `PERMAGENT_HEADLESS_SWEEP_HEADROOM_SECS_ENV` and `PERMAGENT_HEADLESS_COMPLETION_INFLIGHT_SECS_ENV`
  (raw `std::env::var`, `.trim().parse::<u64>()`, fall back to the default on absence or a parse
  error).
- `review_kind_timeout(kind: &GoalReviewKind) -> Duration` picks between the two based on
  `GoalReviewKind::Salvage` vs `GoalReviewKind::Verified`, so the two paths can never again share
  one constant by accident.

### 2. Timeout is an infrastructure failure, not a verdict — retried once, then `errored`

- `ReviewerCallFailure` gained a `TimedOut` variant, produced ONLY by `call_reviewer_provider`'s own
  `tokio::time::timeout` wrapper elapsing (a provider-reported timeout embedded in a completion
  error string is unchanged — it still classifies through `classify_unavailable` as `Unavailable`,
  since that's a provider-side signal, not our own client-side cutoff).
- New `run_reviewer_call_cycle<F, Fut>(attempt_call: F) -> ReviewCycleOutcome`: calls
  `attempt_call()` once; on `TimedOut`, retries the SAME reviewer with the SAME prompt at the full
  timeout exactly once; a second `TimedOut` becomes `ReviewCycleOutcome::Errored { reason,
  elapsed_secs }`. `Unavailable`/`Other` are never retried here (they already have their own
  ladders — the session-tier fallback, the escalation). It is generic over `attempt_call` so tests
  inject a deterministic fake instead of a real provider call and a real multi-minute wait — no
  sleeping.
- `call_reviewer_provider_with_timeout_retry` wires that generic cycle to the real provider call.
- `ReviewRound` gained an `Errored { reason, elapsed_secs }` variant — `label() == "errored"`,
  distinct from `"uncertain"` and `"request_changes"`. `ReviewAttempt` gained an `attempts: u32`
  field (1 normally, 2 after a timeout+retry, whether the retry answered or errored), included in
  `to_json`. An `Errored` round's `to_json` also carries `elapsed_secs`.
- Both call sites (`review_goal_diff_with_cross_family_reviewer`'s first attempt, and
  `escalated_goal_review`'s escalated attempt) now go through
  `call_reviewer_provider_with_timeout_retry` at `review_kind_timeout(kind)`, and build a
  `ReviewAttempt { round: ReviewRound::Errored { .. }, .. }` on a double timeout instead of
  returning early — so it falls through to `finish_goal_review_with_escalation` exactly like a real
  UNCERTAIN.

### 3. The escalation ladder treats `Errored` like `Uncertain`, but never claims it "was UNCERTAIN"

- `finish_goal_review_with_escalation`'s `first_unsure` now includes `ReviewRound::Errored`, so a
  timed-out-twice first round escalates once (when `escalate_uncertain`, i.e. `GoalReviewKind::
  Verified` — matches Lane R65a's existing scope; the salvage path still never escalates, on a
  timeout or otherwise, unchanged).
- New `ReviewAttempt::describe_inconclusive()` words a round for the ladder's refusal text: a real
  `Uncertain`/`CallSiteTableIncomplete` still reads `"{reviewer} was UNCERTAIN (...)"` (byte-for-byte
  unchanged from Lane R65a/R66a); `Errored` reads `"{reviewer} did not respond after 2 attempts
  (Ns elapsed)"` — never the word "UNCERTAIN".
- Every escalation-ladder outcome that involves an `Errored` round (no escalation reviewer
  available; the escalated reviewer also errors) now says the review **"could not be obtained"**
  and never says "was UNCERTAIN" or "both reviewers were UNCERTAIN". A lone `Errored` round with no
  escalation attempted at all (the salvage path) maps to `ReviewerGateOutcome::Unreachable` via
  `ReviewAttempt::into_outcome()` — the SAME "nothing ever looked at the diff" semantics
  `ReviewerGateOutcome::Unreachable` already carries for a dead provider, so the salvage decision
  audit (`park_salvage_the_reviewer_unreachable` vs `_rejected`) is unaffected.
- A genuine double-UNCERTAIN (no timeout involved anywhere) is byte-for-byte unchanged: still
  "both reviewers were UNCERTAIN: ...".

### 4. Spend honesty

`cost_router::routing_receipt`'s `cost_routing dispatches` counter is fed only by
`dispatch_goal_fn` (ordinary goal/worker dispatch) and, per Lane R63c, the `delegate`/`delegate_many`
tool-call path (`record_delegate_dispatch`). **The goal-level cross-family reviewer gate this lane
touches (`call_reviewer_provider` and everything built on it) was never wired into that counter
before this change, for either the first call or a retry** — R63c's own report calls this out as a
known, separate gap ("Goal-level review gate not changed... outside this lane... a follow-up").
This lane does not add that wiring; doing so is a distinct piece of work already tracked as a
follow-up, not part of the R68a defect. What this lane DOES guarantee: the retry is a REAL second
model call, routed through the exact same `AccountedFastCompletion::complete_fast_accounted` path
as the first attempt, so it is billed and appears in the session's cost ledger like any other call
— nothing here fakes or skips accounting for the retry, and nothing double-counts it either (a
retry is a genuinely separate call, not the same call counted twice).

## Tests (all pure or fake-seamed — no real timeouts, no sleeping)

New module `r68a_tests` in `orchestrator.rs`:

- `reviewer_gate_timeout_env_name_and_default_match_the_family` — constants/knob values.
- `review_kind_timeout_is_proportionate_to_the_kind_and_the_env_overrides_the_default` — salvage
  vs verified pick the right constant; the env knob overrides the verified default and never
  affects salvage. `#[serial_test::serial]` (env-var mutation, no OnceLock).
- `a_reviewer_that_times_out_once_then_answers_is_recorded_with_two_attempts` — drives
  `run_reviewer_call_cycle` with a counter-based fake attempt (no sleeping): one timeout then an
  answer → `Responded { attempts: 2 }`; an immediate answer → `Responded { attempts: 1 }`.
- `a_reviewer_that_times_out_twice_is_recorded_as_errored_not_uncertain` — two fake timeouts →
  `Errored`; asserts the round's label is `"errored"` (never `"uncertain"`/`"request_changes"`),
  its JSON carries `attempts` and `elapsed_secs`, and a lone errored round's `into_outcome()` is
  `Unreachable` with wording that never contains "UNCERTAIN".
- `errored_first_round_escalates_when_a_stronger_reviewer_is_available` — bullet 2: an errored
  first round with an escalation available lands via the stronger reviewer's approval; `reviews`
  records the errored round then the approval.
- `errored_first_round_with_no_stronger_reviewer_parks_distinctly_from_uncertain` — bullet 3: no
  escalation reviewer → parks with "review could not be obtained", never "was UNCERTAIN".
- `errored_escalated_round_also_parks_distinctly_from_uncertain` — the escalated reviewer also
  errors twice → same distinct wording, both rounds recorded.
- `a_real_uncertain_is_unaffected_by_the_errored_wording` — bullet 4: a genuine double-UNCERTAIN
  keeps the exact R65a wording ("both reviewers were UNCERTAIN"), never "could not be obtained".
- `salvage_and_auto_land_timeouts_are_independent_constants` — regression guard against
  re-merging the two constants.

## Gate filters

```
cargo test -p permagent --lib r68a_tests::
cargo test -p permagent --lib r65a_tests::
cargo test -p permagent --lib r66a_tests::
cargo test -p permagent --lib reviewer_provider_unavailable
cargo test -p permagent --lib review_round_from_response
```

The first is the new coverage; the rest are the existing suites this lane's wording/enum changes
touch, run as a regression check (unaffected: the genuine-UNCERTAIN and CALLSITE-table wording is
byte-for-byte preserved).

## Risks / follow-ups

- **`cost_routing dispatches` still does not count the goal-level reviewer gate at all** (first
  call or retry) — a pre-existing gap (R63c's own noted follow-up), unchanged by this lane.
- **The salvage path's own timeout is untouched by the env knob.** If a slow local/edge model
  needs more than 120s+one retry for salvage specifically, that needs its own knob — deliberately
  out of scope here since the evidence and the "at least 300s" requirement are about the auto-land
  path.
- **A reviewer that answers on the retry is recorded as one round with `attempts: 2`**, not as two
  separate `reviews` entries (one `errored`, one `approved`). This was a judgment call: the same
  reviewer/model produced the round's final verdict, and folding the retry into that round's own
  JSON (`attempts`, and `elapsed_secs` when it errors) keeps `reviews` indexed by REVIEWER-CYCLE
  (matching its pre-existing meaning: round 1 = first reviewer, round 2 = escalated reviewer)
  rather than by raw HTTP attempt.
- **Escalation now also fires on a double-timeout for `GoalReviewKind::Verified`**, which was
  previously impossible (a timeout always parked immediately). This is the fix, but it does mean a
  verified goal's review can now spend up to `2 × DEFAULT_REVIEWER_GATE_TIMEOUT_SECS` (first
  reviewer's two attempts) plus another `2 × DEFAULT_REVIEWER_GATE_TIMEOUT_SECS` (escalated
  reviewer's two attempts) — up to ~20 minutes wall time in the worst case — before parking. Given
  auto-land is headless and unattended, this trades latency for not throwing away a working
  reviewer's answer, which matches the evidence and the task's intent, but it's worth knowing if a
  drain/wallclock cap sits tightly above this call.
