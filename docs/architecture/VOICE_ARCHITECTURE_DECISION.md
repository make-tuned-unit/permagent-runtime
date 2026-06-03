# Voice Architecture Decision Record

Epic: #198 — Voice and Screen-Grounded Companion
Phase: 0 (architecture spike and decision)
Date: 2026-06-03
Status: All five decisions resolved

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

**Question:** Default to local or cloud for STT and TTS?

**Decision: LOCAL-FIRST.** Cloud is an optional brokered fallback behind
the provider abstraction, off by default. No paid cloud provider
(ElevenLabs, AssemblyAI, OpenAI TTS) is on the Phase 1 path.

### Premise corrections from the Phase 0 draft

The original Phase 0 analysis rested on two premises that were wrong.
Recording the corrections here so they are not reintroduced later:

1. **"Local TTS quality is far behind" — INCORRECT.** Open TTS models now
   match or beat ElevenLabs in blind tests. Chatterbox-Turbo won a
   vendor-run blind test over ElevenLabs. Local TTS quality is not a reason
   to default to cloud. The Phase 0 draft's recommendation of "cloud TTS is
   almost certainly correct for Phase 1" was based on stale information.

2. **"Local STT requires whisper-rs and its bindgen/clang CI cost" —
   INCORRECT framing.** whisper-rs compiles whisper.cpp from source, which
   is the exact bindgen/clang surface that broke CI twice (`bd2e7e755`,
   `f33c4e988`). But whisper-rs is one path, not the path. The clean path
   is ONNX Runtime, which ships prebuilt binaries and has no C++ source-
   compile step. We route around the CI cost rather than accepting it.
   **Do not pull whisper-rs or any from-source C++ speech crate into the
   build graph.**

### Substrate-first principle (DURABLE DECISION)

> **We integrate against an ONNX-based substrate plus the provider
> abstraction, NOT against any single model. The specific TTS and STT
> models are hot-swappable config (a model file and a small adapter),
> never hard-coded into the architecture.**

**Rationale:** The durable, improving layer is the substrate. The field
improves fast; individual models, especially small single-maintainer ones,
may plateau. We buy the field's trajectory by integrating the substrate and
treating models as swappable. We expect to replace the TTS model within
roughly a year as the field moves, and that swap must be a config change,
not a refactor. A future session must not collapse the abstraction by
wiring one model in directly.

**What this means in practice:**
- The daemon exposes a `VoiceProvider` trait with `transcribe()` and
  `synthesize()` methods.
- Local providers load ONNX models from a configurable path.
- Cloud providers are an alternative implementation behind the same trait,
  off by default.
- Swapping a model means changing a config entry (model path + adapter),
  not touching Rust code.

### Substrate: Official sherpa-onnx crate (k2-fsa)

**Crate:** `sherpa-onnx` v1.13.2, published by k2-fsa (the sherpa-onnx
project itself). Apache-2.0 licensed. This is the OFFICIAL Rust API,
not the deprecated community `sherpa-rs` crate.

**Why the official crate, not sherpa-rs:** The community `sherpa-rs` crate
(thewh1teagle) was deprecated in March 2026. It uses build-time bindgen
against the C API header, which requires libclang — the same surface that
broke CI twice (bd2e7e755, f33c4e988). The official `sherpa-onnx-sys`
crate ships hand-written `extern "C"` FFI bindings with no bindgen, no
cmake, no cc, and no libclang dependency at build time. Its only build
dependencies are `bzip2`, `tar`, and `ureq` (for downloading prebuilt
libraries). This resolves the bindgen/libclang concern completely: **the
voice substrate no longer touches the libclang surface that broke CI.**

**Build behavior:** The build.rs downloads prebuilt sherpa-onnx libraries
from GitHub releases (versioned to match the crate version) and emits
link directives. No compilation of any kind occurs. Supports static
(default) and shared linking via feature flags.

**CI cost:** Download time for prebuilt libs (~50-100 MB one-time). Zero
compile time. Zero disk pressure from static archives or C++ toolchain.
Categorically different from both whisper-rs AND the community sherpa-rs.

### Validation status (2026-06-03)

