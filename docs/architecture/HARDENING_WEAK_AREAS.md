# Hardening the weakest areas — research, plan, validation

2026-08-10. Follows the calibrated weakness assessment: (1) no mid-flight
steering, (2) worker runtime discipline (a worker burned 3.85M tokens waiting
on a compile it was told to skip), (3) full DAG flow unproven live, (4) the
"built-but-never-wired" gap class.

## Research: who is strongest where we are weakest

- **OpenHands** — event-stream architecture: a user correction is just
  another event in an append-only log the agent reads next step; the UI can
  interrupt/redirect at any point because it writes the same stream.
  Validates our receipt/event direction; their SecurityAnalyzer-subscribes
  pattern parallels The Guard.
- **LangGraph** — interrupts are only safe because every step checkpoints;
  resume re-enters at the checkpoint with the human's input. Our equivalent
  checkpoint is the goal card (receipts, evidence, escalation handoff) — a
  steering seam must not invent a second state store.
- **Claude Code CLI itself** — `--input-format stream-json` is bidirectional:
  NDJSON user messages on stdin mid-run. And `--settings` accepts a
  PreToolUse command hook whose exit code 2 BLOCKS a tool call with a
  message the model sees. The two primitives we lacked already exist in the
  worker binary we spawn.

## Plan (ordered by leverage; each step carries its own validation)

1. **Worker policy hooks (discipline, deterministic).** Spawn claude workers
   with a generated settings file whose PreToolUse hook blocks the known
   failure classes in goal worktrees: heavy cargo invocations (the C
   failure: cold-worktree compiles that outlive the session) and git push
   (the #522 rule, currently enforced only by pushurl sentinel + prose).
   Blocking returns a message that TEACHES ("verify by reading; the central
   gate compiles"). Validation: unit tests on the generated settings + hook
   script behavior under sh; live goal after install.
2. **Steering seam v1 (external CLI workers).** Spawn with
   `--input-format stream-json` and stdin PIPED (today: Stdio::null()); keep
   the write half in the existing GOAL_WORKERS registry beside the kill
   handle; new orchestrator tool `steer_goal {card_id, message}` writes one
   NDJSON user message. Honest scope: claude workers only (codex exec has no
   stdin protocol; internal subagents need a different seam). Validation:
   protocol shape proven against a live non-privileged claude spawn; unit
   tests on encoding + registry; live steer of a dispatched goal after
   install.
3. **DAG flow proof.** Not code — the pending live run: two-goal roadmap
   where goal 2 depends on goal 1, approve → land → promote → dispatch →
   land, zero manual git. Recorded in docs/benchmarks/.
4. **Never-wired detector (gap class).** Longer-term: a CI check that every
   `remember_with` key constant has a reader (grep-level, advisory). Not in
   this pass; recorded so it is not forgotten.

Sources: OpenHands docs/PR#2709 (event stream), LangGraph interrupt docs,
Claude Code headless + hooks docs, claude-agent-sdk-go cli-protocol.md.
