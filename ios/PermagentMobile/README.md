# Permagent for iOS (SwiftUI)

The pocket client of the hub (see `docs/architecture/MULTI_DEVICE.md`).
On-brand: deep-void dark, cyan→violet ribbon, glass cards (`Theme.swift`
mirrors `ui/command-center/src/styles/tokens.ts` — keep in lockstep).

## v1 surfaces
Pairing (paste the Settings → Devices URL) · Decisions · In Flight · Chat
scaffold. Live events over the hub's `/events` WebSocket; pairing token in
the Keychain; zero user data on-device.

## Building
Open in Xcode 16+ on a machine with the iOS SDK (the mini): create an iOS App
project named PermagentMobile pointing at this folder's sources (or
`xcodegen`/SPM-ify later). Not gated by repo CI yet. Requires the hub daemon
bound to the tailnet (`HOST=0.0.0.0`).

## The companion model (Jesse's rule 2026-07-11)
- **Key info at a glance:** the Home tab — hub health, decisions pending,
  goals in flight, the durable activity feed (#619's journal), remote-hands
  explainer.
- **Remote hands:** everything Henry does acts on the HUB, and every
  connected UI renders it live — ask Henry on the phone to open a site and
  the desktop's Build browser navigates; dispatch a goal and the desktop
  Kanban moves. Control of the desktop is a property of the architecture,
  not a feature to bolt on: the phone chat and desktop chat are the same
  sessions on the same daemon.
- **Desktop↔desktop sync:** every surface that subscribes to /events is
  already multi-client live (Kanban, dashboards, decisions, world). The
  remaining polling/local-only surfaces are tracked in the multi-client
  liveness sweep issue.

## Roadmap
Streaming chat (SSE) · approve/decline decisions in-app · push via #618's
escalation channel · per-device tokens (#628) · Tailscale iOS hints.
