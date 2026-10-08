# Agent app knowledge and safe navigation

Source implementation only; current native and daemon acceptance remain pending. No tools were enabled and no permission, capture or execution policy was broadened.

## Existing capability and actual gaps

`SelfKnowledgeBuilder` in `crates/goose/src/agents/self_knowledge/mod.rs` assembles the main agent's capability brief each turn. Explicitly scoped coding sessions keep their smaller inventory. The daemon also injects its catalog from `crates/goose-server/src/app_catalog/catalog.yaml`. `app_conductor` emits UI requests; `app_perception::observe_app` reads privacy-redacted domain stores.

The catalog has stale naming: Dashboard is labeled Home in the UI, and the former Settings record pages are now History. `observe_app` is not a current-window snapshot and has no navigation acknowledgment. World observation is the worker roster, not camera position. Some component-local states had no navigation tool route. These gaps prevent a claim that the agent already has complete app knowledge or universal UI control.

## Implemented contract

| Existing tool | Request | Effect |
|---|---|---|
| `navigate_app` | `tab: Home` or `Dashboard`; optional `section: overview` or bundled card type | Existing Dashboard route; overview collapses focus, card type focuses its first visible instance |
| `navigate_app` | `tab: History` | Existing Settings overlay route with `section: history`; frontend redirects to History |
| `navigate_app` | `tab: Settings`, `section: models` | Existing Models pane route; no selection/default change |
| `navigate_app` | `tab: Projects`, resolved `state.project_id`, `section: overview/details/kanban` | Select the project and the requested view |
| `navigate_app` | `tab: World`, one of `commons`, `gallery`, `maker`, `council_garden`, `observatory_garden`, `conservatory` as `section` | Select that place in the compact habitat, returning from campus if needed |
| `app_action` | `surface: record`, `action: open` | Open the source/project picker, or reveal the active recording panel; never Start/Stop/Save/Discard |
| `open_item` | `kind: card`, current `project_id` and `card_id` | Existing `openCardOnBoard` seam; no card creation or execution |

Project lenses and World places are allowlisted on both sides. `appNavigationRequests.ts` keeps pending requests until the receiving view can consume them; consuming an old request cannot erase a newer one. Project requests wait for the matching project, and World requests wait for a visible compact view. Record opens through the existing replay-gated action channel; replayed requests and a made-up `record/start` action do not open it.

Navigation and action tool results now say the request was sent and screen delivery is unconfirmed. They are not action receipts. The new `self_knowledge/app_navigation.md` is embedded in the unscoped main-agent brief; the scoped-session boundary remains. It documents routes, model settings, project notes, Record triage status, World demonstrations and remaining limits. The meeting descriptor now reflects selectable capture sources and separate save/triage states rather than promising microphone capture and completed cards unconditionally.

## Validation

- `npx tsc --noEmit`: exit 0.
- Focused Vitest: **47/47 passed** across `useAppNavigate.sections`, `useAppNavigate.openItem`, `useAppNavigate.replay`, `ProjectWorkspace.navigation`, and `MeetingRecorder.focus`.
- Tests exercise every project lens and World place, pin the place list to actual `HABITAT_PLACES` and Rust allowlists, test delayed project mounting and stale-request consumption, Models/History routing, card navigation, replay blocking, and rendered Record picker opening without `getUserMedia`.
- Existing Tooltip `act()` warnings and the shared Vite port warning appeared; the test command exited 0.
- Rust formatting and scoped `git diff --check` passed. No Cargo or native build was run in this lane.

Backend tests added in `app_conductor` still need execution: aliases, project-id requirements, World rejection, Record open-only actions and truthful result text. The new guide and meeting descriptor deliberately change unscoped prompt snapshots (`test_basic`, `test_one_extension`, `test_typical_setup`, `test_all_platform_extensions`); refresh and inspect those during the next authorized backend test run. Native CUA must confirm these exact routes on the rebuilt artifact before they become runtime acceptance.

## Remaining work

Home Focus is integrated at source level; native acceptance remains pending. Its route waits for saved-layout loading and resolves a visible card type to the actual card id. An absent/hidden card or active Customize mode produces a local status notice without creating cards, writing the layout or exiting Customize. Project note-id and triage-session-id deep links remain UI buttons, not agent tool kinds. No generic screenshot/focus/modal observation or delivery acknowledgment was added. Agent actions must continue to distinguish source-defined routes, successful queued requests, rendered UI and verified persisted outcomes.


## Home Focus follow-up

Source route: `navigate_app` with `tab: "Home"` (or legacy `"Dashboard"`) and `section: "overview"` to collapse, or one of these **28 bundled card types** to focus:

