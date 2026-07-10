---
description: Scaffold a Permagent build dispatch from an issue number — isolated worktree, audit-first, gated, evidence-not-assertion. Give it the issue number and a one-line goal.
---

# Permagent Build Dispatch

You are running a Permagent build dispatch. Follow this structure. It encodes
hard-won disciplines — they are CONSTRAINTS, not optional steps. Do not
railroad past them, but do figure out the implementation yourself.

## Inputs
- Issue/goal: $ARGUMENTS (an issue number and/or a one-line goal)

If only a number was given, run `gh issue view <n>` first to load the issue.
If the goal is ambiguous or a design/product decision (not mechanical), STOP
and tell Jesse this is Tier-2 work that needs his ruling, not an auto-build.

## Phase 0 — AUDIT FIRST (non-negotiable)
Before writing ANY code:
1. Reconcile against existing code. Has this already been built/partially
   built? (This repo is a hard fork; phantom dispatches are real — past
   sessions found work already ~85% done.) Grep for the touch-points. If it's
   already substantially built, STOP and report that instead of rebuilding.
2. Identify the exact files/functions you'll touch. Read them. Never claim
   anything about code you haven't opened.
3. Find the pattern to mirror (e.g. navigate_app's event seam, the
   read_only_brain_conn pattern, the TourMode spline). Reuse > invent.
4. Report the Phase 0 findings before building. If the audit contradicts the
   premise of the task, STOP.

## Worktree (isolation is mandatory)
```
git worktree list                       # verify the name is free FIRST
git -C ~/dev/permagent-runtime fetch origin
git worktree add ~/dev/permagent-worktrees/<name> -b <branch> origin/main
export CARGO_TARGET_DIR=~/dev/permagent-worktrees/.shared-target/<name>   # per-lane target (#584)
```
Work ONLY in this worktree. Never the main checkout. After merge, reap with
`scripts/reap-worktrees.sh --apply --delete-remote` — it removes merged clean
lanes (worktree + branch + `.shared-target/<name>` tree, #581) and refuses
anything with uncommitted or unpushed work. Symlink node_modules from main per
the daemon-pkg pattern if needed.

**Per-lane CARGO_TARGET_DIR is a standing rule (#584, ruled 2026-07-03):** a
shared target tree lets one worktree run ANOTHER worktree's compiled test
binary and report false-green — cargo's fingerprint does not disambiguate two
lanes building the same crate version. Every cargo command in the lane runs
with the lane's own CARGO_TARGET_DIR set (state it in the dispatch log — the
convention is explicit and visible, not walked-up-and-discovered).
`scripts/build-guard.sh` enforces this: it refuses a build from a worktree
lane whose resolved target dir isn't namespaced by the lane (exit 12).

## Build
Implement against the Phase 0 plan. Constraints:
- Minimal, readable, production-grade. No scope creep — scope creep looks like
  progress and is the most expensive bug.
- Explicit config over hidden defaults. Clear abstractions, minimal magic.
- If this is a user-facing capability, ADD ITS SELF-KNOWLEDGE DESCRIPTOR in the
  SAME change (the standing rule): the <permagent_self> brief +
  WORKER/SURFACE descriptor. Gate the descriptor on the same flag as the
  feature if flag-gated. A capability Henry can DO but can't DESCRIBE is a bug.

## Gates (verification — show evidence, never assert success)
Run these and PASTE THE REAL OUTPUT. Do not say "tests pass" — show the command
and what it returned.
- **Crate name:** VERIFY the actual `[package]` name in the relevant Cargo.toml
  before `clippy -p` (the directory is goose-server/ but the cargo crate name
  differs — this has broken gates repeatedly). Do not assume.
- **Frontend:** `cd ui/command-center && npx tsc --noEmit && npx vite build`
- **Daemon/Rust:** `cargo check -p <verified-crate>` then
  `cargo clippy -p <verified-crate> -- -D warnings` then `cargo fmt --all`
- **Targeted test:** the unit test for the change (e.g. slice-math, lexicon
  lookup). Run it, show output.
- Do NOT run `--workspace --all-targets` locally (slow, CI does it). But DO run
  `cargo fmt --all` + `clippy --all-targets` as the LAST pre-push step — the
  test-target lint gap leaks to CI otherwise.

## Disk discipline (M1, 16GB)
- `df -h /System/Volumes/Data` is authoritative (NOT `df /`).
- Max two concurrent cargo builds. If another build is active, wait.
- Wrap cargo builds/tests in `scripts/build-guard.sh -- <cmd…>`: it enforces
  the free-space floor, serializes same-target-dir builds, and refuses a
  worktree-lane build on a non-lane target dir (#584).

## PR
- `Closes #NNN` for EVERY issue this addresses (multi-issue PRs must list each
  — this is the root cause of tracker drift).
- Push → CI runs the authoritative --workspace --all-targets.

## Report back
1. Phase 0 findings (what existed, what you'll touch, the pattern reused).
2. What you built + the self-knowledge descriptor (if applicable).
3. The gate output — REAL captured output, not paraphrased.
4. The acceptance check result.
5. Anything you had to STOP and flag.

## STOP-and-flag (the escalate-clause)
At ANY point, if you hit a judgment call — a design decision, an ambiguous
requirement, a product choice, a contradiction with the bible/CLAUDE.md, or
foreign WIP in the worktree — STOP and flag it for Jesse. Do not guess. The
human-in-the-loop is the architecture here, not a fallback.