**macOS Apple Silicon — validated on sherpa-rs 0.6.8 (Gate A), must be
re-validated on official sherpa-onnx 1.13.2:**
- Gate A proved Kokoro TTS (1.84s synth) and Moonshine STT (346ms
  transcribe) with character-perfect round-trip on the community crate.
- Must be re-run on the official crate's API surface to count.

**Ubuntu CI (x86_64-unknown-linux-gnu) — NOT YET PROVEN:**
- The official crate's build.rs has prebuilt archives for linux-x64.
- Must be validated in real CI before sherpa-onnx is added to the
  workspace.

### Open pre-ship decisions

**Static vs shared linking:** The official crate defaults to static
linking. Static linking bundles espeak-ng (GPLv3) into the binary, which
is the most fraught form of that dependency for a closed commercial
product. Shared linking avoids the static-GPL entanglement but requires
shipping dylibs in the app bundle. Current lean: shared, pending
investigation of whether a non-GPL G2P path exists for Kokoro that would
make the question moot.

**espeak-ng GPLv3 G2P dependency:** Kokoro uses espeak-ng-data for
phonemization (confirmed in Gate A). The static-vs-shared decision and
the existence of any non-GPL G2P alternative are under investigation.

### Default model picks (explicitly swappable)

These are starting defaults, not commitments. The substrate-first principle
means they are replaced by changing config, not code.

**STT default: Moonshine**
- License: MIT
- Streaming capable, small footprint, clean license
- Good fit for push-to-talk latency requirements
- Reachable through sherpa-onnx

**STT alternative (max accuracy): NVIDIA Parakeet**
- Only if license clears (see verification items below)
- Also reachable through sherpa-onnx

**TTS default: Kokoro-82M**
- License: Apache 2.0 (commercial OK)
- Real-time on Apple Silicon, streaming capable
- Treat as a default we expect to replace, not a commitment

**TTS alternative (premium slot): Chatterbox**
- License: MIT
- Won blind test over ElevenLabs
- Slot in behind the same abstraction when ready

### License verification items (MUST verify before Phase 1 implementation)

1. **NVIDIA Parakeet / Nemotron license.** These models are under NVIDIA's
   Community / Open Model License, not MIT/Apache. Confirm commercial
   redistribution rights before treating Parakeet as usable for STT. If it
   does not clearly clear, default to Moonshine.

2. **Kokoro G2P / espeak-ng license (MUST resolve before release).**
   Gate A confirmed that the Kokoro TTS model uses espeak-ng-data for
   phonemization — the smoke test loaded `espeak-ng-data/` as the
   `data_dir` parameter. espeak-ng is GPLv3, which is a concern for
   linking or bundling with a closed commercial binary. This does not
   block the substrate decision or Phase 1 development, but MUST be
   resolved before any release that ships Kokoro as the default TTS.
   Options: (a) confirm sherpa-onnx's bundled espeak-ng-data is data-only
   and not a linked library (data may not trigger GPL copyleft),
   (b) replace with a non-GPL G2P path, (c) switch the default TTS model
   to one that does not require espeak-ng.

### Cloud as optional fallback

Cloud STT and TTS providers (OpenAI, AssemblyAI, ElevenLabs, etc.) are
implemented as alternative `VoiceProvider` trait implementations, behind
the same abstraction. They are off by default and available as user-
configured opt-in through the provider routing interface. No paid provider
API key is required for the default experience.

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
| STT (local, default) | 200ms | Moonshine via sherpa-onnx on M-series |
| STT (cloud fallback) | 500ms | Optional, adds network round-trip |
| Intent routing | 50ms | Heuristic pre-filter |
| recall() | 300ms | Single injection, cached |
| LLM inference | 1000-2000ms | First token; streaming after |
| TTS (local, default) | 200ms | Kokoro-82M via sherpa-onnx, streaming |
| TTS (cloud fallback) | 300ms | Optional, adds network round-trip |
| **Total (local)** | **1.8-2.8s** | Local-first is faster than cloud |

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
| 2 | Local vs cloud STT/TTS | Local-first via ONNX substrate; cloud as opt-in fallback | **Decided** |
| — | Substrate-first principle | ONNX substrate + provider abstraction; models are swappable config | **Decided (durable)** |
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
