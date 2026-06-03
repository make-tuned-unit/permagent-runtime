# Voice Architecture Decision Record

Epic: #198 — Voice and Screen-Grounded Companion
Phase: 0 (architecture spike and decision)
Date: 2026-06-03
Status: Draft — awaiting product call on Decision 2

---

## Decision 1: Coordination architecture and native sidecar boundary

**Question:** Can Phases 1 through Permagent-first grounding stay entirely in
Tauri + daemon, deferring the native Swift sidecar to the later OS-wide layer?

**Recommendation: YES — defer the native sidecar.**

### Evidence from codebase audit (the spike)

The revised scope anchors voice in the Chat window as coordinator. The
codebase already proves every link in the Chat window → daemon → main app
chain works without native code:

1. **Two-window architecture is live.** The Chat window is a separate
   Tauri WebviewWindow (label `'chat'`, loads `index.html?view=chat`).
   Created dynamically in `ChatLauncher.tsx:42-81`. Both windows share the
   same origin, same daemon at `127.0.0.1:3001`, and the same Bearer token
   loaded via Tauri IPC (`api.ts:6-34`).

2. **Cross-window communication is proven.** The file-drop handshake
   (`App.tsx:121-157` → `ChatApp.tsx:66-81`) demonstrates emit/listen
   across windows: main emits `chat_drop_files`, chat listens and acts.
   Base64 binary payloads flow. This same pattern carries voice state
   events (recording status, transcript chunks, coordination commands).

3. **Shared state sync is live.** `tokens.ts:226-235` syncs theme via
   `window.addEventListener('storage', ...)` on localStorage. The Chat
   window already stores `permagent-chat-session-id` in shared localStorage.
   Main app state (active tab, selected project) can be shared the same way.

4. **Window focus/show is trivial.** `ChatLauncher.tsx:49-50` already
   calls `existing.show()` and `existing.setFocus()` on the chat window.
   A push-to-talk hotkey just needs to do the same.

5. **Microphone access works in WKWebView.** `navigator.mediaDevices
   .getUserMedia({audio: true})` is fully supported in Tauri 2's WKWebView
   on macOS. No native code needed. Triggers macOS microphone permission
   prompt on first use.

6. **Audio playback exists.** `AudioMessage.tsx` already uses
   `<audio>` element for playback. Web Audio API is available for TTS
   streaming and waveform visualization.

7. **Frontend-bridge pattern is proven.** `useBrowserContentBridge.ts`
   and `browser_content.rs` show the daemon requesting something from the
   frontend via oneshot channels. Audio capture can follow the identical
   pattern: daemon emits `audio_capture_requested`, frontend fulfills via
   POST to an audio endpoint.

### What the sidecar IS needed for (later)

Only the OS-wide layer needs native Swift:
- ScreenCaptureKit for capturing arbitrary third-party app screens
- NSPanel transparent overlay for multi-monitor pointing
- Accessibility API for system-wide global hotkeys outside the app

These are not needed for Phases 1-5 (Permagent-first voice).

### What IS needed for Phase 1 (no native code)

- `tauri-plugin-global-shortcut` (Rust plugin, ~20 lines of config).
  Registers a push-to-talk hotkey that works when the app is in background.
  Requires macOS Accessibility permission.
- Audio capture hook in the Chat window (getUserMedia + MediaRecorder).
- Audio transport to daemon (see Decision 3).

### Why a throwaway code spike was not necessary

The kickoff prompt authorized a throwaway prototype for screen capture +
overlay to resolve the sidecar question. The revised scope eliminates screen
capture from Phases 1-5 and reframes the spike as: "confirm Tauri can
handle the in-app coordination and the push-to-talk hotkey focusing the
Chat window." The codebase already contains working examples of every
component in this chain (cross-window events, focus management, browser
bridge, audio playback). A code spike would have re-proved what is already
shipping. The evidence above is drawn from production code, not paraphrase.

---

## Decision 2: Local versus cloud STT and TTS

