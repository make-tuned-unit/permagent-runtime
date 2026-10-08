# Shipping screen checklist — October 8, 2026

This is an independent reconciliation of existing receipts and navigation source, not a new CUA pass. Root is the sole UI operator. Every row below remains wholly or partly unverified. Record exact frontend/native build, backend/root, viewport, actions and observed result; refind the UI after each transition. Browser evidence does not establish native delivery. Use disposable fixtures for writes and a genuinely empty root for first-user acceptance.

## Current evidence boundary

- Root reports the latest full frontend run passed **3,927 tests / 486 files**. This is code validation, not screen acceptance.
- Root reports foreground Chrome scoped capture produced valid 45-second and 30.18-second tail WAVs; private receipt `chrome-dc232a2ce15f4f5e90458fc89ced392e/capture.json`. This supersedes older zero-PCM wording for that controlled capture only. This worker did not independently observe that run. Record UI → ASR → note → triage/cards remains open.
- Prior browser receipts establish selected Home focus/cancel flows, project lenses, empty Grow lenses, Build history bounds/browser fallback, some Settings labeling, shared-chat opening and compact World desk interactions. They do not establish every branch.
- Keep source changes distinct from the installed/retained native artifacts, which may predate them. Do not use a seeded review root as proof of pristine onboarding.

## Priority execution matrix

| Priority | Screen | Remaining CUA path and required outcome |
| --- | --- | --- |
| P0 | Record | Selected project → keyboard Cancel → Enter closes without capture. Inspect readiness checking/unavailable/retry; no-model state blocks Start truthfully. Later: save retry ownership, interrupted draft recovery and full tail/negations through selected project note, Brain, triage session and stable unexecuted cards. |
| P0 | Fresh installation | Empty isolated root → unpaired/expired pairing → no-credential/no-model onboarding → Decide later → skip folders/extras → Review → Start → restart. No personal defaults, invented readiness, automatic inference or repeated setup. Test failure/Retry/Continue anyway separately. |
| P0 | Empty Home | Every card with zero projects/sessions/memories/jobs: useful guidance, unavailable distinct from verified empty, no persistent spinner. Inspect narrow banner/actions and disconnected recovery. |
| P1 | Calendar | Unconnected compact/Day cannot claim No events. Focus view tabs: arrows/Home/End select view without paging dates. Event popover Escape/focus return; future today-only fallback; compact failed-read Retry; verified native empty and permission deny/retry. |
| P1 | Home layout | Scroll every card; Needs You detail/evidence/discussion then close. Customize/gallery/add-card draft Cancel preserves layout. Focus/collapse/Escape and reduced motion. Deliberate fixture layout save/reload remains separate. |
| P1 | Projects | Portfolio groups → project → Overview/goal → every Details panel → Kanban final columns/card → back/switch. Source note → triage session → authoritative Brain text/source/title. Disposable create/edit/move/delete and missing/deleted-source branches remain open. |
| P1 | People | Graph/List → search/filter → zero results → clear → person → relationships/project → return. Empty directory and disposable add/edit/merge/relation lifecycle remain open. |
| P1 | Build | Existing run detail/cost/council, split resize, pane/tab navigation. Native terminal/browser new/close/back/forward/reload and bounded run start/cancel remain unverified; browser desktop-required fallback has bounded prior evidence. |
| P1 | Grow | Actions/Results/Strategy/Calendar/Analytics; revisit Results, loading/error/retry. Prior receipt covers empty lenses. Populated action text/evidence, post/media/tracking detail, draft cancellation and fixture publication/execution remain open. |
| P1 | Finance | Full holdings/lots, household, watchlist, notes, currency and research-link inspection; optional enable/key dialogs then Cancel. No recent detailed receipt establishes this screen. Empty state and fixture editing remain open; no real trading implied. |
| P1 | Automate | Recent Activity/Automations/Learned/Installed at top and max scroll; chip selection accurate. Revisit Learned twice; inspect skill/proposal/history. Recurring/custom cron → One-time → Interval → Cancel; One-time explains local time without ineffective timezone. Fixture job lifecycle remains open. |
| P1 | Brain | Graph/List/search/clear; source/date zero-results must not claim whole Brain empty. Final-row clearance, full source/detail, person/project links. Authoritative saved-note read has browser evidence; latest human title/labels and filtered-empty changes need recheck. Fixture forget/undo remains open. |
| P1 | History | Sessions/Downloads/Activity/Spend: search/kind filters/sort/period/detail, zero results and bottom scrolling. Earlier receipt chiefly establishes opening tabs. Export/preview/local download and budget-editor Cancel need explicit receipts. |
| P1 | Chat / Voice | Spaces in composer cannot activate voice. Menus, draft retention, resize/close/reopen/narrow reachability. Native pop-out/return, attachment removal, bounded send/cancel/error and permission/no-device/retry remain open. Idle Voice click acquires microphone; it is not read-only. |
| P1 | World | Persist Day/Night/System; mounted reduced-motion response; desk-close focus; all Water prediction/reset branches; three brief types/export; complete bridges/collision routes; all agent movement controls; hidden/visible recovery; legacy campus. Prior compact profiles, four lessons, representative Evidence, Sit/Leave and shared-chat opening are bounded browser evidence only. |
| P1 | Settings | All 13 panes: Preferences, Persona, Agents, Memory, Autonomy, Tools, Models, API keys, Devices, Services, Appearance, Shortcuts, Privacy. Verify full scrolling, labels, masked credentials, unavailable states, footer controls and modal cancellation. Native delivery and fixture-only persisted changes remain separate. |
| P1 | Shell | Collapse/expand rail, focus/hover labels, workspace shortcuts; notification detail/return; rapid World↔other-tab changes; Record-picker→History. Selected navigation and actual content agree; focus remains reachable. |

