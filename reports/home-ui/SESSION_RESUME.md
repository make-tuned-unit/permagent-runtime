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


## Authorized dogfood installation — 2026-10-18

The user subsequently authorized installation. D18 is verified: `/Applications/Permagent.app` now contains build `227484e71` and is open. The existing LaunchAgent was restarted with unchanged configuration; `/status` is 200 and `/api/version` reports `227484e71`. Native Home loaded with hub connected. Previous app retained at `/Applications/.Permagent-rollback-20261018-114756.app`; its executable hashes match the baseline. Read `../ux-dag/desktop-install-receipt.json` for exact installation evidence. The earlier observe-only installation boundary was superseded for this explicitly requested update; it does not authorize unrelated messages, settings changes, recordings or test work against live records.


## Dogfood layout corrections installed — 2026-10-18

`55d5ae29` addresses the user's installed-app screenshots: the actual daemon morning template now migrates in memory to the five-card Home overview while preserving custom layouts; Projects uses aligned responsive cards with internal status footers and filters; live Analytics chart/metrics lead the tab; People opens in Graph even with an old List preference. Three cheaper agents assisted implementation/review; root performed integration and installation.

Full UI regressions passed **564 files /4,643 tests** and TypeScript passed. Synthetic wide/narrow review and fresh native Home/Projects/People/Analytics checks passed. A source-policy test initially rejected a native tag title; using the shared Tooltip resolved it before the complete rerun. The full signed app was rebuilt from `55d5ae293` with the verified unchanged `227484e71` backend. Exact provenance, build receipts and initial/retried version smoke are in `../ux-dag/verification.md` and the updated manifest.

The new app is installed and open on Home at `/Applications/Permagent.app`. Rollback `/Applications/.Permagent-rollback-20261018-121559.app` retains the immediately previous version; the earlier rollback also remains. LaunchAgent configuration is unchanged; health and installed signatures/hashes passed. D00–D23 are verified. See `../ux-dag/desktop-dogfood-layout-install-receipt.json`. Review tab/server were closed. Existing live-action boundaries remain; no real Cursor action, messages, settings changes or recordings were executed. Four unrelated untracked October15 release files were left untouched.


## Functional wiring audit installed — 2026-10-18

User requested functional testing of buttons and flows. Three cheaper agents inventoried and exercised primary surfaces; root integrated fixes, ran real isolated browser persistence, reviewed the native app, and installed the update. Product commit `68e1dfd8d` fixes notification destination acknowledgement and unavailable profile-link guidance, Finance's Financier focus route, and World readiness gated by core terrain/collision. History gained explicit tabpanel semantics; its earlier intermittent native AX omission has no proven root cause.

Final UI suite: **568 files / 4,657 tests passed**; TypeScript passed. Reused unchanged-source daemon artifact: **1,208 passed, 5 ignored**, plus the explicit isolated HTTP/browser test passed separately. Project status and person notes persisted through UI save/reload and independent readbacks; Details detected React/Node.js from the codebase manifest. The remaining four ignored tests need external model/voice fixtures.

App/UI source `68e1dfd8d575551add47f90f756159c1619977b4` is installed at `/Applications/Permagent.app`, reusing verified backend `227484e71`. Build/signatures/resource provenance/version smoke and installed health passed. Artifact copy initially hit the unchanged internal 12 GiB floor; after recovery, a guarded continuation finalized the already compiled bundle and normal verification passed. Rollback `/Applications/.Permagent-rollback-20261018-191835.app` retains prior55d app. LaunchAgent configuration is unchanged.

Native post-install checks: Home connected; Finance Research → World opened The Financier desk and focused camera; World loading cleared without Retry; all four History tabs exposed named panels and rendered; all five Grow tabs opened; People opened Graph; Projects rendered. App remains open on Projects with dark theme/collapsed sidebar. D00–D28 are verified within their stated criteria. Full matrix, exact logs and limits: `../flow-audit-20261018/README.md`, `../flow-audit-20261018/desktop-install-receipt.json`, updated desktop manifest. No live model/harness request, outbound message/publication, recording, financial execution or credential/pairing change was tested. Do not claim exhaustive perfection or that the intermittent AX issue was conclusively fixed. Four unrelated untracked October15 release files remain untouched.


## Home geometry and Calendar refinement installed — 2026-10-18

User reported card shrink/gaps after Home Customize → Done and requested a left-aligned Calendar date with Day/Week/Month on the right, backed by deeper design research. Product commit `3c1c9c4e93cf3b0a011ad27d25133c070fadfaf1` uses the same saved card dimensions/placement in both modes, including custom layouts, while retaining separate focus presentation. Calendar now has an aligned date/view header, equal-width direct launch controls, a separate full-view period navigation row, and scrollable compact agenda/access content. Empty-state duplication was removed.

Apple Layout, Segmented Controls, Tab Views, Design Principles, Dark Mode/Color, Scroll Views, and Calendar guidance informed the documented decisions. Wide dark and narrow silver synthetic render review passed, including actual DOM geometry comparisons across both modes, all three Calendar launch paths, and event details. The final UI suite passed **568 files / 4,663 tests**; TypeScript and diff checks passed. D29–D31 are verified. See `../ux-dag/home-grid-calendar-20261018.md`. The completed app build used the existing external guard and verified unchanged backend 227; no backend rebuild.

The 3c1c app is installed and open on Home, hub connected. Native Customize/Done parity and all three Calendar launch paths passed. Deep signatures, UI/resource provenance, installed hashes/health and rollback hashes passed; LaunchAgent unchanged. Rollback: `/Applications/.Permagent-rollback-20261018-200312.app`. Two external-volume version probes timed out before application code (sample `_dyld_start`); cause unproven. After guarded staging, both isolated local version probes passed (daemon 5.65s, CLI 0.04s), and installed runtime checks passed. See `../ux-dag/home-grid-calendar-install-receipt.json`, `../ux-dag/home-grid-calendar-version-smoke.json`, and updated rebuild manifest. No calendar event writes or permission changes were tested. Owned review browser tabs/server were closed; unrelated October15 files remain untouched.