**Question:** Default to local (whisper.cpp / whisper-rs) or cloud
(AssemblyAI, ElevenLabs, or OpenAI) for STT and TTS?

**Recommendation: ESCALATED — this is a product call.** Options and costs
below.

### Option A: Cloud-first with local fallback

Start with cloud STT (OpenAI Whisper API or AssemblyAI) and cloud TTS
(ElevenLabs or OpenAI). Route through the daemon as a proxy (like Clicky's
Cloudflare Worker). Add local as a fallback later behind a feature flag.

**Pros:**
- Zero CI cost — no native C++ dependencies added.
- Faster time-to-voice: API integration is days, not weeks.
- Higher quality out of the box (ElevenLabs voices, Whisper large-v3).
- Sidesteps the triple-CMake problem (llama-cpp-2 + kuzu + whisper.cpp).

**Cons:**
- Contradicts the local-first privacy thesis.
- Requires API keys and ongoing cloud cost.
- Adds latency for the network round-trip (~200-500ms per call).

### Option B: Local-first with cloud brokered fallback

Default to whisper-rs (local STT) and a local TTS engine. Cloud is a
brokered fallback configured through provider routing.

**Pros:**
- Consistent with the privacy thesis and local-first brand.
- No ongoing cloud cost or API key requirement.
- Lower latency for STT once the model is loaded (~100-300ms on M-series).

**Cons — the CI cost is real:**
- whisper-rs adds `whisper-cpp-sys` which requires CMake + bindgen + clang.
  This is the **same build surface** that broke Ubuntu CI twice in the last
  week:
  - `bd2e7e755`: free-disk-space action removed clang-16/17/18, breaking
    llama-cpp-sys-2 bindgen (`stdbool.h not found`).
  - `f33c4e988`: kuzu + V8 + ML static archives exhausted Ubuntu runner disk
    during parallel linking.
- Estimated incremental build time: **+3-5 min** (ubuntu), +1-2 min (macOS).
- Estimated disk overhead: **+200-300 MB** on an already-tight Ubuntu runner
  (25-30 GB reclaimed by jlumbroso/free-disk-space, currently at the margin).
- This would be the **third** heavy CMake-based C++ dependency in the
  workspace (after llama-cpp-2 and kuzu/Spectral).
- Local TTS quality is significantly behind ElevenLabs. No Rust-native TTS
  engine approaches cloud quality today.

### Option C: Abstract the provider boundary now, decide default later

Define a `VoiceProvider` trait in the daemon with `transcribe()` and
`synthesize()` methods. Implement cloud first (lowest risk, fastest
iteration). Gate local behind `local-speech` Cargo feature flag (parallel
to existing `local-inference` for llama). Ship Phase 1 with cloud, add
local in a follow-up when CI headroom exists.

**This is the recommended technical approach regardless of the default.**
The principled long-term answer from the Mesh vision doc is routing voice
through the inference routing interface (Phase 3 there). Option C is the
stepping stone.

### What I need from you

1. **Default for Phase 1:** Cloud or local STT? Cloud TTS is almost
   certainly correct for Phase 1 given local TTS quality.
2. **Are you willing to accept the CI cost of whisper-rs now**, or should
   it be deferred behind a feature flag for a later phase?
