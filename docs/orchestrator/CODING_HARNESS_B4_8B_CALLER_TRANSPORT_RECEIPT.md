# B4.8B receipt — caller-owned/provider transport inventory proof

Date: 2026-09-04 (America/Halifax)

Status: passed. This bounded proof classifies transport/helper calls whose
physical accounting is already owned by their caller. It does not add a
second reservation or settlement layer, and it does not touch supervised
worker files or browser/UI work.

## Before and after

Before B4.8B, the rebuilt `dispatch-inventory.v2` audit reported 31 seams:
3 wrapped, 4 typed local/non-paid exclusions, and 24 unwrapped residuals.

Seven residual seams were proven caller-owned and now carry typed exclusions:

- `agents/reply_parts.rs:587` — `stream_split` transport owned by the Agent
  primary-turn reservation and terminal-usage settlement.
- `providers/base.rs:666,680` — default fast/full transport and fallback owned
  by `AccountedFastCompletion`, which reserves and settles each physical
  attempt.
- `council/debate.rs:267` — `ProviderAdapter::complete`, reached only through
  `LiveCaller`.
- `council/debate.rs:554` — Council live provider transport, after
  `LiveCaller` reserves and before it settles or marks unknown.
- `council/debate.rs:776,937` — `MemberCaller` dispatch used by round/chair
  orchestration; these are logical caller boundaries, not provider transport.

The after-inventory remains 31 seams with this exact classification:

| classification | count |
| --- | ---: |
| wrapped | 3 |
| explicitly excluded | 11 |
| unwrapped residual | 17 |
| total | 31 |

The 17 intentionally residual seams are now only:

- C1 model/provider calls (7): `doctor.rs:154`, `financier_close.rs:714`,
  `permission/permission_judge.rs:169`, `providers/anthropic.rs:310`,
  `providers/base.rs:858`, `providers/sovereign_guard.rs:207`, and
  `security/adversary_inspector.rs:295`.
- C2 supervised worker/provider CLI calls (10): `acp/provider.rs:889`,
  `agents/extension_manager.rs:330`,
  `agents/platform_extensions/developer/shell.rs:500`,
  `agents/platform_extensions/goal_engine.rs:906`, `forecaster/remote.rs:288`,
  `providers/claude_code.rs:383,654`, `providers/codex.rs:205`,
  `providers/cursor_agent.rs:219`, and `providers/gemini_cli.rs:140`.

## Evidence

The source contracts establish one accounting owner per physical call:

- Agent primary streaming reserves before `stream_response_from_provider`,
  settles one terminal usage snapshot, and marks post-dispatch failure or
  cancellation unknown.
- `AccountedFastCompletion::complete_one` reserves, invokes provider
  transport, attaches the invocation identity, and settles usage; the base
  trait's `complete_fast` is therefore a transport helper, not a second
  billing boundary.
- `LiveCaller::complete` reserves before `p.complete`, marks provider error,
  timeout, missing usage, and cancellation unknown, and settles successful
  usage. Council round/chair calls route through `MemberCaller` and do not
  dispatch providers themselves.

Markers use the typed exclusion contract with `reason` and `authority`, so a
future change must either preserve the caller-owned proof or become an
unwrapped inventory seam. No provider calls were made by this proof.

## Verification

```text
CARGO_INCREMENTAL=0 cargo test -p permagent-eval dispatch_inventory::tests -- --nocapture
9 passed; 0 failed

target/debug/permagent-eval dispatch-inventory --root crates/goose/src --json
31 seams: 3 wrapped, 11 explicitly excluded, 17 unwrapped

CARGO_INCREMENTAL=0 cargo test -p permagent --lib council
63 passed; 0 failed

git diff --check
passed
```

