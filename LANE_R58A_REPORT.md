# Lane R58a: a headless coordinator asks the user to pick routing, then stops

Branch `lane/R58a`, based on `f9ecdb5a`. Edits only. No cargo was run, so the coordinator still has to run the gates.

## Defect

In record r59 q4, fixture `select-checks-worker-before-dispatch` was seeded with a Ready goal card. DeepSeek called `check_worker` and `routing_roster`. It then ended the turn in prose: "One thing I need from you before I dispatch: the routing choice for this card is user-visible spend … I won't pick that one silently." Receipts showed `dispatches=0`, `escalated=0` and `goals=0`. `must_call goal_advance` failed, and so did `verification_integrity`.

Record r56 m3 failed the same way. It called `routing_roster` and `list_workers`, then wrote "I'm ready to dispatch … Shall I dispatch now? One caveat …", so the question mark was in the middle of the text, not at the end.

## Root cause (line numbers are at base f9ecdb5a, `crates/goose/src/after_turn.rs`)

Every after-turn hook ran, and every one of them allowed the turn to end:

1. **`RoadmapDispatchGuard`** (X-dag-2): its "goal … is not dispatched" nudge only reads cards the session created itself (`undispatched_goal_for`, `goal_cards_created_by_session`, 1510-1516). The fixture's card is seeded, so this session never created it and the guard saw nothing.
2. **`NoRoadmapGuard`** (Y5): `decide` allows the turn whenever `has_roadmap_cards` is true (1881-1887). The seeded card is found through `goal_cards_relevant_to_session` (R20c-c, 1700-1716), so "no roadmap" is false. Its R31b stranded-Ready nudge (1980-1989) needs R26b's `resumed_by_headless_unblock_at` stamp, which this card doesn't have.
3. **`VerifiedGoalAwaitingLandGuard`** (R35a): it needs `dispatch_evidence.verdict.status == "pass"` (2126-2135). Nothing had run.
4. **`Rule5HonestFailureGuard`** (R18f/R20c/R20c-b) is the real gap. The turn passed its cheap gates (no tool call at the end, no `escalate`, Auto mode, top-level harness), and it has cards, so the bare-session shape doesn't apply. The mid-session shape also needs `final_message_reads_as_question` (`decide`, 2459-2464). That check looks only for a trailing `?` or a whole-word match against `RULE5_QUESTION_PHRASES` = `please provide | which | should i | let me know | confirm` (2502-2508, 2570-2581):
   - q4 ended with "silently.", and the text only contains "confirmation" and "confirmed", which aren't whole-word matches for `confirm`. `reads_as_question == false`, so `decide` returned `Allow`. I reproduced this against the log text.
   - m3 had its `?` in the middle ("Shall I dispatch now?"), and "shall i" isn't in the list, so the result was the same.
5. **R24b** (the mint, 2784) and **R26a** (the auto-answer, 2817-2860) only run on a rule-5 `Park`, which never happened. R40a/R40a-b only filter escalations that already exist.

In short, the phrase heuristic missed this wording. With a seeded Ready card and nothing dispatched, no guard keyed on durable state rather than on phrasing, so the turn-end path let a silent exit through.

## Fix

### (a) Recipe: `crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`, rule 3 (lines 75-79)

I appended one sentence to rule 3, the `routing_roster`/`worker_tier` rule:

> Choosing the worker and its `worker_tier` from the live `routing_roster` is your own decision, never the user's: pick the cheapest tier the roster marks capable for the goal (a stumble escalates to a stronger tier), dispatch it, and never end a turn asking the user to choose routing or spend. Only a missing credential or a spend a budget refuses is an `escalate`.

It agrees with exception (b)(2) (credentials escalate), with the cost section's "escalate a sub-task to a stronger tier only if the cheaper one stumbles", and with the S4 goal budgets. The roster itself has no spend caps, so the sentence doesn't claim any. `search_recipe.rs::builtin_permagent_coding_recipe_resolves_and_parses` now also asserts both key phrases (lines 136-144).

### (b) Harness backstop in the existing guard: `Rule5HonestFailureGuard`

I added a third shape to the existing guard rather than writing a new hook, and it reuses the existing guard machinery:

