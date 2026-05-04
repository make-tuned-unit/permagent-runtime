# Phase 2.5: Tauri Surface Refactor

## Current State

Activity emission for browser, terminal, and project picker surfaces is
driven from frontend TypeScript code. The React components call
`invoke('emit_activity', ...)` to push events to the daemon via the Tauri
command bridge. The daemon receives events on `POST /activity/emit` and
forwards them to the shared event bus.

This works but has three limitations:

1. **Trust boundary**: The frontend can fabricate any event. The daemon
   validates structure and timestamp but cannot verify that the claimed
   action actually occurred.

2. **Auto-enrichment**: Frontend events arrive with whatever context the
   TypeScript code passes. Rust-owned surfaces could automatically enrich
   events with session context, project ID, user identity, and other
   daemon-side state without the frontend needing to know about it.

3. **Uniform policy**: Rate limiting, deduplication, and aggregation
   happen on the daemon side after events arrive. Rust-owned surfaces
   could apply policy before emission, reducing bus noise.

## Target State

Browser tab management, PTY lifecycle, and project selection move to
Tauri Rust commands. The frontend calls Rust commands for actions
("navigate to URL", "submit to terminal", "select project") and the
Rust side emits activity events as a side effect of performing the
action. The frontend never calls `emit_activity` directly for these
surfaces.

## Why Deferred

- **Scope**: Phase 2 focused on getting all surfaces onto the bus. Moving
  ownership requires rearchitecting each surface's lifecycle management.
- **Terminal latency**: The xterm.js PTY proxy sends keystrokes via IPC.
  Adding event emission on every keystroke in Rust would add latency to
  the terminal's input loop. The current "emit on Enter" approach in
  TypeScript is lower-latency.
- **Browser API ergonomics**: Tauri's webview API handles navigation
  events well (`browser_navigated`), but form submission detection
  requires injecting JavaScript shims into web pages. This is fragile
  and browser-dependent.

## Open Questions

- **PTY streaming model**: Should `spawn_pty_session` return a streaming
  handle that auto-emits TerminalCommandStarted/Completed? Or should
  the daemon detect command boundaries from the PTY output stream?
- **Browser tab ownership**: Currently tabs are managed in React state.
  Moving to Rust-owned tabs requires a tab management layer in the Tauri
  backend with proper lifecycle events.
- **Performance budget**: Keystroke-routed terminals produce ~5-15
  events/second during active typing. Activity emission must add <1ms
  per event to avoid user-perceptible latency.
