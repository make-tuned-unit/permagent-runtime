# B4.8C receipt — supervised/external-worker transport proof

Date: 2026-09-04 (America/Halifax)

Status: passed for the bounded C2 inventory scope. No provider or CLI process
was launched by this work.

## Classification

The B4.8B snapshot contained 31 seams (3 wrapped, 11 typed exclusions, 17
unwrapped), including the ten C2 process sites in this scope. Each site was
classified before adding a marker:

- `acp/provider.rs:890` is ACP provider transport. The Agent/provider caller
  owns the reservation and terminal usage boundary; the ACP child is not a
  second billing attempt.
- `agents/extension_manager.rs:331` is an MCP extension server. Its existing
  timeout, stderr collection, and connection lifecycle supervise a tool
  server; it is not a model invocation.
- `agents/platform_extensions/developer/shell.rs:501` is the user shell tool.
  Its timeout, output collection, and cancellation lifecycle supervise a
  local tool command; it is not paid model work.
- `agents/platform_extensions/goal_engine.rs:907` is the genuine external
  goal worker. It already enters `GoalTask`/`ExternalCliEngine`, which refuses
  paid or unknown billing before worker-session/worktree/process creation and
  carries goal, budget-task, parent, provider, model, and CLI identity.
- `forecaster/remote.rs:289` is a bounded SSH forecast helper. Its configured
  enablement, deadline, stdin/stdout protocol, and child-drop timeout are the
  owning supervision contract; it is not a model provider dispatch.
- `providers/claude_code.rs:384`, `providers/codex.rs:206`,
  `providers/cursor_agent.rs:220`, and `providers/gemini_cli.rs:141` are
  provider transport launches. Their callers own reservation and settlement;
  adding a GoalTask or second reservation at the child process would
  double-account the same physical model request.
- `providers/claude_code.rs:656` is a short-lived model-listing metadata
  probe, explicitly local/non-paid and separate from completion transport.

Typed exclusions now carry stable seam IDs, a reason, and an authority. The
existing GoalTask billing gate remains the refusal boundary for generic
external workers: paid/unknown billing, or missing durable goal/budget
identity, fails before worktree/process launch. Existing worker identity
environment propagation preserves parent/goal lineage.

## Inventory result

The current shared worktree audit reports:

| classification | count |
| --- | ---: |
| wrapped | 3 |
| explicitly excluded | 23 |
| unwrapped | 0 |
| total | 26 |

The total is lower than the B4.8B snapshot because the owner-side C1 changes
landed concurrently; this receipt does not claim those unrelated edits.

## Verification

```text
CARGO_INCREMENTAL=0 cargo test -p permagent-eval dispatch_inventory::tests -- --nocapture
9 passed; 0 failed

target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json
26 seams: 3 wrapped, 23 explicitly excluded, 0 unwrapped

CARGO_INCREMENTAL=0 cargo test -p permagent --lib paid_external_cli_is_denied_before_worktree_or_process_effects
1 passed; 0 failed
CARGO_INCREMENTAL=0 cargo test -p permagent --lib unknown_external_cli_billing_is_denied_before_worktree_or_process_effects
1 passed; 0 failed
CARGO_INCREMENTAL=0 cargo test -p permagent --lib omitted_external_cli_billing_is_denied_before_worktree_or_process_effects
1 passed; 0 failed

CARGO_INCREMENTAL=0 cargo check -p permagent --lib
passed

git diff --check
passed
```

The broader `external_cli` filter also exercised eight passing tests but
retained two pre-existing non-runtime-safe tests: the ACP-adapter test
initializes the global SQLx session manager outside a Tokio context, and the
baseline test then observes its poisoned lazy singleton. The three billing
refusal fixtures above pass independently and prove no worktree/process is
created before refusal.

