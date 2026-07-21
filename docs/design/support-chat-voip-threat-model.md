# Support-Chat + VoIP Threat Model — public ingress into a sovereign hub

> **Status: DESIGN — the #789 design-first deliverable. No code ships from this document.**
> Issue #789 flagged both halves as security-significant and design-first. This doc is the
> threat model + architecture for (a) a **per-project embeddable customer-support chat
> widget** that routes to the user's local Permagent hub via a quarantined sub-agent, and
> (b) **VoIP calling / call capture**. It mirrors the structure and rigor of
> `docs/design/federation-security-spec.md`: Phase-0 ground truth from code, an explicit
> adversaries×assets threat model, numbered open decisions with recommendations, explicit
> non-goals, and a phased plan with gates.
>
> **The one-sentence framing to hold onto:** every ingress the hub has today is either
> bearer-authenticated (device pairing) or pairing-coded (gateway); the support widget is,
> **by definition, anonymous hostile text flowing into an agent runtime**. Prompt injection
> is not an edge case here — it is the operating condition. The design answer is not
> "filter better"; it is **capability quarantine**: a support sub-agent that is *structurally
> incapable* of doing anything an injected prompt could exploit.
>
> **Scope split:**
> - **Half A — support chat** (§1–§5): quarantine model, ingress topology, the embed widget.
> - **Half B — VoIP** (§6): entitlement/WKWebView constraints, in-app vs integrate,
>   capture sides + consent, STT routing vs sovereignty.
> - Shared: Open Decisions (§7), phased plan + gates (§8), dependencies (§9).

---

## 0. Ground truth — what exists today (Phase 0)

Audited at repo `main` @ `e1f6f3ae` (2026-07-20). Every claim below was read from code,
not asserted.

### 0.1 Control-plane auth — landed, and it is the boundary we must not re-open

- `crates/goose-server/src/middleware/auth.rs`: `require_bearer_token` /
  `require_token_header_or_query` validate against the single `AppState.daemon_token` —
  **fail-closed** (no token configured → 503, never open), constant-time (`subtle::ct_eq`).
- `crates/goose-server/src/routes/mod.rs` is the single composition point. **Public
  (unauthenticated):** `/status`, `/version`, the `/events` WS (token-checked inside its
  upgrade), the localhost-only browser content/act bridges, and the `/voice` WS
  (token-checked via query param). **Everything else — reply, sessions, agent, brain,
  projects, decisions, security, dictation, tunnel, gateway, config… — is bearer-protected.**
  The session control plane (`/sessions/{id}/reply|cancel` + per-session SSE) accepts
  header OR `?token=` via the same fail-closed core.
- An **origin guard** (`middleware/origin_guard.rs`) wraps *everything* (public +
  protected): browser requests with a non-allowlisted `Origin` are rejected, closing the
  cross-site-from-the-user's-browser surface. Native/no-Origin clients pass.
- **Consequence for this design:** the daemon's HTTP surface is single-principal
  ("may this client drive this hub" — yes/no). There is **no second, lesser principal**
  ("may this anonymous visitor talk to one project's support agent"). Support-chat ingress
  must NOT be granted the daemon token, and must not terminate on the daemon's existing
  route surface at all (§4).

### 0.2 A tunnel already exists — and it is a cautionary precedent, not a foundation

- `crates/goose-server/src/tunnel/{mod,lapstone}.rs` + bearer-protected
  `/tunnel/start|stop|status` routes: inherited goose code that dials **out** over WSS to a
  Cloudflare Worker (`WORKER_URL = "https://cloudflare-tunnel-proxy.michael-neale.workers.dev"`
  — a third-party developer's worker, overridable via `GOOSE_TUNNEL_WORKER_URL`) and gets a
  public URL.
- It is a **generic HTTP proxy**: each tunnel frame carries `{method, path, headers, body}`
  and is replayed against `127.0.0.1:{port}` — i.e. an authorized tunnel caller reaches the
  **entire daemon route surface** (headers, including `Authorization`, are forwarded).
  Per-request auth is an `X-Secret-Key` header compared with `secure_compare` — which is
  **not** a cryptographic comparison (it compares `DefaultHasher` 64-bit hashes).
- **Verdict:** three properties disqualify it as support ingress as-is — third-party-owned
  relay, full-surface HTTP proxying, weak secret comparison. But its *shape* (hub dials
  out, no inbound port, store-and-forward frames) is exactly the right shape. §4 keeps the
  shape and replaces everything else with a **narrow-protocol** relay we control.

### 0.3 Sub-agent capability inheritance — the quarantine does NOT exist yet

- Sub-agents are spawned via `crates/goose/src/agents/platform_extensions/summon.rs`
  (`handle_delegate` → `build_task_config` → `run_subagent_task` in
  `agents/subagent_handler.rs`).
- **By default a sub-agent inherits the parent session's FULL enabled-extension set**
  (`build_task_config`, summon.rs:1286: `EnabledExtensionsState::extensions_or_default(...)`),
  which includes `developer` (**shell + file editing**, `default_enabled: true`),
  `extensionmanager` (**`search_memory` Brain read + `manage_extensions`**, default on), and
  `summon` itself. It runs in **`GooseMode::Auto`** (tool calls auto-approved, no
  confirmation), shares the global `PermissionManager::instance()` and the same
  `SessionManager`, and executes in the **parent session's working directory**.
- Restriction hooks that DO exist: `DelegateParams.extensions: Option<Vec<String>>` — an
  opt-in caller-supplied allowlist (`Some(vec![])` zeroes extensions); per-extension
  `available_tools` allowlists (empty = allow all); a recursion guard (a `SessionType::SubAgent`
  cannot delegate again); `max_turns` (default 25).
- Restriction mechanisms that do NOT exist: no default allowlist, no capability sandbox, no
  per-sub-agent permission scope, no fs isolation.
- **Brain writes are not an agent tool.** Chat-turn persistence is server-side
  (`brain_ops::spawn_persist_chat_turn`, called from `routes/reply.rs`,
  `routes/session_events.rs`, `routes/voice.rs`); the model-facing memory surface is
  read-only `search_memory`. A sub-agent run through `agent.reply` directly never traverses
  those routes, so it cannot persist chat-turn memories — but it **can read the whole
  Brain** if `extensionmanager` is inherited (it is, by default).
- **Consequence:** "spawn a Henry sub-agent for support" via today's delegate path would
  hand an anonymous visitor a shell, the Brain, and extension management, in Auto mode.
  The quarantine (§3) must be **built, and built structurally** — not as a config default.

### 0.4 The gateway — the exact anti-pattern, already shipping

- `crates/goose/src/gateway/` (Telegram): an external platform user pairs with a 6-char,
  5-min-TTL code, then gets a `SessionType::Gateway` session seeded with
  `get_enabled_extensions()` and **synced every message to the desktop owner's global
  config** — provider, model, extensions, mode. Guardrail: `GATEWAY_MAX_TURNS = 5`.
- i.e. after one pairing code, an external human drives an agent with the **owner's full
  tool surface** (shell, browser, Brain-read, delegate…). That is acceptable-ish for a
  *paired, owner-invited* Telegram user; it is the precise thing the support widget must
  never be: the widget's principal is **anonymous and adversarial**, and gets a quarantined
  agent or nothing. The pairing-code and turn-cap patterns are worth reusing; the
  capability posture is not.

### 0.5 Decision Inbox — the propose→confirm primitive (mitigation, exists)

- `crates/goose-server/src/routes/decisions.rs` + `crates/goose/src/decisions.rs`:
  `GET /api/decisions`, `POST /api/decisions/{id}/answer`, `GET /api/decisions/history`.
  Typed kinds (`approve_review`, `unblock`, `risk_gate`, `automation_proposal`,
  `enrichment_proposal`, `tool_approval`, `choice`) with fail-closed `malformed` storage;
  Tier 1 = Henry-policy-answerable, Tier 2 = human-only (`tool_approval` is always Tier 2);
  an **append-only `decision_audit`** table; answered decisions feed **Learn** ingestion
  into the Brain (`decision_inbox/learn.rs`) — i.e. Brain writes *already* have a
  human-approval funnel.
- #760 routes tool confirmations into the inbox (`tool_execution.rs` →
  `create_tool_approval_decision` / `bridge_provider_action_required`; delivery via
  `handle_confirmation` with honest `NO_WAITER_EFFECT` failure).
