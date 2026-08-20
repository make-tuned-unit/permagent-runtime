# Permagent for iOS (SwiftUI)

The pocket client of the hub (see `docs/architecture/MULTI_DEVICE.md`).
On-brand: deep-void dark, cyan→violet ribbon, glass cards (`Theme.swift`
mirrors `ui/command-center/src/styles/tokens.ts` — keep in lockstep).

An **Apple Watch** companion (`PermagentWatch`) is embedded in this app.
Installing Permagent on iPhone installs the Watch app (Watch automatic
install is on by default). Two buttons: orb chat, and note dictate. The
watch talks to the iPhone over WatchConnectivity; the phone is the hop
to the hub (watchOS cannot run Tailscale). Chat joins the same hub
session as the phone, so the Mac, iPhone, and Watch are one conversation.

## v1 surfaces
Pairing (paste the Settings → Devices URL) · Decisions · In Flight · **Chat
(live)** — sends to the hub's `POST /reply` and streams the agent's answer over
SSE, so an ask on the phone runs on the Mac. **Dictate** — record a note on
the phone (16 kHz WAV), the HUB transcribes it with its local Whisper
(`/api/dictation/transcribe` — no cloud STT), you review/edit, confirm
heuristic to-do proposals, pick a project, and it lands as a project note
(+ board cards for confirmed to-dos). **Control** — agents (the merged
`/api/agents/roster` with the same on/off gates as Settings → Agents on
the Mac), automations, model picker, Features (Initiative / Playbook /
Concierge / Steward / Guard), and pronunciation (the same lexicon the
desktop voice settings write). Live events over the hub's `/events`
WebSocket; pairing token in the Keychain; zero user data on-device.

## Building
`project.yml` is the source of truth. From this directory:

```
xcodegen generate
```

Then open `PermagentMobile.xcodeproj` in Xcode 16+ (the mini). Requires the
hub daemon bound to the tailnet (`HOST=0.0.0.0`). Target iOS 17+ / watchOS
10+ (`.sensoryFeedback`, `AVAudioApplication`). Microphone and camera usage
strings are generated from `project.yml`. No speech-recognition entitlement
is needed: STT happens on the hub, not via Apple's recognizer.

## The companion model (Jesse's rule 2026-07-11)
- **Key info at a glance:** the Home tab — hub health, decisions pending,
  goals in flight, the durable activity feed (#619's journal), remote-hands
  explainer.
- **Remote hands:** everything the agent does acts on the HUB, and every
  connected UI renders it live — ask on the phone to open a site and
  the desktop's Build browser navigates; dispatch a goal and the desktop
  Kanban moves. Control of the desktop is a property of the architecture,
  not a feature to bolt on: the phone chat, Watch chat, and desktop chat
  are the same sessions on the same daemon.
- **Desktop↔desktop sync:** every surface that subscribes to /events is
  already multi-client live (Kanban, dashboards, decisions, world). The
  remaining polling/local-only surfaces are tracked in the multi-client
  liveness sweep issue.

## Roadmap
Choice/input decisions in-app (approve/reject ✓) · push via #618's escalation
channel · per-device tokens (#628) · Tailscale iOS hints. (Streaming chat ✓,
approve/reject decisions ✓ — verify the first `/reply` + `/answer` round-trips
on the mini once Xcode-built.) Watch complications / App Intents so a wrist
raise starts chat or dictation without hunting the icon.
