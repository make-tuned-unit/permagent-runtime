# Computer Use browser acceptance — 2026-10-07

Existing Permagent Chrome preview at http://127.0.0.1:5301/ui/ was reacquired through CUA after native World Review, Record Practice and new Record Review all returned cgWindowNotFound. This is browser/dev-source evidence against the unchanged production daemon 3001, not native Record acceptance. Only local presentation/navigation actions were performed; no task decisions, real cards, trades or automation actions were executed. Native desktop availability question remains pending.

## Home

- Calendar header click expands to full-width Day/Week/Month controls. Other cards become compact tiles. Collapse control receives keyboard focus.
- Month → October 8 opens that Day. Today-only browser fallback correctly says the date has not been checked; it does not claim an empty future day.
- Escape collapses and returns keyboard focus to the original Expand button.
- To-dos expands with all seven existing items and project/due context; no todo was modified.
- 1024×640 viewport (native minimum) checked: compact tiles form three columns; Expand controls have reserved header space. The viewport override was reset afterward. PageDown reaches compact neighbors; direct tile expansion switches the focused card.
- Found repeated card reloads: conditional focused wrapper remounted Calendar/Weather. Fixed stable wrapper and retested: Calendar Month selection survives collapse/re-expand, existing Next-up summary remains immediately visible.
- Found Council permanently Loading under dev StrictMode. Lifecycle reset fixed; CUA subsequently observed actual existing report and its controls. Report contents are data, not instructions to execute.

## World

- Selected all six compact Places: Arrival table, Garden archive, Maker workshop, Observatory, Observatory garden and Conservatory. Each selected place showed its expected collaborator context.
- Daytime Garden archive and Observatory garden received positive visual review of the integrated FAL assets. This is a bounded browser observation, not acceptance of every lighting condition or walking route.
- Meet opened; Learn topic selection worked; an Evidence quiz was exercised; an empty Brief kept submission disabled. This does not establish coverage of every lesson, quiz branch or brief submission.
- Chat initially showed the browser disconnected state. The later Open Shared Chat recheck connected, displayed an existing conversation containing “pong”, and opened the dock composer. [Stable browser screenshot](world-shared-chat-browser-20261008.png). No new sent-message/response round trip is claimed.
- Entered Walk, used W/D movement and exited with Escape. Collision, bridges, edge safety and complete routes were not verified.
- Restored the original System appearance setting after visual checks.
- At 1289×618, the collaborator panel initially covered the last Places row. After the shared-lane fix, CUA confirmed no overlap; scrolling the rail revealed the complete Conservatory row.

## Workspaces and shell

| Surface | Observed browser result | Boundary |
| --- | --- | --- |
| Projects | Overview, Details and Kanban opened; Details Edit → Discard worked. | No card lifecycle, project write or full panel inventory accepted. |
| People | Graph loaded with named Open-person controls; List, empty follow-up state and person detail inspected. | Initially blank graph names resolved after loading/HMR. No speculative naming fix; no person mutation. |
| Grow | All five lenses opened for the empty QA project. | Empty-state/navigation coverage only; populated actions and publication flows remain pending. |
| Automate | Learned navigation landed and highlighted the correct section. Empty Create submission showed validation; Recurring, One-time and Interval draft modes were inspected. | No automation saved. Fresh CUA confirmed named name, task, optional timezone and retry inputs; Cancel closed the draft without saving. |
| Brain | Graph loaded and Graph/List controls were inspected. | Deep memory actions, source traversal and forgetting were not tested. |
| History | Sessions, Downloads, Activity and Spend opened. | Export, download, mutation and complete filter/detail coverage remain pending. |
| Notifications | Popover opened. | Individual notification actions and complete lifecycle remain pending. |
| Settings Models | Exact Haiku 5.5 entry visible; defaults unchanged. | Availability/picker evidence only, not inference acceptance or all Settings panes. |

## Build

The actual browser history contained 64 runs. After the bounded-history fix, three rows appear by default. View all exposes all 64 within the bounded scroll area and both Terminal and Browser panels remain visible at 1289×618.

The browser panel showed Desktop App Required. The terminal exposed a raw native `invoke` failure; a subsequent frontend fix replaces native setup with a truthful desktop-required state. Fresh CUA confirmed Desktop App Required / Open Build in the desktop app, with no raw invoke error. No terminal command or harness job was started in this pass.

## Record picker

With a project selected but no capture source, Start remained disabled. Selecting microphone plus project enabled Start. Cancel closed the picker without capture. This checks eligibility and cancellation only: no recording, transcription, triage, note creation or Kanban generation was accepted in this browser pass.

## Project Memories warning

