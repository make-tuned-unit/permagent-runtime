# Native Record accessibility and isolation source audit — 2026-10-07

Read-only source inspection in world-production-20261006, after fixture host compile. No production edits, daemon launches, UI control, or credential reads performed.

## Record modal accessibility

Parent observed visible Record modal but root-HTML-only accessibility tree in both baseline and review; Cancel restores tree. Current integration `ui/command-center/src/components/voice/MeetingRecorder.tsx` does **not** import/use `useModalLayer`, and no such symbol exists in current command-center source. Picker is a portal under document.body (line348), with `role="dialog" aria-modal="true"` (377). It applies no aria-hidden/inert to ancestors. Its only focus transfer is line160: focus the filter when projects.length > FILTER_THRESHOLD (6). At <=6 projects, initial loading, or failure, focus can remain behind the modal on the sidebar trigger. It has Escape/Enter handling but no focus trap/restore.

Initial outside-focus hypothesis is **insufficient**: parent then confirmed 23 projects (>6), typed successfully into filter, and saw its caret while AX still exposed only root HTML. A focus lifecycle repair remains appropriate accessibility hygiene, but cannot be claimed to fix this runtime finding. Next discriminating native check: compare a generic shared DetailModal in the same app/runtime, then compare Record DOM ancestry and WebKit AX caching/filtering. Owning Claude lane should preserve semantic dialog behavior rather than remove aria-modal to make automation work. Compare `components/common/DetailModal.tsx:197` focus/trap lifecycle. Do not remove modal semantics just to make CUA find controls. If focus does not resolve, inspect actual live DOM ancestry and WebKit AX independently; the present source does not support claiming an inert ancestry root cause.

## Why a web URL override is incomplete native isolation

- `ui/command-center/src/lib/api.ts:16`: VITE_DAEMON_URL (or VITE_API_BASE_URL) is a **build-time** API base. Native default localhost3001. Runtime env does not change bundled JS.
- `ui/desktop/src-tauri/tauri.conf.json:41`: connect-src only allows local3001 (plus remote services); staged3002 needs explicit compiled CSP addition.
- `ui/desktop/src-tauri/src/daemon.rs:42,108,309`: native detects/supervises/spawns fixed3001; spawned sidecar args override port env. Merely starting isolated3002 does not redirect shell.
- `daemon.rs:511`: get_daemon_token reads `$HOME/.permagent/secrets/daemon_token.json`, ignores PERMAGENT_PATH_ROOT.
- `activity.rs:12,63`: native activity token from HOME and POST fixed3001. Reusing real bearer in isolated daemon makes JS work on3002 but still allows native writes to live3001.
- `browser.rs:236`: inbox write fixed3001; `terminal.rs:21`: supervised output fixed3001.
- `main.rs:210`: heartbeat writes `$HOME/.permagent/logs/ui-heartbeat` independent of path root.
- `crates/goose/src/config/paths.rs:7`: standalone daemon Paths honor PERMAGENT_PATH_ROOT for config/data/state/DB/brain.
- `crates/goose-server/src/state.rs:1385`: daemon master token under Paths::data_dir()/secrets; isolated root generates an independent token.
- `crates/goose/src/config/base.rs:302`: PERMAGENT_DISABLE_KEYRING=1 required for fixture to avoid actual system keyring; file secrets then live in isolated config root.
- `crates/goose-server/src/main.rs:103`: daemon CLI accepts agent --host127.0.0.1 --port3002.

`PERMAGENT_CONFIG` and `PERMAGENT_SPECTRAL_DB` in agent.rs are startup observability fields; PERMAGENT_PATH_ROOT is the actual common path control. Do not assume those two alone isolate state.

## Proposed isolated daemon route (not launched)

Create a unique empty root outside ~/.permagent, ensure port3002 is free, and launch the newly staged daemon executable by exact path in an owned process group. Use a clean child environment with only required PATH/TMPDIR/system HOME plus `PERMAGENT_PATH_ROOT=<unique-root>`, `PERMAGENT_DISABLE_KEYRING=1`; do not inherit provider/service credentials. Args: `agent --host 127.0.0.1 --port 3002`. This isolates the Paths-backed stores and file-secret backend; it does not by itself prove every daemon background integration ignores host state. Fresh empty config plus no credentials avoids importing real projects or integrations. Keep its independently generated bearer; never copy the live bearer just to bypass shell routing.

For native acceptance, add a coordinated review runtime target/data-root contract for shell daemon supervision, token, activity, browser inbox, terminal output, and heartbeat, with matching Vite API base/CSP. No partial override can be called fully isolated. An isolated browser UI against3002 can test web/daemon flows, but cannot validate native mic/system IPC. A disposable macOS VM can validate unchanged native artifact without port/data collisions. Current reviewartifact with original HOME and only VITE/CSP changes **cannot** satisfy no-live-writes requirement.
