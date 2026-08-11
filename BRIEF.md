# Assignment: durable decision-inbox item for NeedsMerge goal landings

## Problem
When approving a goal whose landing returns `LandOutcome::NeedsMerge`
(`crates/goose/src/goal_landing.rs`), the outcome is only a string on the
approval response — nothing durable survives. A human who misses the response
has no record that the goal's branch still needs a merge/rebase.

## Task
Make `land_approved_goal` (`crates/goose-server/src/routes/decisions.rs`) ALSO
create a decision-inbox item when the landing outcome is `NeedsMerge`:

- kind: `"unblock"` (see the `permagent::decisions::create_decision` usage
  nearby in the same file/module for the call shape)
- title: phrased so a human understands the outcome at stake (e.g. that the
  goal's work landed on a branch that could not be fast-forwarded and needs a
  manual merge or rebase)
- detail: must name
  - the goal (id and/or title)
  - the branch the work is on
  - the trunk it needs to land on
  - the EXACT git commands to merge or rebase (copy-pasteable)

## Dedupe
Do NOT create a second open decision for the same goal. Use
`decisions::find_open_decision_for_goal` (already exists) to check first.

## Tests
Write focused tests alongside the code for:
1. Dedupe: a second NeedsMerge landing for the same goal does not create a
   second open decision.
2. Payload shape: the created decision has kind "unblock", and its title/detail
   contain the goal, branch, trunk, and the git commands.

## Hard rules
- Work ONLY in this directory: /Users/jessesharratt/dev/permagent-runtime/.codex-lanes/needsmerge
- Do NOT run cargo build / cargo check / cargo clippy / cargo test — the
  machine's disk cannot host more build trees. A central gate runs after merge.
  `cargo fmt` alone is allowed.
- Write the tests even though you cannot run them.
- Commit your work on this branch (codex/needsmerge-decision) with a clear
  message. Do NOT push.
- When done, write OUTCOME.md in this directory summarizing what you changed,
  files touched, and any caveats.
