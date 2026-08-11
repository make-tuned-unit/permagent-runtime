# Outcome

Implemented durable Decision Inbox follow-up for approved goal landings that return `LandOutcome::NeedsMerge`.

## Changes

- Updated `crates/goose-server/src/routes/decisions.rs` so `land_approved_goal` creates an open `unblock` decision when a branch cannot be fast-forwarded onto trunk.
- Added deduplication through `decisions::find_open_decision_for_goal`, so a goal never receives a second open `unblock` decision.
- Added a human-readable headline and detail containing the goal title/id, worker branch, trunk branch, and exact copy-pasteable merge and rebase command sequences.
- Surfaced inbox-persistence failures through the existing post-approval warning path while preserving the landing outcome in the approval effect.
- Added focused tests for open-decision deduplication and the complete decision payload shape.

## Files touched

- `crates/goose-server/src/routes/decisions.rs`
- `OUTCOME.md`

## Verification and caveats

- Ran `cargo fmt` and `git diff --check` successfully.
- Did not run `cargo build`, `cargo check`, `cargo clippy`, or `cargo test`, as required by the assignment. The new tests are written but await the central gate.
- The requested local commit could not be created: this lane's Git index is stored at `/Users/jessesharratt/dev/permagent-runtime/.git/worktrees/needsmerge`, which is read-only in the current environment. `git add` failed while creating `index.lock`; the completed changes remain unstaged in the lane.
