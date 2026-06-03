# Roadmap Candidates: ideas from Odysseus and Clicky

Source review of two competing local-first AI projects:

- Odysseus (https://github.com/pewdiepie-archdaemon/odysseus): self-hosted AI workspace, open source, free, BYO endpoints.
- Clicky (https://github.com/farzaa/clicky): native macOS menu-bar AI companion that sees the screen, talks, and points at UI. Swift, MIT licensed.

Three candidates below, ordered by strategic fit. Each is written to be split into its own issue. The voice/screen candidate has a separate, fuller scope doc (see EPIC_198_VOICE_SCREEN_SCOPE.md); the entry here is the summary stub.

---

## 1. Cookbook: hardware-aware model recommendations and one-click local serving

Source: Odysseus (its strongest single feature).

### Summary
A surface that detects the user's Apple Silicon hardware, recommends models it can actually run, and serves them locally in one click. Odysseus catalogues 270+ models with hardware-aware recommendations.

### Why it fits
- We are Apple Silicon native and hardware tiers are already central to our story (the Mesh vision doc turns on M1/16GB vs M4 Max/128GB).
- It is the local-first on-ramp to Mesh. Narrative ladder: Cookbook = what your machine runs, Mesh = what the network runs for you, Brain = what we run managed.
- Gives the free tier real teeth against free competitors and creates a clean upsell path into the paid brokerage.

### Scope sketch
- Hardware detection (chip, unified memory) and a model-to-tier mapping.
- A curated model catalogue with per-model memory and capability metadata.
- One-click serve via the existing local provider path (Ollama today).
- Surfaces in the app as a browsable, filterable list with "run on this machine" affordance.

### Effort
Substantial. Not a weekend feature. Catalogue, detection, and serving orchestration are each real work.

### Slot and dependencies
Ahead of or alongside Mesh as its local-first precursor. Reinforces the brokerage and Brain tiers at the same time.

---

## 2. Voice plus screen-grounded companion (Epic #198)

Source: Clicky is a working, MIT-licensed reference implementation of nearly the entire stack. Accelerates our existing voice-layer epic.

### Summary
Push-to-talk voice loop with the Orchestrator, optionally grounded in what is on screen, with the ability to point at on-screen UI elements. Clicky's loop: global hotkey, STT over websocket, transcript plus screenshot to the model over streaming SSE, TTS playback, and coordinate tags that fly a transparent cursor overlay to specific UI across monitors.

### Why it fits
- Voice is already on the roadmap as Epic #198 (STT, TTS, intent routing, OS control, Mobius/waveform aesthetic).
- On-screen pointing is a concrete, low-risk first form of "OS control": point at a UI element rather than automate it.
- It is a flagship demo that separates us from the chat-workspace crowd. Odysseus has nothing like it.
- Clicky being MIT and macOS-native means we can study the exact architecture that already works on our target platform.

### Scope sketch
See EPIC_198_VOICE_SCREEN_SCOPE.md for the full phased breakdown. Headline: study Clicky as design, not code, because we are Tauri plus Rust daemon and Clicky is native Swift. The screen-capture and pointing overlay likely want a small native sidecar.

### Effort
Substantial and multi-phase. The first usable milestone (talk to your Orchestrator, text-grounded, no screen) is reachable well before the full screen-pointing experience.

### Slot and dependencies
Starting now per current priority. Touches the daemon (transport), the chat reply path (Brain recall), and the Orchestrator (intent dispatch).

---

## 3. Compare: one prompt to many models, side by side

Source: Odysseus.

### Summary
Send one prompt to several models at once and view the answers side by side.

### Why it fits
- It is the natural user-facing face of the AI brokerage backend. If we are routing across providers and local models anyway, comparison is nearly free surface area.
- Markets the brokerage value directly: we route and compare across providers and local models so the user does not have to.

### Scope sketch
- Fan-out of one prompt to N selected providers/models through the brokerage path.
- Side-by-side rendering with per-model latency and cost where known.
- Reuses existing model selection and streaming infrastructure.

### Effort
Lowest of the three. A cheap win once the brokerage surfaces to users.

### Slot and dependencies
Opportunistic. Slot in when the brokerage backend surfaces. Take it because it is cheap and markets the brokerage, not because it is thesis-defining.

---

## Architecture note, not a roadmap item

Clicky's Cloudflare Worker key-proxy (the app talks to a proxy, the proxy holds keys and talks to providers, keys never ship in the binary) is the implementation blueprint for our paid AI brokerage. It validates the architecture we already intend. Whoever builds the brokerage should read Clicky's worker/src/index.ts before designing it. This is reference material, not a new issue.