- **Consequence:** the "no Brain writes except via Decision Inbox proposals" rule in §3 is
  not aspirational — the funnel, tiering, audit, and Learn ingestion already exist. Support
  needs one new proposal kind, not a new subsystem.

### 0.6 Sovereignty guard + egress audit — the second mitigation primitive (exists)

- `crates/goose/src/providers/sovereign_guard.rs`: `SovereignGuardProvider` wraps **every**
  provider minted by the factory (`provider_registry.rs:81,92`) and gates the two egress
  methods (`stream`, `create_embeddings`). Cloud call → **audit row written first**
  (`egress_audit` table, schema v29, **append-only enforced by DB triggers**: UPDATE/DELETE
  rejected), then blocked (`[sovereign]` refusal, inner provider never called) if the
  context is sovereign. `DataLocality` is **fail-closed**: default `Cloud`; only in-process
  llama.cpp and loopback Ollama return `Local`.
- Routes: `GET/POST /api/security/sovereignty`, `GET /api/security/egress-log`.
- **Known gap (#765 follow-through):** `mark_session_sovereign`/`unmark_session_sovereign`
  are called **only from tests** — per-session/per-project sovereign marking is an API with
  no production caller; only the global toggle / `SOVEREIGN_MODE` env works today. §6.4 and
  §9 depend on this being wired.
- **Consequence:** the provider seam is the single choke point where support-session spend
  caps (§3 Q7) and call-audio STT locality (§6.4) can both be enforced and audited.

### 0.7 Voice pipelines — reusable for call transcription (exists, local-first)

Two distinct, shipping, local pipelines:

- **Dictation (batch):** `POST /api/dictation/transcribe` (bearer-protected, 25 MB WAV) →
  **local Whisper** (`crates/goose/src/dictation/`, bundled `whisper-base-q8_0.gguf`,
  provisioned via `/api/dictation/provision`). Capture is **in the webview**
  (`ui/command-center/src/hooks/useDictation.ts`: `getUserMedia` + Web Audio API, 16-bit
  PCM WAV encoded in-browser). Cloud dictation providers (OpenAI/Groq/ElevenLabs) are
  *defined* in `dictation/providers.rs` but **the shipped route only calls
  `transcribe_local`** — no cloud STT is wired.
- **Conversational voice loop (streaming):** the `/voice` WebSocket
  (`routes/voice.rs`) — client streams f32 PCM; server transcribes on stop with **local
  sherpa-onnx Moonshine** (`voice/sherpa_backend.rs`), replies through the agent, streams
  **local Kokoro** TTS back per-sentence. Proper-noun correction against Brain entities;
  user pronunciation lexicon.
- **Consequence:** mic-side call capture + transcription is **pipeline reuse, not
  greenfield** — capture, WAV encode, local STT, and transcript→agent/notes paths all ship
  today. What does not exist: any system/tab/remote-side audio capture, and any cloud-STT
  route.

### 0.8 The Tauri media-capture seam — what constrains in-app VoIP

Read from `ui/desktop/src-tauri/`:

- WKWebView does not expose `navigator.mediaDevices` by default. `main.rs
  enable_media_capture()` flips the **private `_mediaCaptureEnabled`** WKWebView preference
  per-window via the ObjC runtime (KVC `setValue:forKey:`), exception-guarded. It is invoked
  **per-window from JS on mount** (`enable_media_capture_cmd`, called by
  `ui/command-center/src/App.tsx:134` for the main window and `ChatApp.tsx:100` for the
  chat window) — deliberately not at launch (webview not initialized yet).
- The **Build-tab browser is a native *child* webview** (`browser.rs`:
  `WebviewBuilder::new(...)` + `.add_child(...)` on the main window). The enable command
  takes a `tauri::WebviewWindow`; **no code path enables media capture on the child
  webview** — so a call cannot run inside the in-app browser today (matches the #789
  scoping and the 2026-07-18 call-notes audit).
- Permissions posture: `Info.plist` carries **`NSMicrophoneUsageDescription` only** — **no
  camera key**. `Entitlements.plist` carries **`com.apple.security.device.audio-input`
  only** — no camera entitlement — and hardened-runtime entitlements **only take effect on
  a signed/notarized build** (ties VoIP-in-signed-app to the pre-DMG release-pipeline
  work).
- **Zero** `getDisplayMedia` / ScreenCaptureKit / camera code anywhere in the tree; zero
  WebRTC/SIP/LiveKit/Twilio code anywhere (workspace-wide grep; sole hit is a Whisper
  token-data file).
- **Consequence:** *audio-only, mic-side, in-our-own-windows* capture is proven and
  shipping (dictation + voice loop). Everything beyond that — camera, remote-side audio,
  in-browser-webview capture, screen/tab audio — is real new work with OS-permission
  surface area (§6).

### 0.9 Projects already have a KB substrate

`routes/projects.rs` (bearer-protected): per-project **documents** (upload/get/delete),
**notes**, **memory associations**, **people**, tags, and a **code index**
(`/api/projects/{id}/index-code`). There is **no "published/public" flag** on any of these
— everything is owner-private. The support KB (§3 Q2) is a *published subset* projection of
this substrate, which today does not exist.

### 0.10 What else does not exist (verified absences)

- **No rate-limiting middleware anywhere** on the daemon (grep: only incidental
  provider-side backoff hits). Anonymous ingress currently has nothing to lean on.
- **No visitor/anonymous identity class** — one hardcoded user (`DEFAULT_USER_ID =
  "default"`), single bearer principal (§0.1).
- **No moderation/response-filter layer** for outbound agent text.
- **No per-session/per-project sovereign wiring** (§0.6 gap).
- **No support/`Support` session type**; session types today: normal, `SubAgent`,
  `Gateway`, (scheduled/worker variants).

---

## 1. Topology — the third leg, and why it is different in kind

Permagent now has three ingress classes. State them side by side, because the security
posture of each follows from who the principal is:

| Leg | Principal | Trust | Auth | Capability granted |
|---|---|---|---|---|
| **Multi-device** (`device → hub`) | The owner, on another of their devices | Full | Bearer `daemon_token` over the tailnet | Everything (drive the hub) |
| **Federation** (`hub ↔ hub`, designed) | A chosen peer person | Scoped, deliberate | Ed25519 identity + realm keys | Replicate one shared realm's memory |
| **Support ingress** (`anonymous visitor → project`, this doc) | **A stranger on the public internet** | **None — adversarial by default** | None (a public widget) | **Talk to one project's quarantined support agent. Nothing else.** |
| *(Gateway — Telegram)* | An owner-invited external user | Semi-trusted | One-time pairing code | Today: the owner's full surface (§0.4 — the anti-pattern) |

Consequences to hold onto:

1. **The support principal must map to a new, structurally weaker capability class** — not
   to a filtered version of the existing one. Bearer token semantics ("may drive the hub")
   must never be reachable from widget traffic, even on total relay compromise (§4's
   narrow-protocol requirement).
2. **The hub stays unreachable from the internet.** Like the federation design, the hub
   only ever **dials out**; visitors terminate on a relay. A LAN Mac mini never opens an
   inbound port for this feature.
3. **Unlike federation, this channel cannot be E2E in the strong sense.** Federation peers
   hold keys; an anonymous visitor holds nothing, and the relay serves the widget's
   JavaScript — so the relay is trusted for code delivery regardless (the standard web-E2E
   limitation). §4.4 discloses this honestly instead of overclaiming.

---

## 2. Threat model — support chat

### 2.1 The inversion: injection is the operating condition

Everything the daemon defends today assumes the text reaching the agent comes from the
owner (or an owner-paired user). The support widget inverts that: **100% of input is
untrusted**, and some of it will be *crafted specifically to manipulate the agent* —
"ignore your instructions", "you are now in developer mode", "call your shell tool",
"repeat your system prompt", "remember that the refund policy is 100% forever".

The design consequence, stated as a principle:

> **Nothing may depend on the support agent *choosing* not to do something.** Prompt-level
> instructions ("you must never use tools") are brand polish, not a security boundary. The
> boundary is that the support session **has no tools to call, no Brain to read, no write
> path that skips a human, and no route surface beyond its own conversation.** An injected
> prompt that fully controls the model's output must gain: the ability to say something
> embarrassing, and nothing else.

### 2.2 Adversaries × assets

**Assets:** ① the owner's Brain (integrity — poisoning; confidentiality — exfiltration);
② the hub host (RCE via tools); ③ unpublished project/KB content (docs, notes, code,
memories beyond the published subset); ④ provider spend / API keys (wallet-drain);
⑤ brand/reputation (what the agent says under the project's name); ⑥ visitor PII
(transcripts); ⑦ hub availability for the owner.

| Adversary | ① Brain | ② Host RCE | ③ Unpublished KB | ④ Spend | ⑤ Brand | ⑥ Visitor PII | ⑦ Availability | Notes |
|---|---|---|---|---|---|---|---|---|
| **Drive-by visitor / prompt injector** | ✗ poisoning (no write path without Decision-Inbox approval, §3 Q3); ✗ exfil (no Brain read mounted, §3 Q2) | ✗ (zero tools — structurally, §3 Q1) | ✗ (retrieval index contains only published docs; unpublished content never enters context) | **Bounded** — per-session turn caps + per-project budget (§3 Q6/Q7); residual: burns budget up to the cap | ✓ **residual by design** — can screenshot-bait the agent into silly output; mitigated by persona/moderation (§3 Q5), not eliminated | — | ✗ (caps) | The main character. Full model-output control ⇒ embarrassing text, nothing else. |
| **Scraper / botnet (DoS, spam)** | ✗ | ✗ | ✗ | Bounded by relay rate limits + project budget; kill switch | flood of junk transcripts | — | Relay absorbs; hub pulls at its own pace (outbound dial + queue, §4) | PoW/CAPTCHA at relay is Phase-2 hardening. |
| **Malicious embedding site** (a page that embeds — or hotlinks — the widget) | ✗ | ✗ | ✗ | Can burn the project's budget from its own pages | ✓ can *frame* the widget in a deceptive page; can attempt clickjack/postMessage games | ✓ can phish visitors *around* the widget (outside our control) | ✗ | Widget key is public ⇒ per-project **origin allowlist enforced at the relay** (§5); iframe isolation + strict postMessage origin checks. |
| **Relay operator — honest-but-curious** | ✗ (relay never holds daemon token; narrow protocol has no route surface) | ✗ | ✗ | ✗ | ✗ | ✓ **sees transcripts in plaintext in v1** (§4.4 — disclosed residual; OD-7) | can degrade | Same class as the federation relay for metadata, but *content* is visible in v1 — unlike federation packs. |
| **Relay operator — malicious** | ✗ (no daemon credentials to steal; hub validates message schema, treats relay data as untrusted) | ✗ (narrow protocol: relay can only deliver chat frames, not HTTP requests — the anti-lapstone property, §4.3) | ✗ | Can replay/forge visitor traffic up to caps | ✓ can serve a tampered widget (code-delivery trust, §4.4) | ✓ | ✓ can drop/withhold | The load-bearing design choice: **even a fully hostile relay reaches only the quarantined conversation surface.** |
| **Competitor / brand attacker** | ✗ | ✗ | ✗ | caps | ✓ adversarial screenshots, review-bombing with agent quotes | — | ✗ | Mitigation is honesty: visible "AI assistant" labeling, no-authority persona (§3 Q5), owner kill switch. |
| **The owner (misconfiguration)** | — | — | ✓ *accidentally publishing* a sensitive doc into the support KB | ✓ forgetting caps | ✓ | — | — | Publishing is **explicit, per-document, with a confirmation step** (§3 Q2); default-unpublished. |

### 2.3 Explicit non-goals (correctness, not gaps)

1. We do **not** try to make the model injection-proof. We make injection **yield nothing**:
   the blast radius of full model compromise is one conversation's text.
2. We do **not** hide the support **system prompt or the published KB** — treat both as
   public the moment the widget ships. Nothing secret may ever be placed in either.
3. We do **not** guarantee availability against a malicious relay (same posture as
   federation: it can deny service; mitigation is monitoring + self-hosting, not crypto).
4. We do **not** promise visitor-side E2E in v1 (§4.4 — the relay serves the JS; honest
   TLS-only posture, HPKE hardening as v2 depth, OD-7).
5. We do **not** moderate the *visitor's* behavior toward third parties, or what the
   embedding page does around our iframe.
6. Support-chat content is **not** Brain memory. Transcripts are quarantined data; only
   owner-approved distillations (via Decision Inbox) ever become memories.

---

## 3. The capability-quarantine model for the support sub-agent

The support agent is a **new session class**, not a delegated sub-agent. Today's delegate
path inherits the parent's full tool surface by default in Auto mode (§0.3) — the opposite
of what this principal may have. Quarantine rules, each **structural** (enforced by
construction server-side, never by config or prompt):

**Q1 — Zero tools. Deny-by-construction.**
A new `SessionType::Support` whose session constructor takes **no extension list at all** —
not "empty by default" (config can drift; `extensions_or_default` shows how defaults
propagate) but *no parameter to widen*. The agent object for a support session is built
with an empty extension set and asserts it: a CI test creates a support session and asserts
the tool registry is literally empty (the `full-test-suites` bar). No `developer`, no
`extensionmanager` (⇒ no `search_memory` — this is what keeps the Brain unreadable), no
`summon` (no delegation), no browser. The existing `DelegateParams.extensions` filter and
recursion guard are precedents, but Q1 must not reuse the delegate path.

**Q2 — RAG over an explicitly-published per-project KB only.**
- A per-document/note **`published_to_support` flag** on the existing project KB substrate
  (§0.9), default **false**, flipped only by an explicit owner action with a confirmation
  step ("this becomes public to anyone on the internet").
- Retrieval for support sessions runs over a **separate index** built exclusively from
  published items — not over Brain recall with a filter. (Same lesson as federation
  guarantee A vs B: a *structural* export gate — the sensitive content is never in the
  index — beats a read-time filter that can regress. The Spectral −9pp lesson also applies:
  filters on the read path are the fragile kind.)
- Context assembly for a support turn = system prompt + published-KB retrieval hits +
  this visitor's conversation. **Nothing else.** No project memories, no people graph,
  no notes/docs lacking the flag, no owner chat history.

**Q3 — No Brain writes except Decision Inbox proposals.**
- Support sessions are excluded from the server-side chat-persistence path
  (`spawn_persist_chat_turn` must gate on session type) — no automatic memory formation
  from visitor text, ever. This is the **poisoning gate**.
- The one write path: the support pipeline may file a **new Decision-Inbox kind**
  (`support_capture`, **Tier 2 / human-only**, like `tool_approval`) proposing a
  distillation — "visitor reported bug X", "requested feature Y", "asked for a human".
  Only on the owner's approval does Learn ingestion (§0.5) turn it into a memory, and the
  memory's provenance marks it visitor-originated. The append-only `decision_audit` gives
  the poisoning-attempt paper trail for free.
- Escalation ("talk to a human") is the same mechanism: a `support_capture` card with the
  conversation attached, surfaced in the owner's inbox (and via the Watcher's ntfy push
  once enabled).

**Q4 — Session isolation.**
One session per visitor conversation; short TTL; server-enforced turn cap per conversation
(the `GATEWAY_MAX_TURNS` precedent) and message-size cap; **no cross-visitor context** —
a support session can never load another session's transcript; no shared scratch state.
Support sessions live in a distinct store/namespace so no existing recall or listing
surface picks them up.

**Q5 — Persona + output constraints (brand layer, not a security boundary).**
A dedicated support persona (the `WorkerPersona` mechanism, §0.3, gives the plumbing):
visible "AI assistant" disclosure, scoped to the project, **no authority** (cannot promise
refunds/discounts/commitments — state this in the prompt AND in the widget's static UI
footer so the guarantee doesn't rest on the model), escalate-don't-improvise. An outbound
moderation/response-filter pass is Phase-2 hardening. Everything in Q5 is
best-effort-by-design; Q1–Q4 are the boundaries.

**Q6 — Rate limits + quotas (two layers).**
- **Relay-side** (before the hub sees anything): per-IP and per-project message rate
  limits, connection caps, payload caps. This is where DoS dies — the hub *pulls* from a
  bounded queue and can simply stop pulling.
- **Hub-side** (defense in depth, since the relay may be hostile): per-conversation turn
  cap, per-project **daily token budget** enforced at the sovereign-guard provider seam
  (§0.6 — the one choke point every inference already passes through), concurrent
  support-session cap, and a per-project **kill switch** (widget off = relay rejects at
  the edge + hub stops pulling).

**Q7 — Spend + model routing.**
Support inference defaults to a **cheap model** (the coding-harness cost-router precedent);
the per-project budget (Q6) bounds worst-case wallet-drain; every support-session cloud
call is **egress-audited** like any other (`egress_audit` rows carry `session_id` /
`project_id`, so support traffic is separable in the log). Provider API keys never leave
the hub; the relay and widget never see them.

### Defense-in-depth map (which layer stops which threat)

| Threat | Structural stop | Depth |
|---|---|---|
| Injection → tool execution / RCE | Q1 zero tools (nothing to call) | n/a — no second layer needed, and that's the point |
| Injection → Brain exfiltration | Q2 no Brain read mounted; index contains only published docs | Q5 prompt scope |
| Injection → Brain poisoning | Q3 no write path; Tier-2 human approval on every capture | provenance marking; audit trail |
| Exfiltration of unpublished KB | Q2 structural index gate | owner publish-confirmation (mis-config guard) |
| DoS / flood | Q6 relay-edge limits + pull-based hub | hub-side caps, kill switch |
| Wallet drain | Q6/Q7 budget at provider seam | cheap-model default, egress audit visibility |
| Brand damage | — (not structurally stoppable) | Q5 persona + disclosure + moderation pass + kill switch |
| Relay compromise → hub compromise | §4 narrow protocol (no route surface, no credentials at relay) | schema validation on every relay frame; hub treats relay as untrusted input |
| Widget abuse from foreign sites | §5 relay-side per-project origin allowlist | rate limits, budget |

---

## 4. Ingress topology

### 4.1 Requirements

(i) The hub never opens an inbound port (LAN Mac mini reality). (ii) The relay must be
unable to reach any daemon capability beyond "deliver a support-chat frame" — even if
fully compromised. (iii) Anonymous visitors, so the edge must absorb abuse. (iv) Works
when the hub is temporarily offline (queue + "we'll get back to you" degradation).
(v) Self-hostable, consistent with the sovereignty story.

### 4.2 Options

| Option | Verdict | Why |
|---|---|---|
| **(a) Reuse the lapstone tunnel** | **Rejected** | Third-party-owned worker; generic full-surface HTTP proxy (§0.2) — one bug or leaked secret away from "unauth RCE with extra steps"; weak secret compare. Its *shape* survives; the artifact doesn't. |
| **(b) Raw port-forward / reverse proxy to the daemon** | **Rejected** | Puts the whole bearer-gated surface on the internet; single-token auth was never designed as an internet-facing boundary; violates (i) and (ii). |
| **(c) Tailscale Funnel** | **Rejected as foundation** | Still terminates on the daemon's HTTP surface (fails ii); couples the feature to Tailscale; no abuse-absorbing edge, no queue. Fine for an owner's personal demo, not for the product feature. |
| **(d) Managed cloud queue (SQS/PubSub-style) + serverless widget API** | Viable, not preferred | Meets i/iii/iv but hard-couples to a cloud vendor, complicates self-hosting, and still needs all the relay logic (origin allowlists, rate limits) somewhere. |
| **(e) Narrow-protocol support relay — hub dials out (recommended)** | **Recommended** | Small service we author: visitors POST/WS chat frames to it; it queues per-project; the hub connects **outbound** over WSS and consumes/replies. Shares deployment DNA with the federation relay. |

### 4.3 Recommended architecture (option e)

```
visitor browser ──TLS──> support relay (VPS / our infra / self-hosted)
   widget iframe            • serves widget.js + iframe page
                            • per-project: origin allowlist, rate limits, queue
                            • message schema: {project_widget_id, conversation_id,
                              seq, role, text, ts} — NOTHING ELSE
                                   ▲
                                   │ outbound WSS dial, relay-scoped credential
hub (daemon) ──────────────────────┘
   • support-ingress worker: pulls frames, validates schema hard,
     runs the quarantined support session (§3), pushes reply frames
   • daemon token NEVER leaves the hub; relay holds a separate
     per-hub relay credential that authorizes ONLY queue pull/push
```

The load-bearing property, stated as the anti-lapstone invariant:

> **The relay protocol has no `path`, no `method`, no `headers`.** A relay frame is a
> typed chat message, validated against a strict schema on the hub, processed exclusively
> by the support-ingress worker, which can do exactly one thing with it: append it to a
> quarantined support conversation. There is no code path from relay bytes to the axum
> router. Relay compromise therefore yields: transcript access (§4.4) and service denial —
> never hub capability.

Auth split (three credentials, none interchangeable):
- **Visitor → relay:** none (anonymous) + per-project **widget id** (public, §5) +
  relay-issued per-conversation token (continuity only, no privileges).
- **Hub → relay:** a per-hub **relay credential** minted at project-publish time; scopes to
  that hub's project queues; revocable; stored in the existing secrets dir/keyring pattern.
  Compromise of it = read/inject support traffic for that hub's projects — never daemon access.
- **Owner → daemon:** the existing bearer token, untouched; all support admin surfaces
  (publish KB, budgets, kill switch) are ordinary bearer-protected routes.

Relation to the **federation relay** (`federation-security-spec.md` §6): same deployment
class (dumb, internet-facing, holds queues, honest-but-curious by assumption), and should
share infra/ops. But keep the **namespaces and credentials disjoint**: federation queues
hold E2E ciphertext between keyed peers; support queues hold plaintext-to-the-relay frames
from anonymous strangers. A support-relay bug must not be able to touch federation packs.

### 4.4 What the relay sees — the honest disclosure

- **Sees (v1):** support transcripts in plaintext (it terminates visitor TLS), visitor IPs,
  timing/volume, which projects are active, the widget-embedding origins.
- **Never sees:** the daemon token, provider API keys, anything about the Brain or
  unpublished KB, non-support daemon traffic.
- **Why not E2E like federation:** the visitor holds no key material, and the relay serves
  the widget JavaScript — a malicious relay can ship key-stealing JS regardless. So
  visitor↔hub HPKE sealing (widget fetches the hub's public key, seals frames; relay
  carries ciphertext) upgrades the *honest-but-curious* posture only. That is still worth
  doing — it's the difference between "our relay logs contain customer conversations" and
  "they contain ciphertext" — but it must be sold honestly as HBC hardening, not E2E
  (OD-7, v2).
- Mitigations meanwhile: minimal retention at the relay (deliver-and-delete; transcripts
  persist on the hub, where they belong), self-host option, and the disclosure itself in
  docs/marketing. Do not overclaim — the federation spec's discipline applies verbatim.

---

## 5. The embed widget

**What a customer embeds** — one script tag:

```html
<script src="https://support.<relay-domain>/widget.js"
        data-project="wgt_AbC123..." async></script>
```

- `widget.js` creates a **sandboxed iframe** whose src is the relay origin
  (`allow-scripts allow-same-origin` on the relay origin only; no `allow-top-navigation`,
  no camera/mic/geolocation permissions in the iframe `allow` list — a support chat needs
  none of them in v1). All chat UI and network I/O live inside the iframe on the relay
  origin; the host page never sees transcript content.
- **`data-project` (the widget id) is public by design** — it appears in page source.
  It is an identifier, not a secret. Abuse control is therefore: a per-project **origin
  allowlist registered at the relay** (frames rejected unless the iframe's
  embedding-origin — checked via `Origin`/ancestor headers — matches), plus rate limits
  and the project budget. Hotlinking the widget id from a foreign site buys an attacker
  nothing but rejected frames.
- **Host-page ↔ widget messaging** (open/close/prefill): `postMessage` with strict origin
  pinning both directions, versioned message schema, no transcript data ever posted to the
  host page.
- **What the widget can reach:** exactly the relay origin. It never learns the hub's
  address; there is nothing to leak. CSP on the iframe page pins `connect-src` to the
  relay origin.
- **Visitor state:** a relay-issued conversation token in iframe-scoped storage —
  continuity only. No tracking cookies on the host page. Visitor PII policy: collect
  nothing by default; optional email field for "get back to me" flows, carried in the
  frame schema, stored on the hub, retention-bounded at the relay (§4.4).
- **Degraded modes (honest UI):** hub offline → widget says "message received, replies by
  email/later" (queue holds frames); project kill-switched → widget renders disabled.
  No fake liveness — the wired-UI bar applies to the widget too.

---

## 6. VoIP calling

### 6.1 Ground truth constraints (from §0.7/§0.8)

Proven today: audio-only mic capture in our own two windows (main + chat) via the
per-window `_mediaCaptureEnabled` enable + `getUserMedia`; mic Info.plist key + hardened-
runtime audio-input entitlement; local STT twice over (Whisper batch, Moonshine
streaming); local TTS (Kokoro); transcript→agent and transcript→notes paths. Not existing:
camera (no plist key, no entitlement, no code), any remote-side/system/tab audio capture
(no ScreenCaptureKit, no getDisplayMedia), media capture in the Build-tab child browser
webview (no enable path for child webviews), any WebRTC/SIP stack, cloud STT wiring.
Entitlements bind only on signed builds → any in-app calling story is sequenced behind the
release-pipeline work (§9).

### 6.2 Scope: in-app calls vs integrate-with-existing

Three candidate scopes, in ascending cost:

- **(A) "Call notes" — integrate with existing calls (recommended first).** The user takes
  calls wherever they already do (Meet/Zoom/phone); Permagent captures **the user's own
  mic side** in the chat window (shipping capture path), transcribes **locally** (shipping
  STT), and turns transcripts into notes/CRM-ish follow-ups via the existing
  transcript→agent path — with Decision-Inbox proposals for anything that becomes a memory
  or an action. Zero new OS surface, zero telephony infra, reuses ~everything (§0.7). This
  is also what the 2026-07-18 call-notes feasibility audit sized as mostly-existing.
- **(B) Both-sides capture for (A).** Remote-side audio requires system/tab audio capture
  — ScreenCaptureKit + TCC screen/audio permission on macOS: a **Large** new subsystem
  (per #789), with the consent problem attached (§6.3). Phase after (A), gated on the
  consent ruling.
- **(C) In-app calls.** Two sub-cases: *owner-to-contact calling* inside Permagent (a full
  WebRTC/SIP subsystem: signaling, TURN/STUN or a provider — greenfield, §0.8), and
  *visitor voice via the support widget* (all of §2–§5's threat model **plus** live media,
  consent, and telephony abuse — the widget iframe currently grants no mic permission by
  design). Both deferred: (C) is a new subsystem competing against the moat ruling
  ("validate before stacking"), and support-voice specifically multiplies an
  already-novel attack surface. Design-only until support-chat text has shipped and
  survived contact with reality.

### 6.3 Capture sides + consent law

- **Mic-side-only (default, ships with A):** records only the user's own voice. The
  consenting party is the recording party — the safe posture everywhere, including
  all-party-consent jurisdictions (California, Washington, Florida, Illinois, and others;
  ~a dozen US states plus stricter regimes abroad). Ship this without a consent UI beyond
  a visible "capturing" indicator.
- **Both-sides (B) is a consent feature, not just a capture feature:** an explicit
  per-call consent flow (announce-and-confirm, or the meeting platform's native recording
  notice) must gate the capture path *in code* — not a settings checkbox the user can
  set-and-forget, because all-party-consent violations are per-call. Recommendation:
  both-sides capture refuses to start until the per-call consent affirmation is given, and
  the affirmation is logged (append-only, alongside the transcript's provenance). This doc
  flags the policy shape; the exact UX is one of the standing call-notes rulings for Jesse
  (not a lawyer; this is engineering posture, not legal advice).

### 6.4 STT routing vs sovereignty

Call audio is among the most sensitive data the product will ever touch (other people's
voices, business conversations). Posture:

- **Local STT is the mandatory default for call audio.** Both local engines already ship
  (§0.7); there is no functional gap forcing cloud STT into the MVP. The existing-but-
  unwired cloud dictation providers **stay unwired** for call capture in v1.
- If cloud STT is ever offered (accuracy/language reasons), it must be (i) explicit
  per-project opt-in, (ii) routed through the sovereign-guard seam so it is
  **egress-audited** (`kind` extended beyond `"inference"|"embedding"` to cover STT, or a
  parallel audit row), and (iii) hard-blocked for sovereign contexts. **Dependency:** the
  audio path today (`transcribe_local`, sherpa in goose-server) does *not* pass through
  `SovereignGuardProvider` — fine while local-only; the moment a cloud STT path is wired,
  it must be wrapped at the same factory-style choke point, and the #765 per-session
  sovereign wiring gap (§0.6) must be closed so *per-project* sovereignty can pin call
  audio local even when global sovereign mode is off.
- Transcripts follow the same Brain rules as everything else: call transcripts →
  notes/documents directly (owner's own data, owner-initiated), but *distilled memories
  and CRM-ish facts about other people* go through Decision-Inbox proposals — consistent
  with §3 Q3 and the existing enrichment_proposal pattern.

---

## 7. Open Decisions

> Each with a recommendation, mirroring the federation spec. OD-1..OD-3 are the #789
> "rulings needed" items; the rest fell out of Phase 0.

**OD-1. Support sub-agent capability set.**
*Fork:* zero-tools structural quarantine vs "restricted toolset" (e.g. a KB-search tool +
a ticket tool) vs reusing delegate with an extensions filter.
**Recommendation: zero tools, dedicated `SessionType::Support`, deny-by-construction
(§3 Q1); RAG runs in the pipeline around the model, not as a model-invoked tool.** Every
tool added to this surface is attacker-invocable by definition; a retrieval step the
*pipeline* performs is not. Do not reuse the delegate path (§0.3) or the gateway posture
(§0.4).

**OD-2. Support KB scope.**
*Fork:* filtered Brain recall vs a separate published-only index; implicit publishing
(e.g. "all project docs") vs explicit per-item flags.
**Recommendation: separate index over explicitly-published items only, per-item
`published_to_support` flag, default-unpublished, confirmation step on publish (§3 Q2).**
Structural exclusion over read-time filtering, same reasoning as federation guarantee A.

**OD-3. Brain-write posture for support traffic.**
*Fork:* auto-ingest transcripts vs propose-only vs no capture at all.
**Recommendation: no direct writes; `support_capture` Decision-Inbox kind, Tier-2
human-only, Learn ingestion only post-approval, visitor provenance on resulting memories
(§3 Q3).** Auto-ingestion of anonymous text is Brain poisoning as a feature; zero capture
throws away the product value. The inbox already provides exactly the needed funnel.

**OD-4. Ingress topology.**
*Fork:* §4.2's options (a)–(e).
**Recommendation: narrow-protocol outbound-dial support relay (e), sharing deployment
infra with the federation relay but with disjoint namespaces/credentials; explicitly
reject lapstone reuse and anything that terminates on the daemon's HTTP surface (§4.3).**

**OD-5. Who hosts the relay.**
*Fork:* we host (product feature, ops+liability+transcript custody) vs self-host-only
(sovereignty-pure, adoption-hostile) vs both.
**Recommendation: we host the default relay; the relay is open-source and self-hostable
from day one; relay retention is deliver-and-delete (§4.4).** Custody of plaintext
transcripts (until OD-7 hardening) is the liability to price before GA — flagged for
Jesse, interacts with pricing.

**OD-6. Widget authentication + abuse control.**
*Fork:* secret embed keys (leak instantly in page source — false security) vs public
widget id + relay-enforced origin allowlist.
**Recommendation: public widget id + per-project origin allowlist at the relay + per-IP/
per-project rate limits + per-project kill switch (§5).** Never rely on the embed key
being secret.

**OD-7. Visitor-channel encryption.**
*Fork:* TLS-only (relay sees plaintext) vs HPKE widget→hub sealing (HBC-hardening) vs
claiming E2E.
**Recommendation: v1 TLS-only with the residual disclosed; v2 HPKE sealing as
honest-but-curious hardening; never market it as E2E while the relay serves the JS
(§4.4).**

**OD-8. Support spend control.**
*Fork:* trust rate limits alone vs hard budget enforcement.
**Recommendation: per-project daily token budget enforced at the sovereign-guard provider
seam (the existing single choke point), cheap-model default for support inference,
kill switch; budget exhaustion degrades the widget honestly ("replies delayed") rather
than silently dropping (§3 Q6/Q7).**

**OD-9. VoIP scope.**
*Fork:* §6.2 (A) call-notes integrate-first vs (C) in-app calling; support-widget voice.
**Recommendation: (A) first — mic-side call notes on the shipping capture+STT pipelines.
In-app calling (C) stays design-only; support-widget voice is explicitly deferred beyond
that.** This is also the moat-ruling-compliant answer: (A) validates the actual
differentiator (Brain + local-first) with near-zero new surface.

**OD-10. Capture sides.**
*Fork:* mic-side-only vs both-sides (ScreenCaptureKit, Large) with consent flow.
**Recommendation: mic-side-only default; both-sides is a separate later phase, gated on a
per-call consent affirmation enforced in code and logged (§6.3), and on the standing
call-notes consent ruling.**

**OD-11. STT routing for call audio.**
*Fork:* local-only vs cloud-permitted.
**Recommendation: local-only mandatory in v1 (both engines ship today); any future cloud
STT is per-project opt-in, egress-audited at the guard seam, and blocked for sovereign
contexts — which requires closing the #765 per-session sovereign wiring gap first
(§6.4).**

**OD-12. Sequencing.**
*Fork:* build now vs after the security wave.
**Recommendation: nothing from this doc ships before the pre-DMG security criticals land
(C3 injection posture included — it is the same threat class this doc quarantines) and the
moat ruling's validate-first bar is met. Support chat before any VoIP-in-app work;
call-notes (A) may proceed in parallel since it adds no new ingress.** Matches #789
ruling-need 4.

---

## 8. Phased build plan

**Phase 0 — this document.** Jesse rules OD-1..OD-12.
> **Gate G0:** rulings recorded; pre-DMG security criticals merged (control-plane auth ✅
> landed; download integrity + C3 injection posture pending); federation-relay infra
> decisions known (shared deployment DNA, §4.3).

**Phase 1 — support chat MVP.**
Scope: `SessionType::Support` with structural zero-tool construction + CI assertion (Q1);
`published_to_support` flag + separate support index + publish-confirmation UI (Q2);
support persistence exclusion + `support_capture` Decision-Inbox kind, Tier 2 (Q3);
support-relay v0 (narrow schema, per-project queues, origin allowlist, per-IP/project rate
limits, deliver-and-delete); hub support-ingress worker (outbound WSS, hard schema
validation, relay credential in keyring/secrets); widget v0 (script + sandboxed iframe,
degraded modes); per-project budget at the provider seam + kill switch (Q6/Q7); owner
admin surfaces as ordinary bearer-protected routes with real wired UI.
> **Gate G1 (exit = evidence, not assertion):**
> 1. **Quarantine proof:** CI asserts a support session's tool registry is empty; an
>    injection-eval corpus (tool-invocation attempts, system-prompt exfil, KB-exfil,
>    Brain-write attempts) runs against the real session class and demonstrates *no
>    capability effect* — the only observable outcome of any attack is text.
> 2. **Publish-gate proof:** retrieval eval shows zero unpublished-item leakage into
>    support context (property-style test on the index builder).
> 3. **Abuse proof:** rate limits and budget exhaustion behave as specified under load;
>    kill switch verified end-to-end (relay edge + hub).
> 4. Mini validation of the full loop (widget on a real external page → relay → mini hub
>    → reply), per the runtime-diagnosis-on-the-mini rule.

**Phase 2 — support hardening.**
PoW/CAPTCHA at the relay edge; outbound moderation/response-filter pass (Q5 depth);
HPKE widget→hub sealing (OD-7 v2); relay self-host docs + packaging; transcript retention
policy + visitor PII controls; abuse analytics for the owner ("what are attackers trying").
> **Gate G2:** external-facing pen-test/red-team of widget+relay; HBC-hardening claims
> reviewed against §4.4's honesty constraints before any marketing copy.

**Phase 3 — VoIP (A): call notes, mic-side.**
Chat-window capture reuse; local STT (choose one of the two engines for calls —
consolidation opportunity, §0.7); "capturing" indicator; transcript→notes; distillations
via Decision Inbox; per-session/per-project sovereign wiring closed (#765 gap) so call
audio can be pinned local per-project.
> **Gate G3:** consent ruling for mic-side recorded (expected trivial); sovereignty
> assertion — demonstrate zero cloud egress during an end-to-end call-notes session
> (egress log empty for the session, on the mini).

**Phase 4 — VoIP (B): both-sides capture.** ScreenCaptureKit subsystem (Large); in-code
per-call consent affirmation + logging (§6.3).
> **Gate G4:** the consent-UX ruling; signed-build entitlement validation on the mini.

**Phase 5 (deferred indefinitely) — in-app calling (C) / support-widget voice.**
Re-enter through a fresh design round; do not inherit approval from this doc.

---

## 9. Dependencies & coordination points

1. **Pre-DMG security gate (launch-blocker list):** control-plane auth ✅ (this doc builds
   on it, §0.1); **C3 injection→shell posture** is the same threat class §3 quarantines —
   land it first so the two designs share vocabulary and tests; download integrity +
   release pipeline gate the signed-build entitlement story for VoIP (§6.1).
2. **Federation relay (federation-security-spec §6):** shared deployment infra, disjoint
   namespaces/credentials (§4.3). Build order: whichever relay ships first establishes the
   ops pattern; the support relay must not weaken federation's ciphertext-only claims by
   cohabitation.
3. **#765 sovereignty follow-through:** per-session/per-project sovereign marking is
   currently test-only (§0.6). Required by Phase 3 (per-project local-only call audio) and
   generally overdue — the flag must drive both enforcement points per the federation
   spec's §8.3 single-source-of-truth rule.
4. **Decision Inbox:** one new kind (`support_capture`, Tier 2) + session-type gating of
   chat persistence; no structural changes. Coordinate with the decision-spine work so the
   kind gets a typed payload schema (malformed-fail-closed like the others).
5. **Moat ruling / sequencing:** this is new surface area; OD-12 keeps it behind the
   validate-first bar. Call-notes (Phase 3) is the moat-aligned half — it exercises
   Brain-changes-behavior on the user's real workflow; the widget is the
   revenue/product-surface half.
6. **Testing bar:** every phase gate above is written as evidence-not-assertion
   deliberately — comprehensive unit+integration+wiring tests and no dead/decorative UI,
   per the standing hard bar.

---

*Prepared as the #789 design-first deliverable. Companion to
`docs/design/federation-security-spec.md`; audited at `main` @ `e1f6f3ae`, 2026-07-20.*
