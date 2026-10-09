# World activity lane (wq/world-activity), 2026-10-09

## Defect location
- `crates/goose-server/src/routes/henry_status.rs` (before this change): the `active` filter (`s.updated_at >= two_min_ago && User|Scheduled`) fed `classify_henry_state(has_tool, !active_sessions.is_empty())`, which returned `in_conversation` for any session touched in the last 120 s. A blank "New Chat" therefore made Henry "working".
- Same heuristic duplicated in `crates/goose-server/src/agent_state_tick.rs` `derive_henry_hud_state` (pre-first-turn fallback, `!active_ids.is_empty()`).
- UI consumer: `ui/command-center/src/components/world/agents/stateSources.tsx` `mapHenryState` (`in_conversation|tool_call` -> `working`) polls `/api/henry/status`; ForumView/AgentDeskProfile read it via `useAgentRuntimeStates()` (`shared/agentStatus.ts`). The UI mapping is correct; the wire value was wrong, so no UI change.

## Design
The daemon already has authoritative in-flight signals: the Drop-guarded reply-loop registry (`permagent::events::agent_runtime_state`, fed by `ReplyStateGuard`) and per-session event-bus `active_request_ids()`. New pure fn `derive_henry_state(runtime, has_in_flight_request)` uses only those two (request -> `tool_call`; registry Working -> `in_conversation`; registry Error with nothing in flight -> `error`; else `idle`), and session recency is no longer an input. Both the `/api/henry/status` route and the agent-state tick call it, so the poll and the pushed event agree. `active_sessions` stays in the response as informational data only.

Behaviour change to note: an in-flight request now outranks a latched error (matches the registry's own documented rule); previously the latch won on the route.

## Files
- crates/goose-server/src/routes/henry_status.rs (derive_henry_state replaces classify_henry_state; 5 tests)
- crates/goose-server/src/agent_state_tick.rs (uses derive_henry_state, no session-recency fallback)
- crates/goose/src/events/mod.rs (3 registry tests: never-ran, lifecycle, counter never negative)

## Gate
- `cargo test -p goose-server --lib -- henry_status`
- `cargo test -p goose --lib -- events::tests`
- `cargo check -p goose-server` (agent_state_tick has no test of its own)
- Vitest: none required (no TS touched).

## Compile uncertainties
- Crate package names: confirm `goose-server` / `goose` are the cargo -p names (lib is imported as `permagent`).
- `permagent::events::AgentRuntimeState` assumed publicly reachable (the enum is `pub` in `events/mod.rs`, which is `pub mod events`).
- Exhaustiveness of the 4-arm tuple match in `derive_henry_state`.
- `agent_errored` is now unused by goose-server; it stays `pub`, so no dead-code warning expected.
- Registry tests share a process-global map; they use unique agent ids.
