# B4.8A receipt — dispatch inventory correctness

Date: 2026-09-04 (America/Halifax)

Status: passed for the B4.8A inventory-correctness scope. This receipt covers
the static scanner and typed local-subprocess exclusions only. It does not
claim completion of shared model accounting, supervised CLI accounting, or
the B4.8 primary-stream/direct/background work.

## Counts

The pre-B4.8A evaluator output was 42 seams: 3 wrapped and 39 unwrapped. That
output included test-only process calls and logical `.spawn(args)` helpers.
After rebuilding `permagent-eval` with `dispatch-inventory.v2`, the production
inventory is 31 seams:

| classification | count |
| --- | ---: |
| wrapped | 3 |
| explicitly excluded | 4 |
| unwrapped residual | 24 |
| total | 31 |

The four typed, deterministic/non-paid exclusions are:

- `agents/platform_extensions/developer/shell.rs:196` — login-shell path probe
- `agents/platform_extensions/developer/verify.rs:983` — verifier command
- `config/secret_source.rs:581` — credential backend CLI
- `providers/apple_fm/sidecar.rs:255` — on-device Apple sidecar

## Residual classes

The 24 unwrapped seams remain intentionally visible for later B4 work:

- C1, actual model/provider invocations requiring shared reservation and
  settlement: `council/debate.rs:266,552,773,933`; `doctor.rs:154`;
  `financier_close.rs:714`; `permission/permission_judge.rs:169`;
  `providers/anthropic.rs:310`; `providers/base.rs:856`;
  `providers/sovereign_guard.rs:207`; and
  `security/adversary_inspector.rs:295` (11 seams).
- C2, external workers or provider CLIs requiring supervised/GoalTask
  accounting: `acp/provider.rs:889`; `agents/extension_manager.rs:330`;
  `agents/platform_extensions/developer/shell.rs:500`;
  `agents/platform_extensions/goal_engine.rs:906`;
  `forecaster/remote.rs:288`; `providers/claude_code.rs:383,654`;
  `providers/codex.rs:205`; `providers/cursor_agent.rs:219`; and
  `providers/gemini_cli.rs:140` (10 seams).
- C4, caller-owned/helper seams that must not be wrapped at the provider base:
  `agents/reply_parts.rs:586` and `providers/base.rs:665,678` (3 seams).

## Scanner changes

The scanner now:

- tracks multiline `#[cfg(test)]` modules/functions and excludes only their
  bodies;
- distinguishes Rust lifetimes from character literals while masking source;
- recognizes zero-argument `.spawn()` as a process-launch candidate, leaving
  logical dispatch methods such as `engine.spawn(task)`, ACP
  `client.spawn(rx, init_tx)`, and thread-builder closures out of the process
  inventory; and
- keeps qualified provider sampling and unknown zero-argument process
  receivers visible.

## Verification

All commands were local and made no provider calls:

```text
CARGO_INCREMENTAL=0 cargo test -p permagent-eval dispatch_inventory::tests -- --nocapture
9 passed; 0 failed

CARGO_INCREMENTAL=0 cargo test -p permagent-eval -- --nocapture
159 library tests + 18 binary tests passed; 0 failed

target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json
31 seams: 3 wrapped, 4 explicitly excluded, 24 unwrapped

git diff --check
passed
```