- `RULE5_READY_GOAL_INJECT_TEXT` / `RULE5_READY_GOAL_PARK_REASON` (2364 / ~2376). The nudge says "Headless run: no user will answer … make the routing choice yourself … call `goal_advance` with action `dispatch` (or `resume_roadmap` if the roadmap is paused), or call `escalate` with a concrete blocker". Its marker is `rule5_ready_goal_undispatched`.
- `DISPATCH_PREP_TOOL_NAMES` (2387): `check_worker | list_workers | routing_roster | goal_list | goal_status | goal_advance`. The shape only fires if at least one of these was attempted, so the coordinator was actually preparing a dispatch.
- `has_undispatched_ready_root_goal` (2404): a root card in `ready` with no worker session. This is the first clause of `cards::first_undispatched_goal`.
- `Rule5HonestFailureGuard::decide_undispatched_ready_goal` (2566) is a pure function in the same form as `decide`. It passes through any non-`Allow` result from `decide`, so the existing shapes and their tests don't change. It returns `Allow` unless all of these hold:
  - a true headless CLI run (`is_headless_dispatch_policy_installed()`, the signal R26a uses; Auto mode alone isn't enough);
  - an undispatched Ready root goal exists;
  - dispatch preparation was attempted;
  - no `goal_advance` succeeded anywhere in the transcript.

  After that it follows the R20c-b two-strike rule: first strike (`consecutive_free_text_coordinator_turns == 1`) returns `Continue` with the nudge; second strike, or the shared-holds ceiling, returns `Park`.
- `after_turn` (2881-2902) calls it only when `decide` returned `Allow`, which means the guard's gates had already passed: the final message has no tool call, no escalate, Auto mode, top-level harness, and no roadmap tool succeeded.
- On `Park`, R24b's redirect now fires for `reads_as_question || ready_goal_shape` (2918). So the second strike always goes through `mint_rule5_question_escalation`, the same path, sink and `dag_enforcement.escalated` receipt R24b uses, including R26a's headless information-answer follow-up. The run ends with `escalated=1` and exit 0.
- If the mint fails, the existing honest-failure park still happens, now with `harness_error = "rule5_ready_goal_undispatched"` (3021) and `RULE5_FREE_TEXT_TURN` set.
- It never dispatches for the model. It adds no new receipt counter and leaves the qualification grader alone.

## Gates

```
cargo test -p permagent --lib r58a_
cargo test -p permagent --lib rule5_
cargo test -p permagent --lib after_turn::tests
cargo test -p permagent-cli --lib builtin_permagent_coding_recipe_resolves_and_parses
rustfmt --edition 2021 --check crates/goose/src/after_turn.rs crates/goose-cli/src/recipes/search_recipe.rs   # already passes
```

New tests in `after_turn.rs` (6557 onward):

- `r58a_the_r59_q4_routing_ask_slips_past_the_mid_session_question_shape`: reproduces the gap using q4's verbatim final sentence.
- `r58a_first_free_text_turn_with_an_undispatched_ready_goal_nudges`: headless, seeded Ready goal, strike 1 → nudge.
- `r58a_second_free_text_turn_parks_and_is_redirected_into_a_minted_escalation`: strike 2 → `Park` with the marker, and the R24b redirect predicate is true even though the text isn't a question.
- `r58a_interactive_session_with_the_same_free_text_is_not_guarded`: no headless policy → `Allow` at both strikes.
- `r58a_a_turn_that_dispatched_is_not_guarded`: covers these cases:
  - a successful `goal_advance` → `Allow`;
  - a refused `goal_advance` → still nudged;
  - no Ready goal → `Allow`;
  - no dispatch-prep tool → `Allow`;
  - an existing `decide` result passes through unchanged.
- `r58a_has_undispatched_ready_root_goal_only_matches_a_never_dispatched_ready_root`
- `r58a_a_seeded_ready_card_reads_as_undispatched_through_the_session_scope` (tokio, global `SessionManager` pool): a card created by a different session in a cwd-matched project is seen and counts as undispatched. It uses the same pattern as the neighbouring R20c-c/R31b tests and needs no serial key, because it doesn't touch receipts or the sink global.

No test sets a process-global OnceLock. None needs `#[serial]`, because no new test touches `receipts`.

## Snapshots

None need regenerating. The `crates/goose/src/agents/snapshots/permagent__agents__prompt_manager__tests__*.snap` files don't contain the recipe's instructions (checked with grep for `routing_roster`/`worker_tier`/`NON-NEGOTIABLE`). The other `include_str!` consumers of the recipe (`summon.rs` 3595, `orchestrator.rs` 44723) only read the `extensions` list.

## Overlap with lane/R57a (`5f8715f6`)

- **Recipe:** R57a edits exception (a) (lines 33-38) and the header comment (lines 21-24). My sentence goes at the end of rule 3 (after line 74) and I left the header comment alone on purpose, so the hunks are separate. The coordinator can add a header note ("Lane R58a made worker/worker_tier choice the coordinator's own…") at merge time. My nudge's "(or `resume_roadmap` if the roadmap is paused)" is consistent with R57a's refusal to dispatch a paused roadmap one goal at a time.
- **`search_recipe.rs`:** R57a appends after the "SMALLEST executable graph" assertion (after line 140). I inserted after the `developer.available_tools()` assertion (after line 135), with five unchanged lines between, so this should merge cleanly. If it doesn't, keep both assertions.
- **`orchestrator.rs`:** I didn't touch it. R57a's `handle_goal_advance` Dispatch refusal returns `Err`, which `turn_tool_call_succeeded` reads as not dispatched. So a coordinator whose paused-roadmap dispatch was refused and who then ends in prose gets this nudge, which points it at `resume_roadmap`. That interaction is intended.

## Risks

- **False positives in real headless runs.** A headless CLI coding run whose cwd project has a stale, never-dispatched Ready root card, where the coordinator called `routing_roster` (rule 3 says to call it once per session) for an unrelated task and then ended in prose, would get one nudge and then a minted escalation. The dispatch-prep gate and the root/no-worker-session gate make this narrow. The nudge also lets the model escalate a concrete blocker instead.
- **"Headless" means the goose-cli builder installed the headless dispatch policy** (coding harness plus Auto mode, `builder.rs:1321`). That is the signal R26a already relies on. An interactive `permagent session` in Auto mode on this recipe would satisfy it too, but R26a has the same exposure. The desktop app (goose-server) never installs it.
- **The second-strike mint uses R24b's fixed headline**, "A coordinator question needs a human decision", even when the prose isn't strictly a question. `why_blocked` keeps the model's text verbatim. If the text contains " or ", the kind is `decision`, which stays open and gives `escalated=1`. Otherwise it's `information`, which R26a may auto-answer "proceed on your best reading" (capped at 2 per session) and continue the turn. That is acceptable here, because the honest answer to a routing ask is to choose yourself.
- **R24b + R26a on repeated turns.** On the ready-goal shape, strike 3 and later also park and mint. That is already true of R24b for question-shaped text, and R26a's cap and the reply loop's hold ceiling bound it.
- **Unverified.** No compile or test has run, so the gates above are the check. I only checked the logic by reading, plus rustfmt.
