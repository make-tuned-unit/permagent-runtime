# Settings, Automate, Finance, Brain, and World flow audit

Date: 2026-10-18  
Workspace: `world-production-20261006`

This audit traced UI handlers to their API clients or navigation callbacks and exercised the selected routes with mocks. No live daemon writes, provider calls, model downloads, scanner starts, brokerage actions, wallet/RPC calls, or native app actions were made.

## Concrete World readiness defect

Compact World startup reported the whole stage ready only after `HabitatScene` had built terrain/collision **and** the separately loaded architecture and planting builders had both reported ready. The latter are detailed scene layers, so their loading time kept the full-screen “World is taking longer to load” / Retry panel over an already useful scene. The separate scenery health message already handles slow or failed detail assets.

`HabitatScene` now reports stage readiness once the walkable terrain and collision build exists. `ForumStageReady` still waits for the first active Canvas frame, and no longer waits for the optional architecture and planting layers. This keeps core readiness honest while preventing optional detail work from holding the stage overlay indefinitely. `ForumView.loading.test.tsx` now verifies the stage stays unready before the terrain/collision callback, then becomes ready and clears the overlay while detail builders have not reported completion.

## Settings pane inventory

The source inventory is `PANELS` in `SettingsView.tsx` plus the section resolver in `sections.ts`. There are 13 panes. `settingsReach.test.ts` verifies accepted keys resolve to real panes and retired keys route to their current home; that test checks routing, not that all 13 panes were mounted. Root also manually navigated all 13 panes in the native app and reported no generic load failures; no settings were edited. The full Settings test directory was executed separately (41 files, 373 tests passed), including the focused flows below.

| Pane key | Main interaction/API path | Executed evidence |
| --- | --- | --- |
| `agent` (Persona) | identity and provider configuration | Native route visit; `OnChainIdentitySection.test.tsx` (valid config writes and invalid slug writes nothing), provider/model and custom-provider tests |
| `preferences` | owner record / dev-root preferences | Native route visit; `SettingsView.owner.test.tsx`; preview-control reachability |
| `memory` | Brain feed source toggles and links | Native route visit; `MemoryFeedPanel.test.tsx` covers optimistic serialized writes/failure behavior; preview-control reachability |
| `autonomy` | trust mode config write | Native route visit; `SettingsView.autonomy.test.tsx` |
| `agents` | worker gates, agent settings, pending Librarian answers | Native route visit; `agents/AgentsPanel.test.tsx`, `agents/agentSettings.test.tsx`, `agents/LibrarianAwaiting.test.tsx` |
| `tools` | extension/tool availability | Native route visit; `ToolsPanel.canUse.test.tsx` |
| `models` | Chat/Harness/Voice route selection and local models | Native route visit; `SettingsView.models.test.tsx`, `LocalModelsSection.test.tsx`, `ProviderModelPicker.test.tsx` |
| `keys` | provider credential and secret-source controls | Native route visit; provider list/configuration and secret-source tests |
| `devices` | pair, rename, revoke, retry/reconnect | Native route visit; `SettingsView.devices.test.tsx` (7 tests) |
| `services` | Gmail and messaging gateway connect/disconnect | Native route visit; `GmailConnectSection.test.tsx`, `MessagingGatewaysSection.test.tsx`, provider removal test |
| `appearance` | theme/preferences surface | Native route visit; source pane is present, but no dedicated appearance interaction test was identified in the executed test list |
| `shortcuts` | shortcut settings surface | Native route visit; source pane is present, but no dedicated shortcut interaction test was identified in the executed test list |
| `privacy` | data boundary and analytics consent | Native route visit; `SettingsView.privacy.test.tsx`, `DataPanel.consent.test.tsx` |

`settingsWrites.test.ts` also freezes the Settings config keys and named write endpoints. It is source scanning rather than a user-event test. The component tests above exercise mock API callbacks; no live endpoint was called.

## Other audited surfaces

**Automate.** The main mutation routes are schedule `POST` actions for kill, pause/unpause, run-now, and deletion, plus saved-run finding recovery and finding review actions. `AutomateView.delete.test.tsx`, `AutomateView.resume.test.tsx`, `AutomateView.wiring.test.tsx`, `AutomateView.st1.test.tsx`, and `FindingsPanel.confirm.test.tsx` exercise row-level actions, immediate refresh after resume, inspect/run detail, finding recovery, and confirmation behavior. These tests stub `apiFetch`; no scheduler or model run was launched.

**Finance.** The UI writes config for opt-in state, API keys, and the user's Picker list; its visible operation controls call mocked `/api/finance/*` routes for scanner start/scan and Polybot start/pause, while watchlist/notes actions use DELETE routes. `FinanceView.test.tsx` covers the risk confirmation gate and Picker scan lifecycle, and `FundamentalsKey.test.tsx` / `PolybotKeys.test.tsx` cover key save/remove behavior. The Picks “View in World” action previously only switched workspaces, despite its tooltip pointing to the Financier panel. It now calls `focusWorldAgent('financier')`: the store verifies World is reachable, sets `pendingWorldAgent`, then `WorldView` consumes that ID, opens the Financier HUD, selects the agent, and switches to third-person camera. A regression asserts the Finance click calls that exact seam. Root's manual native check confirmed the old link opened World but did not check camera/panel targeting; the changed native behavior remains unverified. Financial execution remains an external boundary: tests prove frontend wiring and confirmation only, never a broker or real order. Root also verified the Finance Household/Holdings/Research tabs and that Record cancel closes the form without submission.

**Brain.** Search and graph/list selection are read-only routes; Forget is a delayed, undoable delete. `BrainView.search.test.tsx` exercises search selection/provenance, while `BrainList.forget.test.tsx` verifies click → undo window → one mocked delete, and no delete on undo or failed request. Root also verified native graph-to-List navigation with populated data. No live memory was removed.

**World.** `WorldView`/Forum entry and HUD controls route through host navigation callbacks, while agent actions such as Librarian Run Now use named API client methods. Executed World interaction coverage included `WorldHUD.test.tsx`, `useSelectedAgentProxy.test.tsx`, `ForumView.loading.test.tsx`, and `ForumView.lifecycle.test.tsx`. The lifecycle suite verifies hide/resume and walk/fly listener release; the loading suite verifies stage feedback, retry and now core-vs-detail readiness. In these tests Canvas, fetch, API calls, and world asset loading are mocked; this does not verify native GPU timing or live daemon responses.

## Verification and limits

- All Settings tests: 41 files, 373 tests passed.
- Focused cross-surface batch: 25 files, 198 tests passed.
- World readiness/loading/lifecycle after the readiness fix: 3 files, 12 tests passed. The direct `HabitatScene.readiness.test.tsx` mount proves no early ready callback before core terrain/collision, then exactly one callback once core exists while architecture and planting remain pending; this fails against the former all-builders gate.
- Final `npx tsc --noEmit` now fails at `components/projects/PersonDetailModal.tsx:50` because `useBrowserNavigate` is imported but unused. That file is outside this audit and was changed elsewhere in the shared worktree. An earlier typecheck in this task passed before that change appeared.
- Vitest logs a WebSocket “Port is already in use” warning in this shared environment, along with existing React `act` deprecation notices; the cited test runs still exited successfully.
- The full recursive test run across all five directories was started but stopped after it continued past 90 seconds. Counts above come from completed, bounded test commands, not from that incomplete run.
- Root observed that World Retry eventually recovered in the current app; the code-level regression and synthetic lifecycle check are the verification of the faster core-readiness path. No manual native verification was done after this change.
