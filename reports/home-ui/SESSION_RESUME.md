# Session resume — 2026-10-15

## Recovered work

Resumed the latest Claude session (`ff051541-e6d5-41d8-97f7-672ab026ef8e`, compacted from `a978c4a6-ebdf-4eaf-814e-5859f5a380a9`). The session had stopped at its usage limit. The Home redesign was unfinished in `ui/home-20261015`; the voice continuation and proactive memory prompt changes were in the World worktree.

## Consolidated source

Canonical branch: `world/production-20261006`, in `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`.

- `71477386`: recovered voice continuation and proactive memory prompt; prior focused validation passed 102 tests with one ignored, plus two delivery-tag tests.
- `d90278c8`: bounded local manifest discovery and the project stack endpoint.
- `e9212ab4`: focused Home overview, migrated exact defaults without writing layouts on read, preserved customization.
- `1642b211`: shorter Projects Overview, grouped Details, detected stack UI, fixtures and research/audit.
- `57d8e125`: broader command-center UX, compact subscription label/chevron, preserved centered People graph, loading/failure/draft/unread behavior and synthetic review fixture. Full UI suite: 564 files / 4,622 tests passed; TypeScript passed. Production build and backend gates are recorded separately in the UX DAG receipt.
- `f54d1a78`: stable Watcher evidence, terminal-task filtering, quiet-review memory and atomic metadata updates; authenticated route regressions plus the explicitly invoked port-3002 HTTP test. Corrected daemon library check passed; executed Rust/HTTP results remain separate gates in the receipt.

- `e2da46f6`: isolated browser acceptance harness uses actual production components and the test-owned daemon on port 3002. It checks persisted project/person edits; executed HTTP/browser gates passed; see the UX DAG receipt for exact logs and the interrupted repeat.

Home/UI work was tested in `/Users/j/Documents/dev/permagent-runtime/.agent-worktrees/home-ui-20261015`. Its Home and Projects changes were cherry-picked into the canonical branch. The canonical branch additionally retains the voice delivery-tag change. The main checkout and unrelated mobile work were not modified.

## Product decisions

Read `APP_UX_REVIEW_2026-10-15.md` for the complete page inventory, primary-source research and proposed structures. Every primary page and all 13 Settings panes are covered; source-only inspection is marked.

Preserve the People graph as the primary experience, with the user at the center. This is an explicit user preference. Improve its labels, selection, evidence and person inspector without replacing the graph with a CRM table.

Home and Projects were followed by the broader implementation in [the UX DAG](../ux-dag/README.md). Read its [verification receipt](../ux-dag/verification.md) for current page contracts, passing regressions and exact build/browser gates. All seven Watcher regressions passed in the full daemon library suite; authenticated stack discovery and real HTTP/browser persistence passed.

## Validation and preview

Read `verification.md` for exact gates and limitations. UI fixtures run with no daemon proxy and intercept every fetch/socket. The local preview server is on port 5199:

- `/ui/scripts/home-review.html`
- `/ui/scripts/project-review.html`

Supported fixture parameters include `width=480`, `theme=silver`, `state=empty`, `state=error`; Projects also supports `partial=1`. These are synthetic review surfaces, not the user's actual records. Restart with `node scripts/home-review-server.mjs` from `ui/command-center` if needed.

The canonical broader-page fixture uses port 5200 (`PERMAGENT_REVIEW_UI_PORT=5200 node scripts/home-review-server.mjs`) and `/ui/scripts/ux-review.html?page=people`. Supported pages include projects, people, automate, finance, actions, results, strategy, analytics, settings, brain and history. Settings accepts `section=privacy`. The same width/theme/state parameters apply. This fixture also denies writes; it is not staging acceptance.

## Preserved operating boundaries

No installation or deployment was performed. `/Applications/Permagent.app`, the live daemon on port 3001 and the live profile remain observe-only. Mutable acceptance work belongs in the established staging lane on port 3002. Do not send messages, start workers, record audio or change live settings while doing a visual review. The paused video work remains paused.

Use the established external build launcher and its disk guards, lane lock, one-job setting and one-hour watchdog. Do not recursively scan or glob the external build volume, weaken the guards, or kill unrelated processes. Build logs are referenced by exact filename in verification. Source reports contain no private-record screenshots; earlier local crawl material and generated logs were excluded from commits.

## Final verification (2026-10-16)

The final UI suite passed **564 files / 4,625 tests**. The daemon library suite passed **1,208 tests, 0 failed, 5 ignored**; four ignored tests need external model/voice fixtures and the isolated HTTP/browser test was run separately. Real HTTP/browser gates passed twice, including project-status and person-note persistence, detected stack, task/security navigation, Automate and History review. A repeat staging process became unresponsive and was stopped; a clean retry passed, but its root cause is unproven. The exact receipts and earlier guarded build interruptions are preserved in `../ux-dag/verification.md`.

The real People review exposed a sparse-network rendering bug: the world-space You sphere became enormous when the camera fitted one person. The hub now uses a fixed 36px screen-space marker consistent with the fitter, stays at the network origin and retains hover highlighting. Three regressions and a real browser visual check cover it.

The test artifact was compiled from Rust source at `8be925892f24bfd158a80722dc9292158c120d8a`, with no later Rust changes. The runner supports explicit artifact reuse with a current-revision/clean-Rust check. Read the receipt before reusing an artifact; the check does not independently establish artifact provenance. Synthetic and real staging preview servers on 5200/5201 were stopped after review. The final production UI build is recorded in the DAG receipt; no live installation was performed.

## Resume follow-up — 2026-10-18

Continued the unresolved staging reliability investigation without changing product code. Added `reports/ux-dag/probe_staging.py` and ten regression tests. The independent four-minute probe passed 714 reads (slowest 70 ms), both fixture edit readbacks, and the daemon's own final assertions. All 16 Python tests and the updated D00–D15 DAG completion check passed. Exact receipt: `../ux-dag/verification.md`, October 18 section. The previous browser hang remains unexplained; this sustained HTTP check did not reproduce it and is not a new browser acceptance pass. The live application remains unchanged.

## Cursor launch and desktop rebuild — 2026-10-18

The user reported that Cursor Grow actions require a delayed manual Send now and that the app had not been rebuilt. `5889056f` passes Cursor its directive as a safely quoted initial positional argument, retains Grow/Build metadata and avoids duplicate follow-up paste. Follow-up delivery for other harnesses now waits for native write acknowledgement and preserves explicit retry after failure. Final checks: 4,635 UI tests, 90 focused Grow/terminal tests, TypeScript, and real fake-executable zsh PTY argument transport passed. No real Cursor model request was made.

D16 and D17 are verified. The complete signed app is `/Volumes/PermagentBuilds/permagent/targets/world-production-20261006/artifacts/Permagent-227484e71-20261018-104250.app`, built from `227484e71`. The October 16 build was a frontend build. `reports/ux-dag/rebuild_desktop.py` now orchestrates guarded backend/staging/app/verification with separate backend and UI source provenance, strict signatures and resource/code checks. Its supporting Python suite has 27 passing tests. Read `../ux-dag/verification.md` and the generated desktop manifest for final status; do not equate a running build with a finished app. The initial release build log is `/Volumes/PermagentBuilds/permagent/logs/20261018-080825-79769.log`, from backend source `7620c68f`; the successful third backend attempt is `20261018-100911-75414.log`, and the full Tauri app build is `20261018-103701-86795.log`, both with successful adjacent receipts. Deep strict signatures, resource/code provenance and isolated bundled version smoke passed. No installation is authorized by this build phase.