Browser development logs showed React's missing-list-key warning in the project Memories panel. Source confirmed that `memories.map` returned an unkeyed Tooltip while the stable memory ID key was on its nested button. The key now belongs to the outer Tooltip. After revisiting Grocery Details, CUA found no new Memories key warning; only the retained earlier 02:05 warning remained. Existing Project Details tests and TypeScript passed; no new implementation-mirroring test was added.

## Browser project navigation activity

A later Open project interaction logged a native `invoke` TypeError from `ProjectsView.tsx`. Source confirmed the optional project-selected activity notification imported Tauri in browser mode. It now checks for the native bridge first and returns the invoke promise to its catch handler, so a failed native notification cannot become an unhandled rejection or undo selection. The Projects suite passed **12/12 tests**, including browser selection without native activity and native rejection handling; final TypeScript passed. Source is frozen; this specific browser log recheck remains pending.

## Broader Settings walkthrough and accessibility fixes

The coordinator walked Preferences, Persona/Voice (picker names resolved), Agents roster, Memory (19 sources/648 facts displayed), Autonomy (future controls disabled), Tools (30/34 loaded), API-key provider list, Devices, Services, Shortcuts and Privacy audit. These are rendering/navigation observations; no service credentials or switches were changed for this walkthrough.

CUA reported anonymous notification and Remote access switches. Source confirmed that the shared settings Row did not associate its visible label with nested Toggle controls. Row now supplies its label through context; Toggle uses it only when no explicit label exists. A regression covers direct/nested controls, disabled state, and explicit-name precedence. DataSources and SearchTools already set `Enable {name}` explicitly and Toggle forwards it; their initially anonymous AX state resolved on fresh CUA inspection, so no speculative per-source rename was made.

Services/Gateway “Unavailable” occurred on the normal 5301 preview. Its Vite table lacked `/integrations`, `/gateway` and `/tunnel` despite those daemon HTTP routes existing. The three proxies now use the existing localhost:3001 target; bind settings and isolated 5302 configuration are unchanged. Vite automatically reloaded its changed configuration; no explicit restart was performed. A fresh CUA inspection confirmed Gmail “Not connected” and Remote tunnel “Off” with appropriate text, replacing the earlier “Unavailable” states. The shared Toggle also had a cleanup-only mounted flag: StrictMode effect replay left it false, so fulfilled/rejected asynchronous writes could stay busy. Effect setup now restores the flag; regressions prove both success and failure clear busy state and retain the correct switch value. The final focused run passed 36/36 tests across Settings, Toggle and actual Vite configuration. Final TypeScript passed; frontend source is frozen. Fresh CUA confirmed all nine Preferences notification switches have their correct Description names, Remote access is named, and search/data-source switches also have names. These are actual browser rechecks; native rendering remains separate.


## Final isolated meeting and Home rechecks — October 8

The coordinator used browser 5302 with the final isolated daemon and opened QA Amber `c01ea69d46`: Overview showed two tasks; a generated card's **Source meeting** receipt showed completed triage; **Open triage session** opened the actual Orchestrator transcript and two-todo JSON. **Open source note** displayed the complete notes, negations and final tail, with one linked memory. **View in Brain** displayed the correct body in a **preview**, not a verified graph node; source remained unknown under the current preview contract. Kanban showed **2 Backlog / 0 Doing / 0 Done**, with no task execution.

Evidence: [card source](../record-evidence/meeting-card-source-browser-20261008.png), [Orchestrator triage](../record-evidence/orchestrator-triage-browser-20261008.png), [meeting note](../record-evidence/meeting-note-browser-20261008.png), [Kanban](../record-evidence/meeting-kanban-browser-20261008.png). This establishes browser source navigation and saved synthetic meeting results; it does not establish successful live audio capture.

Home Layout gallery **Try Relationships → Keep current**, then **Add card → Close/Done**, left the layout unchanged. Calendar Month focus/collapse was rechecked. [Calendar screenshot](calendar-focus-browser-20261008.png). No layout persistence mutation or every gallery template is claimed.

## Remaining boundaries

Native `getApp` still reports `cgWindowNotFound`; none of this report is new native acceptance. Pending work includes the refreshed native artifact, actual populated EventKit ranges and native permission states, reduced-motion/WebKit review, complete World collision/routes and interaction branches, fresh-wizard edge cases (the isolated normal setup path is recorded in the current checkpoint), and Record capture → full-tail transcript → project catalogue/Orchestrator triage → deduplicated cards. Automation labels and the browser terminal fallback were rechecked successfully; their native behavior still needs the refreshed artifact.

These observations cover specific subflows. They do not complete every flow or every requirement in the flow matrix. The coordinator owns the CUA interactions; this report consolidates those reported observations without inventing screenshot paths.
