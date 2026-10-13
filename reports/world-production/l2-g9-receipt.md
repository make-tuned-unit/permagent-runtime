L2 / G9 — Rust shell startup responsiveness, 2026-10-13

Reviewed `wq-reports/qa/native-gate-20261013.md` (“Stalled-daemon recovery test”) and `recovery-11s.png` / `recovery-late.png`. The 11-second capture still shows the frontend's full 10-second countdown.

Changed `ui/desktop/src-tauri/src/main.rs` and `ui/desktop/src-tauri/src/daemon.rs` only, plus this receipt and `reports/world-production/l2-g9.patch`. Unit tests are inline in `daemon.rs`. No frontend files, Git index, or commits were touched; no cargo command was run.

- **Moved:** setup explicitly shows the main window and returns without starting/waiting for the daemon. `RunEvent::Ready` starts a named background thread that runs the existing daemon selection, spawn, and health waits. The event loop can paint the boot screen while those waits run; the shell no longer adds its 10/20-second wait before the frontend deadline begins.
- **Preserved:** prefer an existing daemon; launchd plist means wait-only; bundled sidecar fallback and external wait when spawning fails; review mode never consults production launchd or spawns. External/spawned waits remain 10/20 seconds. Supervision keeps five consecutive restarts, 1/2/5/15/30-second backoff, and a 120-second healthy reset. Logging, stderr tails, leaving a slow child running, and `RunEvent::Exit` SIGTERM → two-second grace → SIGKILL are retained.
- **Failure reporting:** `surface_daemon_failure` logs and emits `daemon-failure` with the detail string. It no longer changes the native title or touches AppKit chrome; no dialog is introduced. The existing boot/retry screen remains the user-facing status. The event is best-effort and has no new frontend subscriber in this patch; stderr retains the detail if no listener is present.
- **Races:** `get_daemon_token` polls a missing file for at most one second on the blocking pool, then returns “Daemon token not yet available … Retry shortly” through the existing result contract. Existing frontend failure handling permits a later retry. Permission/parse failures retain their actual diagnostics; native synchronous readers remain immediate. Exit cancels waits, and the child mutex covers spawn plus publication so shutdown either prevents spawning or takes the child for cleanup. Termination clears only the matching child before notifying supervision.

Validation passed: `rustfmt --edition 2021 --check --config skip_children=true ui/desktop/src-tauri/src/main.rs ui/desktop/src-tauri/src/daemon.rs`, scoped `git diff --check`, and source assertions for setup/Ready ordering, worker dispatch, unchanged lifecycle constants/mode order, shutdown locking, event-only failure reporting, and token worker dispatch. Four token tests cover immediate availability, creation during a request, timeout followed by recovery, and real parse/I/O errors. Rust unit tests and compilation are **not run**, per the reserved build lock.

Coordinator, from the worktree root:

```sh
cargo check --manifest-path ui/desktop/src-tauri/Cargo.toml --no-default-features
cargo test --manifest-path ui/desktop/src-tauri/Cargo.toml --no-default-features daemon::tests
```

Remaining validation: rebuild and repeat the no-daemon native gate, including quitting during startup and Retry after a late daemon/token appears. This removes the measured shell delay; absolute launch-to-recovery timing, including WebKit/frontend loading, still needs measurement on the rebuilt candidate.
