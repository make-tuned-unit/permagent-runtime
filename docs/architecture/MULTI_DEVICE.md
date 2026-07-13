# Multi-Device Permagent — Hub and Spoke (ruled 2026-07-11)

Jesse's directive: install Permagent on every device, with the strongest home
machine (the M4) as the server for the largest local model. This document is
the architecture ruling for how that works.

## The core decision: don't sync — connect

**One Brain, one truth.** The hub (the most capable machine: most RAM, most
storage, always on) runs the daemon, the Brain (Spectral), the Librarian, all
local models, and holds every byte of user data. Every other device is a
**client of the hub over the tailnet** — it renders state and issues API
calls; it stores nothing but its pairing token.

This answers the "we don't use auth — how do they sync?" question by
dissolving it:

- **No sync** → no conflict resolution, no CRDTs, no merge bugs, no
  eventually-consistent Brain. A phone and a laptop looking at the Decision
  Inbox are looking at the same rows on the hub.
- **No accounts** → "auth" is *device pairing*: the hub's bearer
  `daemon_token` is the pairing secret. Adding a device = opening the pairing
  URL (Settings → Devices) once on that device; the token rides the URL
  fragment, is captured into the device's local storage, and is scrubbed from
  the URL (`api.ts` `browserToken()`). Removing all devices = rotating the
  token file on the hub.
- **Transport identity/encryption is Tailscale's job.** The tailnet already
  gives every device a stable, WireGuard-encrypted, ACL-able identity. We do
  not rebuild that layer; the bearer token gates the app, the tailnet gates
  the network.

## Data placement (the "strongest device" rule)

Storage and compute follow capability, exactly as directed:

| Concern | Placement | Why |
|---|---|---|
| Brain / memories / graph | Hub only | Recall and enrichment run next to the model; the data never travels except as query results |
| Local models (Ollama, Kokoro, sherpa) | Hub | The M4 runs the largest model any device can benefit from |
| Sessions, projects, decisions, journal | Hub (already single-DB) | Single-writer, no distribution needed |
| Client devices | Pairing token + UI state only | A lost phone leaks one revocable token, zero data |

**Latency analysis (why thin-client wins):** tailnet RTT is ~1–5 ms on LAN
and ~10–40 ms over WAN/DERP. Every heavy operation a client asks for —
recall, chat inference, describe — is dominated by *hub-side compute*
(hundreds of ms to seconds). The network share is noise. The only operations
where RTT is user-visible are keystroke-level interactions, which are all
client-local. Verdict: no satellite caches until real usage shows an
off-network need; when it does, the answer is a **read-through cache on the
satellite** (never a second writable Brain) or the Mesh track (#306).

## Enablement (all seams exist today)

1. **Bind the hub daemon to the tailnet:** `HOST=0.0.0.0` (or the Tailscale
   IP) in the daemon environment — the `Settings.host` layer already reads it
   (`configuration.rs`). Default stays loopback; exposure is explicit opt-in.
   The bearer middleware protects every real route regardless of bind.
2. **Pair a device:** Settings → Devices on the hub shows the pairing URL
   (`http://<magicdns-name>:3001/ui/#token=…`). Any browser on the tailnet —
   phone, laptop, tablet — becomes a full Permagent client (#366's token
   bootstrap). The web UI is served by the daemon at `/ui`.
3. **Biggest-model access from anywhere:** clients talk to the hub daemon, so
   they inherit its models automatically. A second *daemon* (e.g. a laptop
   wanting local tools but hub inference) points its Ollama provider at the
   hub via the existing `OLLAMA_HOST` config — no code needed.

## The native iOS app (SwiftUI)

The phone deserves better than a webview: **`ios/PermagentMobile`** is the
native SwiftUI client — on-brand (deep-void dark, cyan `#00D5FF` → violet
`#8D44AE`, glass materials), speaking the same daemon API with the same
pairing model. v1 surfaces are the *supervision* set: chat with Henry,
Decision Inbox (approve/decline), active goals, and live notifications from
`/events`. World View and Build stay desktop-class surfaces.

Build/verify note: Swift compiles only where Xcode lives (the mini); the
scaffold is authored to be opened as an Xcode project there. CI does not gate
`ios/` yet.

## Explicit non-goals (for now)

- Multi-user (the daemon is single-user `default` throughout — a second
  *person* is a second hub).
- Peer-to-peer/split inference — that is the Mesh research track (#306/#321),
  which the M0 spike showed needs speculative decoding to beat the per-token
  RTT wall. Client-server to the hub is the shipping topology.
- Offline mutation on satellites (would reintroduce sync; revisit only with
  a concrete need).