## Work available without an inference runtime

Prioritize current-source Record keyboard/readiness, Calendar state/tab focus, Brain filtering/source title, Automate bottom selection/timezone/cancel, History filtering/idle Chat, full Finance read surface, and World preferences/walking. Fresh-root setup, backend writes and the complete captured-audio pipeline wait for their appropriate guarded environment; resource pressure is not permission to weaken guards.

## Sources

- [Full flow matrix](FLOW_MATRIX_2026-10-07.md): detailed subcases and historical evidence distinctions.
- [First-user Home/World checklist](FIRST_USER_HOME_WORLD_CUA_2026-10-08.md).
- [Browser receipts](CUA_BROWSER_2026-10-07.md).
- [Agent Desk receipt](AGENT_DESK_REDESIGN_2026-10-08.md).
- [Read-only batch](READONLY_CUA_BATCH_2026-10-08.md) and [project batch](ISOLATED_PROJECT_UI_BATCH_2026-10-08.md).
- Source inventory: `components/sidebar/Sidebar.tsx`, `components/settings/SettingsView.tsx`, `lib/store.ts` and named files in the matrix.

No new UI interaction, model/runtime operation, test execution or source edit was performed for this checklist.

## Native continuation reconciliation

[Native resume receipt](NATIVE_RECORD_RESUME_2026-10-08.json) and the final [shipping QA](SHIPPING_QA_2026-10-08.md) section add real native coverage for Record selection/cancellation/scope/permission denial, retained meeting source-to-Brain navigation, Calendar ranges/unqueried dates, World demonstration branches/brief export/night/reduced-motion navigation, and terminal/browser pop-outs. These supersede only those subcases above, not whole rows. Full native capture remains gated on Screen Recording permission; broader lifecycle/fresh-user/deeper Settings cases remain open. Recovery wording source fix passes82 targeted tests and is not embedded in the retained r4 stage.

### Read-only browser continuation, October 8 after18:02UTC

[Bounded evidence](READONLY_RESUME_2026-10-08.json): Brain Notes/source/date/empty state/last-row clearance, ranked search/date honesty/Graph, existing project-note provenance; Automate draft modes/focus/cancel/reset; History categories/search/empty/clear/last-row; Chat spaces/no Voice, clear/keyboard resize/close/reopen. No mutating actions or all-row completion. Search excerpts, native capture, full Settings/workflow matrix and release gates remain open.