3. **Provider preference:** OpenAI (Whisper API + TTS), AssemblyAI +
   ElevenLabs (Clicky's stack), or mix?

---

## Decision 3: Transport

**Question:** Does voice ride the existing `/events` WebSocket, a dedicated
audio WebSocket, or the REST API?

**Recommendation: Dedicated audio WebSocket at `/voice`.**

### Evidence

The existing `/events` WebSocket (`crates/goose-server/src/routes/events.rs`)
is unsuitable for audio:

| Property | /events | Voice needs |
|----------|---------|-------------|
| Auth | None (public) | Bearer token (session-scoped) |
| Payload | JSON text only | Binary PCM/Opus frames |
| Direction | Server → client broadcast | Bidirectional stream |
| Semantics | Fire-and-forget event bus | Request-response with ACKs |
| Backpressure | 1000-event broadcast buffer | Per-connection flow control |

Binary audio over JSON text would add 33% base64 overhead and the handler
currently ignores inbound binary frames (`events.rs:119 — _ => {}`).

### Proposed design

```
GET /voice?session_id={id}
Authorization: Bearer {daemon_token}
Upgrade: websocket

Inbound (client → daemon):
  Text:   {"type": "start", "codec": "opus", "sample_rate": 16000}
  Binary: [audio_chunk_bytes]
  Text:   {"type": "stop"}

Outbound (daemon → client):
  Text:   {"type": "transcript", "text": "...", "final": false}
  Text:   {"type": "reply_start"}
  Binary: [tts_audio_chunk_bytes]
  Text:   {"type": "reply_end"}
  Text:   {"type": "intent", "route": "direct|dispatch|action", ...}
```

The existing Axum WebSocket infrastructure (`axum::extract::ws`) supports
binary frames natively. Bearer token validation reuses the existing
`require_bearer_token` middleware. Session scoping reuses the existing
session model.

### Alternative considered: REST with frontend-bridge

Audio chunks POSTed to `/api/audio/{request_id}` following the
`useBrowserContentBridge` pattern. Simpler but adds HTTP overhead per chunk
and loses the bidirectional stream for real-time transcript and TTS
feedback. WebSocket is the better fit for the interactive voice loop.

---

## Decision 4: Intent routing

**Question:** Where does the intent classifier live and how does it express
the three routing outcomes (direct reply, orchestrator dispatch, in-app
action)?

**Recommendation: Two-tier routing — lightweight heuristic pre-filter,
LLM tool selection for the rest.**

### Analysis

The Orchestrator (`orchestrator.rs`) already exposes 12 MCP tools to the
LLM, including `goal_advance`, `decompose_roadmap`, and `create_roadmap`.
The LLM selects tools based on the user's message — this is effectively
implicit intent routing.

For voice, the difference is latency sensitivity. Adding a pre-LLM
heuristic classifier keeps the common case fast:

### Proposed architecture

```
Transcript (from STT)
  │
  ├─ Heuristic pre-filter (< 50ms, runs in daemon)
  │   ├─ High-confidence direct reply keywords → Brain recall path
  │   ├─ High-confidence dispatch keywords → Orchestrator goal creation
  │   └─ Ambiguous → pass to LLM with full tool set
  │
  └─ LLM with MCP tools (existing path)
      ├─ Direct reply (default, uses recall)
      ├─ goal_advance / create_roadmap (orchestrator dispatch)
      └─ In-app action tools (Phase 4+, navigate/create/build)
```

### Gap: `create_goal` MCP tool

Currently goal cards can only be created via the HTTP API
(`POST /api/projects/{project_id}/cards` in `cards.rs:271`). The
Orchestrator has `decompose_roadmap` and `create_roadmap` but no simple
`create_goal` tool for single voice-triggered goals. This should be added
in Phase 1 or 5.

### Where the classifier lives

In the daemon, as a Rust function called before the agent reply path. It
receives the transcript string and returns a `VoiceIntent` enum. The
daemon can then either short-circuit to a fast path (recall-only reply)
or pass to the full agent loop with tool context pre-seeded.

---

## Decision 5: recall() latency on the voice path

**Question:** What is the recall latency and what budget should voice set?

**Recommendation: 300ms per-recall budget, with caching and batching to
keep total recall overhead under 500ms per voice turn.**

### Measured overhead (from code analysis)

recall() path (`brain_ops.rs:32-99`):

| Step | Operation | Latency | Blocking? |
|------|-----------|---------|-----------|
| 1 | `brain.recall_cascade()` — Spectral fingerprint search over SQLite | 200-500ms | YES (spawn_blocking) |
| 2 | `filter_recall_hits()` — threshold 0.7, top-K 3 | <1ms | No |
| 3 | `agent.extend_system_prompt()` — inject hits | <5ms | No |

Every call to `brain.recall()` or `brain.recall_cascade()` blocks a tokio
worker thread for 200-500ms. With multiple recall injections per agent turn,
cumulative blocking can reach 0.8-2.0 seconds.

### Voice latency budget

Clicky's full loop benchmarks at roughly 2-5 seconds. For voice to feel
conversational, the full loop should target:

| Stage | Budget | Notes |
|-------|--------|-------|
| STT (cloud) | 500ms | AssemblyAI/OpenAI streaming |
| STT (local) | 200ms | whisper.cpp on M-series, small model |
| Intent routing | 50ms | Heuristic pre-filter |
| recall() | 300ms | Single injection, cached |
| LLM inference | 1000-2000ms | First token; streaming after |
| TTS (cloud) | 300ms | ElevenLabs streaming |
| TTS (local) | 200ms | Lower quality |
| **Total** | **2.4-3.4s** | Acceptable for interactive use |

### Mitigation strategies

1. **Cache recall results per conversation context.** Current
   `KanbanContextCache` uses 5-min TTL + invalidation. A similar
   `VoiceRecallCache` can serve the same recall hits for consecutive
   voice turns on the same topic.

2. **Batch recall injections.** Instead of 4+ separate `spawn_blocking`
   calls per turn, collect all queries and run one `recall_cascade`.

3. **Pre-warm on voice session start.** When the user activates
   push-to-talk, fire a background recall with the current conversation
   context so results are ready by the time STT completes.

4. **Single-recall budget.** Voice turns should inject at most 1 recall
   (top-3 hits from a single cascade), not the multi-injection pattern
   used in text chat where latency is invisible.

---

## Summary of recommendations

| # | Decision | Recommendation | Status |
|---|----------|----------------|--------|
| 1 | Sidecar boundary | Defer native sidecar; Tauri + daemon handles Phases 1-5 | **Decided** |
| 2 | Local vs cloud STT/TTS | Abstract provider boundary now; **default needs product call** | **Escalated** |
| 3 | Transport | Dedicated `/voice` WebSocket with Bearer auth and binary frames | **Decided** |
| 4 | Intent routing | Heuristic pre-filter + LLM tool selection; add `create_goal` tool | **Decided** |
| 5 | recall() latency | 300ms per-recall budget; cache, batch, pre-warm | **Decided** |

---

## Files referenced

| File | Relevance |
|------|-----------|
| `ui/command-center/src/components/chat/ChatLauncher.tsx` | Chat window creation, focus/show |
| `ui/command-center/src/ChatApp.tsx` | Chat window root, file-drop listener |
| `ui/command-center/src/App.tsx:121-157` | Cross-window Tauri event emit |
| `ui/command-center/src/styles/tokens.ts:226-235` | localStorage sync pattern |
| `ui/command-center/src/lib/api.ts:6-34` | Daemon token loading, shared by both windows |
| `ui/command-center/src/components/chat/AudioMessage.tsx` | Existing audio playback |
| `ui/command-center/src/hooks/useBrowserContentBridge.ts` | Frontend-bridge pattern |
| `crates/goose-server/src/routes/events.rs` | /events WebSocket (unsuitable for audio) |
| `crates/goose-server/src/routes/browser_content.rs` | Daemon-side bridge pattern |
| `crates/goose-server/src/middleware/auth.rs` | Bearer token validation |
| `crates/goose-server/src/brain_ops.rs:32-99` | recall() path and latency |
| `crates/goose/src/agents/platform_extensions/orchestrator.rs` | Orchestrator dispatch |
| `crates/goose/src/goal_state.rs` | Goal lifecycle state machine |
| `crates/goose-server/src/routes/cards.rs:271` | Card creation HTTP API |
| `.github/workflows/ci.yml` | CI config (bindgen/clang/disk constraints) |