- Code cards: `decisions`, `stats`, `in_flight`, `recent`, `timeline`, `todos`, `growth_results`, `council`, `calendar`, `today_focus`, `overnight_digest`, `fleet_kpis`, `stalled_pattern`, `project_health_grid`.
- Additional built-in manifests: `system_stats`, `weather`, `henry_status`, `job_health`, `finance_summary`, `finance_watchlist`, `cost_vs_outcome`, `people_attention`, `open_commitments`, `recent_meetings`, `week_headline`, `week_small_multiples`, `memory_growth`, `on_this_week`.

`useAppNavigate.sections.test.ts` verifies this is the exact union of the actual `CARD_REGISTRY` keys and daemon `builtin_card_manifests`, and that the Rust allowlist matches. Runtime third-party manifest types are excluded. The new `layoutLoaded` readout in `useLayout` prevents a request consuming against initial defaults before saved layout loading settles; it changes no persistence behavior.

The Home follow-up focused run passed **132/132 tests** across Dashboard focus (15), navigation sections (48), replay (19), and existing layout logic (50). These overlap the earlier 47-test run and are not additive coverage. Rendered tests cover real card-id resolution, collapse, deferred loading, hidden-card notice and leaving Customize untouched. Native routing and backend tests are still pending.

A separate readiness-hook run passed **2/2 tests**, covering pending saved-layout fetch and failure fallback. TypeScript after the Home wiring exited 0. Scoped diff checks passed. Source is frozen for the coordinator build.


## Independent Home interaction review

Concrete frontend findings and fixes (no UI control, Rust edits or heavy builds in this review):

- **Delayed transition focus:** `requestAnimationFrame` was scheduled before the browser's asynchronous view-transition update, so the new Collapse/Expand control could be absent when focus was attempted. Focus now schedules inside the update callback. A rendered regression deliberately delays that callback past a frame and verifies focus after both expand and collapse.
- **Overlapping transitions:** finishing an older transition could clear the newer transition's styling marker. Cleanup now checks ownership; a deferred-promise regression covers this.
- **Navigation waiting:** registered code cards and overview collapse unnecessarily waited for unrelated manifest loading. They now act when their own prerequisites are ready. A missing manifest still waits and reports unavailability on failure. Customize rejection takes precedence over waiting for a manifest.
- **Header geometry:** the 26-pixel Expand control at the top-right overlapped the area occupied by right-aligned header metadata. Card heading/Calendar header now reserve a 28-pixel gutter only where an Expand control exists. FocusBar can wrap long labels while preserving its Collapse button. These are source-based geometry fixes; actual narrow-window spacing still needs native visual acceptance.

Focused review run: **27/27 tests passed** (Dashboard focus 18, transition handling 3, Calendar on Home 6). Tests also cover reduced-motion bypass and a failing transition engine. Existing scoped navigation tests remain distinct from native rendered acceptance.


## Browser CUA follow-up: card continuity and Council loading

The coordinator observed repeated Calendar “Checking your calendar…” after expanding and collapsing Home. Source inspection confirmed that Dashboard conditionally inserted a wrapper around the focused card, remounting it and discarding local state. Dashboard now keeps one wrapper and child position across all presentations. Outside focus, `display: contents` preserves the original card box and percentage-height relationship. The masonry ResizeObserver still measures the real outer cell, never the boxless wrapper; focused flex sizing remains unchanged.

A regression checks component mount/unmount counts, local counters and an input draft through expand, collapse, re-expand and focusing another card. Test registry component identities are stable, matching the production registry. Calendar's main range still intentionally pauses in compact mode and can revalidate when expanded; its ahead/Next-up hook no longer remounts merely because presentation changed.

The coordinator also observed Council loading for several minutes in the React StrictMode browser. Council is a first-party code card, despite the older manifest description. Its cleanup-only effect set `live.current = false` and never reset it when StrictMode replayed setup, so both success and failure responses were ignored. Setup now resets the flag. Two rendered StrictMode regressions cover a completed response and a rejected request.

Validation: **32/32 focused tests passed** (Dashboard focus 19, Calendar Home 6, Council 7); `npx tsc --noEmit` exited 0. No backend changes or builds ran. Browser CUA re-verification and refreshed native artifact acceptance remain coordinator work.

Calendar latency is separate from remounting: the daemon today-only endpoint awaits two sequential calendar reads with eight-second timeouts, plus database work. The frontend has no total request deadline. The observed future-date view eventually resolved to truthful “not checked” messaging; no infinite Calendar request was established and no speculative timeout change was made.
