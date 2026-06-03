# Epic #198: Voice and Screen-Grounded Companion

Status: scoping. Net-new epic, starting now.
Platform: macOS Apple Silicon (consistent with product).
Reference implementation: Clicky (https://github.com/farzaa/clicky), MIT, native Swift. Study as design, not as code to port.

## Goal

A push-to-talk voice loop with the Orchestrator that can optionally see what is on screen and point at on-screen UI. The first usable milestone is "talk to Henry" with no screen involvement. The flagship milestone is voice plus on-screen pointing as a low-risk first form of OS control.

## Why now

Voice has been on the roadmap as the five-layer epic (STT, TTS, intent routing, OS control, Mobius/waveform aesthetic). Clicky just shipped an open-source, macOS-native implementation of nearly that exact stack, which removes most of the unknowns and de-risks the build. It is also a demo that visibly separates us from the Odysseus-style chat-workspace crowd.

## Non-goals (at least early)

- Full desktop automation that clicks or types on the user's behalf without an approval gate. Pointing is the safe first step.
- Always-listening wake word. Start with push-to-talk.
- Non-macOS platforms.
- Replacing the existing text chat surface. Voice is additive.

## The loop we are copying (Clicky, for reference)

Global hotkey held, audio streams over a websocket to STT (AssemblyAI in Clicky), transcript plus a screenshot go to the model over streaming SSE, reply plays through TTS (ElevenLabs in Clicky), and the model can embed `[POINT:x,y:label:screenN]` tags that move a transparent cursor overlay to specific UI across multiple monitors. Keys live in a Cloudflare Worker proxy. Relevant Clicky files: CompanionManager.swift (central state machine), AssemblyAIStreamingTranscriptionProvider.swift, ElevenLabsTTSClient.swift, OverlayWindow.swift (the pointing overlay), worker/src/index.ts (the proxy), CLAUDE.md (full architecture).

## Architecture decisions to settle in Phase 0

These are the load-bearing choices. Nothing downstream should start until they are made.

1. Native sidecar boundary. Clicky uses ScreenCaptureKit, an NSPanel transparent overlay, and accessibility-based global hotkeys, all native Swift. We are Tauri 2 plus React plus the Rust daemon (permagentd). Tauri can handle global shortcuts and a transparent always-on-top click-through window, but reliable multi-monitor screen capture and a precise pointing overlay are more natural in a small native Swift helper. Likely outcome: a native sidecar process that talks to permagentd, rather than forcing everything through Tauri. Spike both and decide.

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

### Phase 3: Screen grounding
On demand, capture a screenshot and send it as additional context to the model. Requires the Screen Recording permission. No pointing yet. Outcome: the user can ask about what is on screen and get a grounded answer.

### Phase 4: On-screen pointing
Implement the coordinate-tag pattern (Clicky's `[POINT:x,y:label:screenN]`). Transparent, click-through, multi-monitor overlay that moves to the indicated UI element. This is OS-control-lite and the flagship demo. Most of the native sidecar work from Phase 0 pays off here.

### Phase 5: Intent routing and OS control proper
Activate the full transcript-to-intent-to-action path: direct reply, Orchestrator dispatch, or OS action. Any side-effectful action goes through an approval gate, consistent with the Orchestrator's always-required Review gate. This is the phase where voice becomes a control surface, not just a conversation.

## Open questions

- Default for local versus cloud STT and TTS, and how it routes through the brokerage. Privacy thesis points to local default with brokered fallback.
- The native sidecar process boundary and its auth to permagentd. Bearer token, or localhost-only trust like /events.
- Push-to-talk versus wake word for v1. Recommend push-to-talk first.
- Where the intent classifier lives (daemon, a worker model, the Orchestrator) and how it expresses the three routing outcomes.
- Latency budget. Clicky's full loop is the benchmark; the Mesh doc cites roughly 2 to 5 seconds for interactive use. recall() adds to this. Measure before committing to a target.
- Barge-in behavior. Can the user interrupt TTS midstream by speaking. Study Clicky's state machine.
- Permissions UX. Mic, Screen Recording, and Accessibility are three separate macOS prompts. Sequence and explain them so the first-run experience is not three scary dialogs in a row.

## Permissions required (macOS)

- Microphone: push-to-talk capture.
- Accessibility: global hotkey.
- Screen Recording and Screen Content (ScreenCaptureKit): Phases 3 and 4 only.

## Suggested first step for CC

Phase 0 only, framed as a read-only audit and spike with an explicit stop before any production code: resolve the sidecar-versus-Tauri question with a throwaway prototype, recommend local-versus-cloud STT and TTS with the whisper or llama-cpp CI cost called out, propose the transport, and write VOICE_ARCHITECTURE_DECISION.md. Stop and report before Phase 1.
