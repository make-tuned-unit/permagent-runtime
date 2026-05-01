# Permagent Desktop

Tauri-based macOS application. Single double-click launch with the daemon
embedded as a sidecar binary.

## Quick start

```bash
# Build everything (icons + UI + daemon + sidecar + app bundle)
npm run build:all

# Result:
#   src-tauri/target/release/bundle/macos/Permagent.app
#   src-tauri/target/release/bundle/dmg/Permagent_1.31.0_aarch64.dmg
```

## Development

Run the daemon and Tauri dev mode separately:

```bash
# Terminal 1: daemon
cargo run --release --bin permagentd -- agent

# Terminal 2: Tauri dev (opens app window with hot-reload)
cd ui/desktop && npm run dev
```

The dev window points at the Vite dev server (`:5273`) which proxies
API calls to the daemon on `:3001`.

## Build pipeline

| Script | What it does |
|--------|-------------|
| `build:icons` | Generate .icns + PNGs from logo |
| `build:ui` | Build command-center to dist/ |
| `build:daemon` | `cargo build --release --bin permagentd` |
| `build:sidecar` | Copy daemon binary with arch-specific name |
| `build:all` | All of the above + `tauri build` |

## Architecture

```
Permagent.app
├── Contents/MacOS/
│   ├── permagent-app     (14MB Tauri shell)
│   └── permagentd        (226MB daemon sidecar)
├── Contents/Resources/
│   ├── icon.icns
│   └── (bundled UI assets from command-center/dist)
└── Contents/Info.plist
```

On launch:
1. Tauri shell checks if daemon is already on `:3001`
2. If not, spawns `permagentd agent` as child process
3. Polls `:3001` until ready (up to 10s)
4. Opens window with command-center UI
5. On quit, kills the sidecar daemon

## Migrating from launchctl

If you previously ran permagentd via launchctl:

```bash
# 1. Unload the launchctl service
launchctl unload ~/Library/LaunchAgents/ai.permagent.daemon.plist

# 2. Remove the plist (optional — keeps things clean)
rm ~/Library/LaunchAgents/ai.permagent.daemon.plist

# 3. Launch Permagent.app (from /Applications or build output)
open /Applications/Permagent.app
```

The app now manages the daemon lifecycle. Your data at `~/.permagent/`
remains untouched.

If you want to run the daemon separately during development, the app
detects it on `:3001` and skips spawning a duplicate.

## Apple Silicon only

v1.0 targets `aarch64-apple-darwin` (Apple Silicon). Universal Binary
(Intel + ARM) and code signing are planned for future releases.

If macOS Gatekeeper blocks the unsigned app on first launch:
Right-click → Open → Open (bypasses Gatekeeper for that binary).
Or: `xattr -cr /Applications/Permagent.app`
