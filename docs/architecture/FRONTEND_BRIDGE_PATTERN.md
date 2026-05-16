# Frontend-as-Bridge Pattern: Daemon ↔ Tauri Communication

## The Constraint

The daemon (`permagentd`) is a sidecar child process spawned by the Tauri app. Communication is **one-way: Tauri → daemon via HTTP** on port 3001. The daemon has no `AppHandle`, no IPC channel, and no way to invoke Tauri commands or access webviews directly.

## The Pattern

When the daemon needs to access Tauri-side state (webview content, terminal state, native dialogs), the **frontend bridges the gap**:

```
Daemon                    Frontend (React)              Tauri (Rust)
  │                           │                            │
  ├─ emit event on ──────────>│                            │
  │  /events WebSocket        │                            │
  │  {request_id, type}       ├─ invoke Tauri command ────>│
  │                           │                            ├─ access native
  │                           │                            │  resource
  │                           │<── return result ──────────┤
  │<── POST /api/.../{id} ───┤                            │
  │    with result            │                            │
  ├─ fulfill oneshot ─────>   │                            │
  │  channel                  │                            │
```

## Concurrency

Each request gets a UUID-keyed `oneshot::Sender` in a `HashMap`. Parallel requests are fully independent. The daemon waits with a timeout (10s default); if the frontend doesn't respond, the pending entry is cleaned up.

## Auth

In-process MCP tools skip daemon token auth (they can't access it). If a tool moves out-of-process, add Bearer token auth. Mark these endpoints with `TODO(mesh)`.

## Reference Implementation

- **Daemon:** `crates/goose-server/src/routes/browser_content.rs` — `BrowserContentBridge`, read + fulfill endpoints
- **Frontend:** `ui/command-center/src/hooks/useBrowserContentBridge.ts` — WebSocket listener with auto-reconnect
- **Tauri:** `ui/desktop/src-tauri/src/browser.rs` — `get_page_content` using `eval_with_callback`

## Future Uses

Any daemon capability that needs Tauri-side state follows this shape:
- Terminal content extraction
- World View selection state
- Native file dialogs from agent context
- Agent-initiated window management
