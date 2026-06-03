# Epic #198: Voice and Screen-Grounded Companion

Status: scoping. Net-new epic, starting now.
Platform: macOS Apple Silicon (consistent with product).
Reference implementation: Clicky (https://github.com/farzaa/clicky), MIT, native Swift. Study as design, not as code to port.

## Goal

A push-to-talk voice loop with the Orchestrator, anchored in the Chat window. The first usable milestone is "talk to Henry" in the Chat window with no screen involvement. The flagship milestone is voice plus the Chat window acting as a cockpit that coordinates the rest of the app by voice.

## Product principle: the Chat window is the coordinator

The chat interface lives in the Chat window, and that window is the primary surface for this epic. It is not a Clicky-style ambient menu-bar buddy floating over the OS. The end-state vision: the user keeps the Chat window open as mission control, works through it, and trusts the Orchestrator (Henry) to coordinate the main Permagent app, which may be minimized. The differentiator over Clicky is therefore "conversational cockpit for an agent OS," not "ambient buddy over the desktop."

Scope boundary on "coordinate the main app": near term this means Permagent's own surfaces (World View, Build, Kanban, and so on). Henry coordinates Permagent itself through the Chat window and daemon. OS-wide screen grounding and pointing at arbitrary third-party apps (the full Clicky capability) is a separate, later, optional layer, and only that layer needs the native screen-capture and overlay work. Everything before it stays in-app.

## Why now

Voice has been on the roadmap as the five-layer epic (STT, TTS, intent routing, OS control, Mobius/waveform aesthetic). Clicky just shipped an open-source, macOS-native implementation of nearly that exact stack, which removes most of the unknowns and de-risks the build. It is also a demo that visibly separates us from the Odysseus-style chat-workspace crowd.

## Non-goals (at least early)

- Full desktop automation that clicks or types on the user's behalf without an approval gate. Coordinating Permagent's own app is the safe first step; OS-wide control is the later optional layer.
- Always-listening wake word. Start with push-to-talk.
- Non-macOS platforms.
- Replacing the existing text chat surface. Voice is additive to the Chat window.

## The loop we are copying (Clicky, for reference)

Global hotkey held, audio streams over a websocket to STT (AssemblyAI in Clicky), transcript plus a screenshot go to the model over streaming SSE, reply plays through TTS (ElevenLabs in Clicky), and the model can embed `[POINT:x,y:label:screenN]` tags that move a transparent cursor overlay to specific UI across multiple monitors. Keys live in a Cloudflare Worker proxy. Relevant Clicky files: CompanionManager.swift (central state machine), AssemblyAIStreamingTranscriptionProvider.swift, ElevenLabsTTSClient.swift, OverlayWindow.swift (the pointing overlay), worker/src/index.ts (the proxy), CLAUDE.md (full architecture).

## Architecture decisions to settle in Phase 0

These are the load-bearing choices. Nothing downstream should start until they are made.

1. Coordination architecture and the native sidecar boundary. The core coordination path is Chat window to daemon to main app: the Chat window sends intents to permagentd, which drives the Orchestrator and the main app's state. Both the Chat window and the main app are Tauri windows of the same app, with the daemon as the hub, so the main app can be minimized while the Chat window stays open. This path needs no native code. Clicky's native pieces (ScreenCaptureKit, an NSPanel transparent overlay, accessibility-based global hotkeys, all Swift) are required only for the later OS-wide layer that grounds in and points at arbitrary third-party apps. Decision to confirm in Phase 0: keep Phases 1 through the Permagent-first grounding work entirely in Tauri plus daemon, and defer the native Swift sidecar to the OS-wide layer where it is actually needed. Spike only enough to confirm Tauri can handle the in-app coordination and the push-to-talk hotkey focusing the Chat window.

2. Local versus cloud STT and TTS. Clicky uses cloud (AssemblyAI, ElevenLabs) through its Worker proxy. Our local-first thesis argues for local STT (whisper.cpp or whisper-rs) and local TTS where viable, with cloud as a brokered fallback. Caution: a whisper sys crate shares the same bindgen and clang build surface that just broke ubuntu CI on llama-cpp-sys. Factor that cost in. The principled long-term answer is to route voice providers through the inference routing interface described in the Mesh vision doc (Phase 3 there), so local versus cloud becomes a routing policy, not a hardcoded choice.

3. Transport. permagentd already exposes a WebSocket at GET /events (no auth on localhost) and REST at /api/* behind a Bearer token from ~/.permagent/secrets/daemon_token.json. Audio streaming wants a websocket. Decide whether voice rides /events or a dedicated audio socket, and how the native sidecar authenticates.

4. Intent routing. This is where we differ from Clicky, which is single-tier. Transcript goes to an intent step that routes to one of: a direct chat reply (the existing Brain recall path), an Orchestrator dispatch (goal creation via project_manager.card_create with auto_dispatch), or an OS action (pointing or control). Decide where the classifier lives and how it expresses these three outcomes.

5. recall() latency on the voice path. recall() sits on the synchronous chat reply path and Brain calls block_on internally, so every call site must use spawn_blocking. Voice makes latency visible in a way text does not. Measure recall latency under the voice loop and set a budget early.

## Phased plan

### Phase 0: Architecture spike and decision (read-only, no user-facing change)
Resolve the five decisions above. Spike the native sidecar versus Tauri-native question with a throwaway prototype of screen capture and one overlay marker. Produce a short decision doc (docs/architecture/VOICE_ARCHITECTURE_DECISION.md) before any feature code. Audit-before-build: this phase writes no production code.

### Phase 1: Push-to-talk to the Orchestrator, text-grounded only
The first usable milestone. Global hotkey, audio capture, STT, transcript into the existing chat reply path, TTS playback of the reply. Minimal UI. No screen capture, no pointing. Outcome: the user can hold a key, speak to Henry, and hear a spoken reply grounded in Brain recall.

### Phase 2: Voice UI and aesthetic
Waveform or Mobius visual feedback. Wire the agent HUD color semantics (gray idle, amber working, cyan available, red error). Handle barge-in (user interrupts TTS by speaking). Decide whether the waveform lives in the HUD or as an overlay, and how it relates to the World View 3D aesthetic (Three.js, cyber-classical).

### Phase 3: Grounding in the main app (Permagent-first)
The Chat window becomes aware of the main app's state so Henry can ground answers and coordination in what the app is currently showing (active tab, selected project, Kanban state, and so on). This is in-app state shared over the daemon, not screen capture, so it needs no native sidecar and no Screen Recording permission. Outcome: the user can ask about and act on what the main app is doing while it is minimized. OS-wide screen grounding of arbitrary third-party apps is deferred to the later optional layer.

### Phase 4: Coordinating and pointing within the main app
Henry navigates and drives Permagent's own UI by voice: switch tabs, open a project, create a card, kick off a build, and highlight where it acted so the user can follow. Because this is the app coordinating itself, highlighting is done in-app (React surfaces highlighting their own elements), not via an OS overlay, so it still needs no native sidecar. This is the flagship demo: keep the Chat window open, main app minimized, and steer the whole app by talking to Henry.

### Phase 5: Full intent routing within Permagent
Activate the full transcript-to-intent-to-action path, scoped to the app: direct reply (Brain recall), Orchestrator dispatch (goal creation via card_create with auto_dispatch), or an in-app action (navigate, create, build). Any side-effectful action goes through an approval gate, consistent with the Orchestrator's always-required Review gate. This is where the Chat window becomes a true control surface for Permagent, not just a conversation. Control of anything outside Permagent depends on the optional layer below.

### Later optional layer: OS-wide grounding and pointing
This is the full Clicky capability: capture arbitrary screens, ground in third-party apps, and fly a transparent multi-monitor overlay to on-screen UI via the coordinate-tag pattern (`[POINT:x,y:label:screenN]`). This is the only part that requires the native Swift sidecar (ScreenCaptureKit plus overlay) and the Screen Recording and Screen Content permissions. Treated as a separate, later, optional epic rather than core to #198.

## Open questions

- How the Chat window and the minimized main app communicate. Both are Tauri windows of the same app with the daemon as hub: confirm the shared-state mechanism and how the Chat window observes and drives the main app's state.
- What "coordinate the main app" means concretely as actions: navigate tabs, open a project, create a card, run a build, and how Henry confirms or highlights what it did.
- Default for local versus cloud STT and TTS, and how it routes through the brokerage. Privacy thesis points to local default with brokered fallback.
- Push-to-talk versus wake word for v1. Recommend push-to-talk first, focusing the Chat window.
- Where the intent classifier lives (daemon, a worker model, the Orchestrator) and how it expresses the routing outcomes.
- Latency budget. Clicky's full loop is the benchmark; the Mesh doc cites roughly 2 to 5 seconds for interactive use. recall() adds to this. Measure before committing to a target.
- Barge-in behavior. Can the user interrupt TTS midstream by speaking. Study Clicky's state machine.
- Native sidecar boundary and auth, deferred to the later OS-wide layer: how the Swift helper authenticates to permagentd. Bearer token, or localhost-only trust like /events.
- Permissions UX for the early phases. Mic and Accessibility are the only prompts before the OS-wide layer. Sequence and explain them so first-run is not a wall of scary dialogs. Screen Recording is introduced only with the later optional layer.

## Permissions required (macOS)

- Microphone: push-to-talk capture. Needed from Phase 1.
- Accessibility: global push-to-talk hotkey. Needed from Phase 1.
- Screen Recording and Screen Content (ScreenCaptureKit): the later OS-wide grounding and pointing layer only. Not needed for Phases 1 through the Permagent-first work.

## Suggested first step for CC

Phase 0 only, framed as a read-only audit and spike with an explicit stop before any production code: confirm the Chat-window-to-daemon-to-main-app coordination path works in Tauri (both windows, daemon as hub, push-to-talk hotkey focusing the Chat window) and that the native Swift sidecar can be deferred to the later OS-wide layer; recommend local-versus-cloud STT and TTS with the whisper or llama-cpp-sys CI cost called out; propose the transport over the daemon; and write VOICE_ARCHITECTURE_DECISION.md. Stop and report before Phase 1.
