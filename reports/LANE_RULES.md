# Lane rules (all workers) — 2026-10-07

- You work ONLY in your own worktree (given in your brief) on its branch `wq/<lane>`. Never touch any other path under /Users/j.
- Commit as you go with clear messages; end every commit message with the trailer line `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. If git refuses to commit from the sandbox, leave the edits uncommitted and say so in your report.
- Frontend checks you MAY run from `<worktree>/ui/command-center`: `npx tsc --noEmit`, `npx vitest run <path-or-filter>` (focused first; the full suite once at the end), `npx vite build`. node_modules is a symlink to the main checkout; do not npm install.
- Rust: you MAY run `cargo check -p <crate>` and `cargo test -p <crate> --lib -- <filter>` ONLY wrapped as
  `CARGO_TARGET_DIR=/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006 scripts/build-guard.sh -- cargo check -p permagent`
  (crate names: crates/goose = `permagent`, crates/goose-server = `permagent-daemon`, crates/goose-cli = `permagent-cli`). Never `--all-targets`, never clippy, never `cargo test -p permagent-daemon` (its test binary is SIGKILLed; use the lib filter form only). Use `export PATH="$PWD/bin:$PATH"` from the worktree root first (hermit toolchain). One cargo at a time machine-wide: the guard waits for the lock. Keep free disk above 25 GiB; if `df -h /` shows less, stop and report.
- No Blender, no Unreal, no native app builds, no releases, no git push, no changes to ~/.permagent or any live config.
- Never write the developer's first name anywhere, and never write the legacy agent names "Henry"/"Hank" in code (comments may say Henry as shorthand for the orchestrator role; tests may configure any persona name). `cargo test -p permagent --lib -- identity_name_guard` enforces this; emulate it for TS by grepping your diff.
- Every user-visible value must be user-agnostic and vendor-agnostic: config key with a neutral default, persona lookup, or env var. No personal preference of the developer may be a literal.
- Follow the existing design system: tokens in `ui/command-center/src/styles/tokens.ts` (space/radius/textSize/duration/ease), `Button` from `components/common/Button`, `Tooltip` instead of raw `title=`, theme colors from `useTheme()`. The convention scanners in the vitest suite flag raw px spacing, raw title attributes and hardcoded colors.
- Report at the end: files changed, commits, checks run with exit codes, what you could not verify, and anything you found but left out of scope.
