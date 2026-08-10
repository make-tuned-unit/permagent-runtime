# Orchestrator observability wave outcome

## Task 1 — execution receipt output timestamps

- Replaced `wait_with_output` with concurrent streaming reads of external-worker stdout and stderr.
- Preserved the complete stdout/stderr byte buffers used by existing evidence, redaction, summary, and error-tail logic.
- Added timestamped stdout progress events. The orchestrator stamps `first_output_at` once and refreshes `last_heartbeat_at` on every read through the existing execution-receipt metadata path.
- Queued events are drained before receipt finalization so fast workers cannot finish before their first-output stamp is persisted.
- Added a shell-backed streaming regression test that prints, sleeps, and prints again; it verifies the receipt is stamped while the child is still running and that the full stdout remains `firstsecond`.

Commit: `feat(orchestrator): stream worker output into receipts`

## Task 2 — bounded provider check

- Traced Moonshot construction through the declarative OpenAI provider into `Config::get_secret` / `Config::all_secrets`.
- Documented the hang mechanism: the synchronous OS-keyring read can wedge while holding the secrets-cache mutex, so an ordinary async timeout cannot preempt it.
- Moved the complete provider construction onto Tokio's blocking pool and bounded the HTTP handler at 15 seconds.
- Timeout and task-failure responses are clean JSON errors that name the requested provider.
- Added a regression test using genuinely blocking work to prove the handler returns before that work completes.

Commit: `fix(server): bound provider configuration checks`

## Task 3 — shared session picker

- Extracted the pop-out toolbar dropdown into `components/chat/SessionPicker.tsx`.
- Rendered it in both `ChatApp` and the docked `ChatView` header.
- Preserved session loading, click-outside close, new-session creation, `switchToSession`, active-session styling, message counts, and relative timestamps.
- Kept the picker header rows unclipped so the absolute dropdown panel remains visible.
- Added a `ChatView` mount test asserting the shared picker is present.

Commit: `feat(chat): share session picker across chat views`

## Gates

- `cargo fmt --all -- --check` — **passed**.
- `cargo test --locked -p permagent --lib -- goal_engine::` — **passed**: 27 passed, 0 failed, including `streaming_output_reports_first_bytes_before_child_exit`.
- `cargo clippy --locked -p permagent -p permagent-daemon --lib -- -D warnings` — **environment-blocked**. The first attempt could not download the `rusty_v8` archive. After the focused test populated more build artifacts, the retry advanced further but stopped in `sherpa-onnx-sys` because DNS/network access to its GitHub native-library archive is unavailable. Clippy did not reach project diagnostics.
- `cd ui/command-center && npx tsc --noEmit && npx vitest run src/components/chat` — **environment-blocked** before TypeScript started. This worktree has no `node_modules`; `npx` attempted to resolve `tsc` from the npm registry and failed with `ENOTFOUND`. An offline `npm ci --offline --ignore-scripts` was also attempted, but the local cache lacks `zustand-5.0.12.tgz`, so neither TypeScript nor Vitest could run.

Per the brief, `cargo test -p permagent-daemon` was not run.
