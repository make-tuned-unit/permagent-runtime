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

## Roadmap
Streaming chat (SSE) · approve/decline decisions in-app · push via #618's
escalation channel · Tailscale iOS detection hints.
