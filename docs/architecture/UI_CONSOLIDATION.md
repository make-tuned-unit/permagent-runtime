# UI Consolidation: goose2 stripped from build, command-center is the shipping UI

**Date:** 2026-04-30
**Commit baseline:** `a7561e514` (main)

## What Changed

Two build-config files modified:

1. **`ui/pnpm-workspace.yaml`** — Removed `goose2` and `desktop` (phantom directory) from pnpm workspace member list.
2. **`ui/package.json`** — Removed `acp` and `desktop` from npm workspaces (both pointed to non-existent directories).

No Rust code was changed. No command-center code was changed.

## What Was Already True (Pre-Commit)

The daemon (`permagentd`) was already serving command-center exclusively:
- `crates/goose-server/src/routes/mod.rs` lines 63-93 resolve UI from `ui/command-center/dist/` only.
- No reference to `goose2` exists anywhere in the Rust server code.
- The justfile (`just release-binary`) only builds Rust crates — no UI build steps.
- goose2's Tauri bundle (`com.goose.app`) was never built or installed.

The only linkage was pnpm-workspace.yaml listing `goose2` as a workspace member, meaning `pnpm install` at the `ui/` root would resolve goose2's dependencies.

## Parity Gap: goose2 Features Not Yet in command-center

The following goose2 sections have no command-center equivalent and should be ported in follow-up work before goose2 source is deleted:

| goose2 Feature | goose2 Location | command-center Status |
|----------------|-----------------|----------------------|
| Settings: Appearance (theme/accent/density) | `settings/ui/AppearanceSettings.tsx` | Missing |
| Settings: Voice Input (dictation providers) | `settings/ui/VoiceInputSettings.tsx` | Missing |
| Settings: Doctor (health checks) | `settings/ui/DoctorSettings.tsx` | Missing |
| Settings: Projects archive/restore | SettingsModal section | Missing |
| Settings: Chats archive/restore | SettingsModal section | Missing |
| Settings: General (language/i18n) | SettingsModal section | Missing (no i18n system in CC) |
| Full i18n support (en/es) | `shared/i18n/` | Missing |
| Agent provider cards (ACP setup UI) | `providers/AgentProviderCard.tsx` | Missing |
| Streaming markdown (streamdown) | `streamdown` dependency | Missing (uses react-markdown) |
| Drag-and-drop file attachments | Tauri shell integration | Missing |
| Deep link handling (goose://) | Tauri plugin-shell | command-center has `permagent://` scheme registered but not wired |
| Apps platform (MCP sandbox) | Complex multi-component system | Missing |

## What command-center Has That goose2 Does Not

| command-center Feature | Location |
|----------------------|----------|
| World View (placeholder) | `components/world/WorldView.tsx` |
| Brain panel (placeholder) | `components/brain/BrainPanel.tsx` |
| Workspace navigation (Chat/Skills/Trace/Brain) | `components/sidebar/Sidebar.tsx` |
| xterm.js terminal | `components/terminal/` |
| Browser panel | `components/browser/Browser.tsx` |
| Event stream viewer | `components/events/` |
| Automations sidebar | Store-driven workspace system |

## Orphaned Build References Cleaned Up

| File | Entry | Problem |
|------|-------|---------|
| `ui/pnpm-workspace.yaml` | `desktop` | Directory `ui/desktop/` does not exist |
| `ui/package.json` | `acp` | Directory `ui/acp/` does not exist |
| `ui/package.json` | `desktop` | Directory `ui/desktop/` does not exist |

## Verification Results

| Check | Result |
|-------|--------|
| `cargo build --release` | Pass |
| `npm run build` (command-center) | Pass — dist/ with 310 asset files |
| goose2 dist exists | No — never built |
| Spectral smoke tests (3) | Pass |
| Daemon serves command-center at `/ui/` | Pass — title: "Henry Command Center" |
| Recall fires on chat | Pass — "Recall injected 2 memories" |
| Remember fires after response | Pass — "Remembered chat turn: chat-..." |
| Tauri app installed | No — UI accessed via browser at localhost:3001/ui/ |
| World View renders | Placeholder only (globe emoji + title text) |

## Findings

1. **goose2 was never in the daemon's serving path.** The strip is a build-config cleanup, not an architectural change. The daemon has served command-center exclusively since the routes/mod.rs was written.

2. **Three phantom workspace entries existed** (`acp`, `desktop` in package.json, `desktop` in pnpm-workspace.yaml). All pointed to non-existent directories. Cleaned up in this commit.

3. **World View is a placeholder**, not a functional sprite-based view. It renders a globe emoji with "Global agent activity and connections" text. The sprite data infrastructure exists in the orphaned `agents_registry.rs` (per INHERITANCE_AUDIT.md) but was never connected to this component.
