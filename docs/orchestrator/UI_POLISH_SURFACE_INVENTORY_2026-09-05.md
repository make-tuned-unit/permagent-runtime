# UI polish surface inventory — 2026-09-05

Status: source-of-truth inventory for planning. This is not a visual-audit pass,
does not certify visual polish, and does not certify device/native behaviour.
Rows are derived from the current router, store, component, and native menu
sources. Owner is intentionally `TBD` for the follow-on polish DAG. This is a
bounded source inventory, not yet a claim that no hidden or newly added route
exists; the unresolved-coverage list at the end is part of the ledger.

## Contract for a complete surface

Every discoverable control must resolve to a callable backend seam, or be
disabled with an honest reason and an available recovery path. Every backend
capability intended for a user must have a discoverable surface; a capability
that is deliberately headless must be documented as such. A surface is not
complete until its loading, empty, unavailable/offline, permission-denied,
error, active, cancellation, and success states are distinguishable where
those states are possible. A disabled button is not a substitute for an
unwired button. No visual review may be marked passed from this inventory.

Shared state truth is `ui/command-center/src/lib/store.ts`; workspace-to-tool
rendering is `components/workspaces/WorkspaceRenderer.tsx`; settings routing is
`components/settings/sections.ts` plus `SettingsView.tsx`; native menu truth is
`ui/desktop/src-tauri/src/menu.rs`; mobile tab truth is
`ios/PermagentMobile/PermagentMobile/PermagentApp.swift`.

<!-- Machine-readable lower-bound coverage anchors. Keep these in sync with
     source-defined routes and stable inventory IDs; the Node checker below
     intentionally fails closed when either declaration or anchor parsing fails.
ui-polish-route tool="chat" surface="CC-CHAT"
ui-polish-route tool="skills" surface="CC-SKILLS"
ui-polish-route tool="trace" surface="CC-TRACE"
ui-polish-route tool="world" surface="CC-WORLD"
ui-polish-route tool="terminal" surface="CC-TERMINAL"
ui-polish-route tool="browser" surface="CC-BROWSER"
ui-polish-route tool="memory" surface="CC-BRAIN"
ui-polish-route tool="dashboard" surface="CC-HOME"
ui-polish-route tool="build" surface="CC-BUILD"
ui-polish-route tool="grow" surface="CC-GROW"
ui-polish-route tool="finance" surface="CC-FINANCE"
ui-polish-route tool="automate" surface="CC-AUTOMATE"
ui-polish-route tool="projects" surface="CC-PROJECTS"
ui-polish-route tool="people" surface="CC-PEOPLE"
ui-polish-route setting="agent" surface="SET-AGENT-IDENTITY"
ui-polish-route setting="preferences" surface="SET-PREF-DEFAULTS"
ui-polish-route setting="memory" surface="SET-MEMORY-MANAGE"
ui-polish-route setting="autonomy" surface="SET-AUTONOMY-DEFAULT"
ui-polish-route setting="tools" surface="SET-TOOLS-EXTENSIONS"
ui-polish-route setting="models" surface="SET-MODELS-PROVIDERS"
ui-polish-route setting="keys" surface="SET-KEYS-PROVIDERS"
ui-polish-route setting="devices" surface="SET-DEVICES-PAIRING"
ui-polish-route setting="search" surface="SET-SEARCH-PROVIDERS"
ui-polish-route setting="sources" surface="SET-SOURCES-CATALOG"
ui-polish-route setting="appearance" surface="SET-APPEAR-THEME"
ui-polish-route setting="shortcuts" surface="SET-SHORTCUTS-MAP"
ui-polish-route setting="data" surface="SET-DATA-LOCAL"
ui-polish-route setting="sovereignty" surface="SET-SOV-MODE"
ui-polish-route setting="sessions" surface="SET-SESSIONS-LIST"
ui-polish-route setting="inbox" surface="SET-INBOX-DOWNLOADS"
ui-polish-route setting="activity" surface="SET-ACTIVITY-TRACE"
ui-polish-route setting="spend" surface="SET-SPEND-BUDGET"
ui-polish-route setting="agents" surface="SET-AGENTS-ROSTER"
ui-polish-route setting="features" surface="SET-FEATURES-SWITCHES"
-->

## macOS / Command Center (primary polish order)

| Stable ID · owning node · surface | Entry, nested surfaces, and files | Capability / backend seam | States and current verification gap | Owner |
|---|---|---|---|---|
| CC-SHELL · u3 · Workspace shell and sidebar | Dynamic workspace rows, settings row, notifications tray, collapsed rail; `components/sidebar/Sidebar.tsx`, `components/workspaces/WorkspaceRenderer.tsx` | Workspace layout from daemon; `navigateToTool()` and store workspace actions | Active/hidden workspaces are source-defined; verify every configured tool has a row and that missing/empty layouts are honest. | TBD |
| CC-CHAT · u5 · Chat workspace | `ChatView`; `ChatApp` detached `?view=chat`; `ChatDock`, `ChatLauncher`; `ChatInput`, `MessageList`, `SessionPicker`, `ModelPicker`, `InspectionPanel`, attachment/drop zones | Session and reply APIs, model selection, streaming events, decisions and voice; `components/chat/*`, `lib/chatWindow.ts` | Streaming, stop, model error, empty session, reconnect and detached-window lifecycle have focused tests; native visual parity and all backend failure copy remain pending. | TBD |
| CC-SKILLS · u11 · Skills workspace/overlay | `SkillsPanel` list/grid; `SkillDetailPanel`, `SkillEditor`, `SkillExecutionHistory` | Skill catalog, edit/run/history APIs; `components/skills/*` | Empty/loading/error and editor validation are component concerns; verify overlay close/back and run failure recovery. | TBD |
| CC-TRACE · u15 · Trace / Activity | `ExecutionTrace`; also Settings → Activity | Execution and tool-event stream; `components/trace/ExecutionTrace.tsx` | Long-running, empty and failed runs need a visible distinction; verify deep links from notifications and Settings use the same event scope. | TBD |
| CC-WORLD · u13 · World | Lazy `WorldView`; agent/station panels and focus navigation | World scene/agent/station data; `components/world/*` | W3 browser evidence covers the standalone browser journey; native World rendering, reduced-motion, hidden-tab and pointer journeys remain pending. | TBD |
| CC-TERMINAL · u6 · Terminal | `TerminalManager`, pane tabs and `CycleTabsButton` | Project terminal/session process seam; `components/terminal/*`, `components/build/*` | Pane empty/starting/exited/error and reconnect are separate states; verify tab cycling and process teardown. | TBD |
| CC-BROWSER · u6 · Browser | `Browser`, `BrowserTabs`, `BookmarksBar`; content/act bridge; save-to-Inbox and pop-out | Native child webviews and browser commands; `components/browser/*`, `ui/desktop/src-tauri/src/browser.rs` | Browser W3/W4 interaction evidence exists in the standalone harness; native child-webview permission, teardown and visual checks remain pending. | TBD |
| CC-BRAIN · u12 · Memory / Brain | `BrainView`, `BrainList`, `BrainScene`, search, legend and memory focus | Brain/memory query and focus APIs; `components/brain/*` | Search, legend, and focus tests exist; verify large/empty/error graph and keyboard/pointer affordances visually. | TBD |
| CC-HOME · u4 · Dashboard / Home | `Dashboard`, `HomeBanner`, `LearnNext`, decision cards, customization/add-card picker, overflow menu | Dashboard layout/cards, decisions and goals deep links; `components/dashboard/*`, store navigation | Missing-card and stale/race tests exist; verify card permissions, loading/error, and no dead card after a backend capability is unavailable. | TBD |
| CC-BUILD · u6 · Build | `BuildView`, `CostStatusline`, project chip, progress rail, pending launch, council escalation, cycle tabs | Coding harness/project launch and budget projection; `components/build/*`, harness API/store | Pending launch, rail and cost tests exist; verify unknown/unavailable budget is not shown as zero and terminal/browser pane failures remain actionable. | TBD |
| CC-GROW · u9 · Grow | `GrowView`, lens/category tabs, action groups, results, analytics, funnel, calendar/posts, connection panels | Growth actions, first-party analytics, verification and project seam; `components/grow/*` | Many loading/error/empty branches and focused analytics/action tests exist; verify provider-not-configured vs no-data vs failed query. | TBD |
| CC-FINANCE · u10 · Finance | `FinanceView`, holdings/fundamentals/household areas, labs/picks and “View in World”; `PolybotKeys`, `FundamentalsKey` | Finance provider and key seams; `components/finance/*` | Currency and key tests exist; verify provider/key permission, stale quote, empty holdings, and no accidental account substitution. | TBD |
| CC-AUTOMATE · u11 · Automate | `AutomateView`; Recipes, Running Now, Findings; category tabs; detail panel; confirm/delete/bulk dialogs | Automation roster, findings and action APIs; `components/automate/*` | Delete/confirm/category tests exist; verify queued/running/failed/partial action states and retry semantics. | TBD |
| CC-PROJECTS · u7 · Projects | `ProjectsView`, project cards and `ProjectWorkspace`; `ProjectOverview`, `ProjectKanban`, card detail | Project/goal/document/task APIs; `components/projects/*` | Project selection/drilldown and panel liveness tests exist; verify no-project, loading/error and stale selected-project recovery. | TBD |
| CC-PROJECT-OVERVIEW · u7 · Project lens: Overview | `ProjectWorkspace` `ViewToggle` key `overview`; `ProjectOverview` and its Summary, Watcher Insights, Guard findings, Key Facts, Activity, Links, Tasks and Publish panels | Per-project summary, watcher/guard, activity, task and publish APIs; `components/projects/ProjectOverview.tsx` | Individually verify loading/empty/error and selected-project identity; existing panel tests do not constitute a complete visual pass. | TBD |
| PRJ-OV-SUMMARY · u7 · Project Overview / Summary | `ProjectOverview.tsx` `SummaryPanel` | Project description/brief PATCH seam | Verify read/edit/save/cancel/error and poll does not clobber draft. | TBD |
| PRJ-OV-GUARD · u7 · Project Overview / Guard findings | `ProjectOverview.tsx` `StrixFindingsPanel` | Guard/security findings seam | Verify no findings vs loading/error and actionable finding state. | TBD |
| PRJ-OV-WATCHER · u7 · Project Overview / Watcher insights | `ProjectOverview.tsx` `WatcherInsightsPanel` | Watcher insight seam | Verify stale/empty/error and project identity. | TBD |
| PRJ-OV-FACTS · u7 · Project Overview / Key facts | `ProjectOverview.tsx` `KeyFactsPanel` | Project facts and build bridge | Verify missing facts and navigation failure. | TBD |
| PRJ-OV-ACTIVITY · u7 · Project Overview / Activity | `ProjectOverview.tsx` `ActivityPanel` | Project activity seam | Verify empty/live/error and return scope. | TBD |
| PRJ-OV-TASKS · u7 · Project Overview / Tasks | `ProjectOverview.tsx` `TasksPanel` | Project board/goal event seam | Verify board loading/error/empty and goal detail identity. | TBD |
| PRJ-OV-PUBLISH · u7 · Project Overview / Publish | `ProjectOverview.tsx` `PublishSequencePanel` | Publish sequence seam | Verify blocked/ready/failure and durable result. | TBD |
| CC-PROJECT-DETAILS · u7 · Project lens: Details | `ProjectWorkspace` `ViewToggle` key `details`; `ProjectDetails` and its nested project detail panels | Project metadata, documents, notes, memories, people and related artifact APIs; `components/projects/ProjectDetails.tsx`, `DocumentsPanel.tsx`, `NotesPanel.tsx`, `MemoriesPanel.tsx`, `PeoplePanel.tsx` | Individually verify each nested panel, save/error and return-to-project behavior; source inventory does not claim every nested panel is wired. | TBD |
| PRJ-DE-STACK · u7 · Project Details / Stack | `ProjectDetails.tsx` `StackPanel` | Project stack/reference seam | Verify empty/edit/error and secret/reference boundaries. | TBD |
| PRJ-DE-VERIFICATION · u7 · Project Details / Verification | `ProjectDetails.tsx` `VerificationApprovalPanel` | Verification allowlist/privilege seam | Verify pending/approved/denied/save failure and authority copy. | TBD |
| PRJ-DE-DOCUMENTS · u7 · Project Details / Documents | `ProjectDetails.tsx` `DocumentsPanel` | Project documents and `DocumentViewer` | Verify loading/empty/open/error and stale project identity. | TBD |
| PRJ-DE-NOTES · u7 · Project Details / Notes | `ProjectDetails.tsx` `NotesPanel` | Project notes seam | Verify empty/edit/save/error and note-to-project identity. | TBD |
| PRJ-DE-MEMORIES · u7 · Project Details / Memories | `ProjectDetails.tsx` `MemoriesPanel` | Project memory seam | Verify unavailable/empty/provenance and action errors. | TBD |
| PRJ-DE-CODE · u7 · Project Details / Code index | `ProjectDetails.tsx` `CodeIndexPanel` | Indexed code seam | Verify indexing/loading/stale/error and no-repo state. | TBD |
| PRJ-DE-PEOPLE · u7 · Project Details / People | `ProjectDetails.tsx` `PeoplePanel` | Project/contact association seam | Verify empty/merge/error and person identity. | TBD |
| PRJ-DE-ECOSYSTEM · u7 · Project Details / Ecosystem | `ProjectDetails.tsx` `EcosystemPanel` | Ecosystem relation seam | Verify empty/loading/error and relation navigation. | TBD |
| PRJ-DE-MARKET · u7 · Project Details / Market | `ProjectDetails.tsx` `MarketPanel` | Market context seam | Verify unavailable/empty/stale quote/error without finance mutation. | TBD |
| PRJ-DE-RESOURCES · u7 · Project Details / Resources | `ProjectDetails.tsx` `LinksPanel` title `Resources` | Project resource links seam | Verify empty/add/edit/delete/error and URL validation. | TBD |
| CC-PROJECT-KANBAN · u7 · Project lens: Kanban | `ProjectWorkspace` `ViewToggle` key `kanban`; `ProjectKanban`, card detail and goal deep-link path | Project board/card/task APIs; `ProjectsView.tsx`, `ProjectKanban.cardDetail.test.tsx` | Verify empty columns, drag/action failure, card modal and pending dashboard navigation; this is the actual current lens list (not the older imagined lens list). | TBD |
| CC-PROJECT-MODALS · u7 · Project modals | `CardDetailModal`, `DocumentViewer`, `PersonDetailModal`, merge/edit/enrich/relationship/project/meeting/delete flows | Project card/document/person APIs and confirmation seams | Focused modal tests exist; verify focus trapping, close on failure, unsaved edits, and cross-project identity boundaries. | TBD |
| CC-PEOPLE · u8 · People | `PeopleView`, directory/graph, `MergePersonPanel`, person face/detail | Contacts, graph, enrichment, merge and calendar import APIs; `components/people/*` | Directory/graph/navigation and merge tests exist; verify permission/empty/error and correct person/project deep links. | TBD |
| CC-GOALS · u7 · Goals | `GoalDetailModalHost`, `GoalDetailModal`, dashboard/project goal links | Goal/decision state and project association; `components/goals/*`, store `goalDetail` | Host is globally mounted and links are source-wired; verify missing goal, stale goal, permission and completion/error states. | TBD |
| CC-NOTIFICATIONS · u15 · Notifications | `NotificationHost`, tray and deep-link actions | Notification/event store and navigation | Host is always mounted; verify unread/empty/error, deep-link target disappearance and non-blocking failure. | TBD |
| CC-VOICE · u5 · Voice and meeting | `VoiceHost`, `VoiceButton`, `VoiceOrb`, `VoiceVisualizer`, `MeetingRecorder`, `VoicePicker`, pronunciation settings | Microphone/voice websocket, meeting/system-audio capture and voice model seams; `components/voice/*`, native `system_audio.rs` | Permission, capture, disconnect and playback paths are separately represented in source; verify native permission prompts, no-content leakage, and recovery from route changes. | TBD |
| CC-DROPZONE · u3 · Global drop target | `App.tsx` `DropZone` around `MainContent` plus chat file-drop path | File-to-chat queue/detached-window delivery | Verify disabled World/Brain behavior, dock/detached routing, timeout and delivery failure. | TBD |
| CC-GOAL-HOST · u7 · Global goal modal host | `App.tsx` `GoalDetailModalHost` | Shared goal detail state and deep links | Verify missing/stale goal, close/focus and cross-project identity. | TBD |
| CC-PERSON-HOST · u8 · Global person modal host | `App.tsx` `PersonDetailModalHost` | Shared person detail state and deep links | Verify missing/stale person, close/focus and project/person identity. | TBD |
| CC-NOTIFICATION-HOST · u15 · Global notification host | `App.tsx` `NotificationHost` | Notification tray and action deep links | Verify unread/empty/error, target disappearance and non-blocking action failure. | TBD |
| CC-WORKSPACE-SAVE · u3 · Workspace save error chip | `App.tsx` `WorkspaceSaveErrorChip` | Workspace layout persistence failure | Verify retry, offline/error copy and no silent loss of layout edits. | TBD |
| CC-VERSION-SKEW · u3 · Version skew banner | `App.tsx` `VersionSkewBanner` / `useVersionSkew` | Client/daemon compatibility warning | Verify unavailable/version mismatch and recovery after reload. | TBD |

### Settings panels (SettingsView `PANELS` map)

| Stable ID · owning node · nested panel | Source surface and primary seam | States/gap | Owner |
|---|---|---|---|
| SET-AGENT-IDENTITY · u14 · Persona / Identity | `SettingsView.tsx` `PersonaPanel` → Identity | Name/greeting/traits save, loading/error and identity refresh need parity across windows/devices. | TBD |
| SET-AGENT-VOICE · u14 · Persona / Voice | `PersonaPanel` → Voice, pronunciation and voice picker | Verify unavailable voice, permission, save/error and persisted selection. | TBD |
| SET-AGENT-TONE · u14 · Persona / Tone | `PersonaPanel` → Tone | Verify empty/default tone, save/error and next-conversation semantics. | TBD |
| SET-PREF-DEFAULTS · u14 · Preferences / Defaults | `PreferencesPanel` → Defaults | Verify persisted launch and notification defaults after reload. | TBD |
| SET-PREF-CODE · u14 · Preferences / Your code | `PreferencesPanel` → Your code / developer roots | Verify path validation, permission and unavailable root recovery. | TBD |
| SET-PREF-NOTIFICATIONS · u14 · Preferences / Notifications | `PreferencesPanel` → Notifications | Verify persisted toggles and unavailable native notification permission. | TBD |
| SET-MEMORY-MANAGE · u14 · Memory / Manage | `MemoryPanel` → Manage | Verify empty/unavailable Brain, Librarian action, indexing progress and error. | TBD |
| SET-AUTONOMY-DEFAULT · u14 · Autonomy / Default autonomy | `AutonomyPanel` → Default autonomy and approvals | Verify approval pending/expired/denied, persisted trust mode and actionable errors. | TBD |
| SET-AUTONOMY-SPEND · u14 · Autonomy / Spend caps | `AutonomyPanel` → Spend caps link to Spend | Verify link target and unknown/unavailable cap state. | TBD |
| SET-TOOLS-EXTENSIONS · u14 · Tools / Extensions | `ToolsPanel` → MCP/extensions | Provider unavailable vs not configured must not look like a dead toggle. | TBD |
| SET-MODELS-PROVIDERS · u14 · Models / Providers | `ModelsPanel` → Providers and model picker | Verify route refusal, missing key, loading and persisted model. | TBD |
| SET-MODELS-ROSTER · u14 · Models / Worker roster | `ModelsPanel` → Worker roster | Verify role availability, required secrets and partial/error roster. | TBD |
| SET-MODELS-LOCAL · u14 · Models / Local models | `ModelsPanel` → Local models (Ollama) | Verify daemon unavailable, empty local model list and load failure. | TBD |
| SET-KEYS-PROVIDERS · u14 · API keys / Keys | `KeysPanel` → provider key catalogue | Secret redaction and save/replace/delete/error states require native and visual review. | TBD |
| SET-KEYS-FINANCE · u14 · API keys / Finance keys | `PolybotKeys`, `FundamentalsKey` as reached from key/search panels | Verify absent, invalid, saved and revoked key without exposing credentials. | TBD |
| SET-DEVICES-PAIRING · u14 · Devices / Pairing | `DevicesPanel` → QR, named registry and pair-a-device | Pairing pending/expired/revoked/unreachable states are explicitly modeled; verify on a real paired device. | TBD |
| SET-SEARCH-PROVIDERS · u14 · Search / Providers | `SearchPanel` → Search providers | Verify missing key, disabled provider, quota/error and no-results. | TBD |
| SET-SEARCH-POLYBOT · u14 · Search / Polybot | `SearchPanel` → Polybot | Verify consent/enable, invalid key and finance handoff. | TBD |
| SET-SEARCH-FUNDAMENTALS · u14 · Search / Fundamentals | `SearchPanel` → Fundamentals | Verify optional key, unavailable provider and quote-without-key behavior. | TBD |
| SET-SOURCES-CATALOG · u14 · Data sources / Catalog | `DataSourcesPanel` → source catalogue | Verify consent/enable/disable and failed source separately from empty result. | TBD |
| SET-APPEAR-THEME · u14 · Appearance / Theme | `AppearancePanel` → Theme | Verify system/light/dark persistence and native titlebar coherence. | TBD |
| SET-APPEAR-MOBIUS · u14 · Appearance / Möbius | `AppearancePanel` → Möbius/idle animation/glow/hero | Verify reduced motion, disabled animation and persistence. | TBD |
| SET-APPEAR-DENSITY · u14 · Appearance / Density | `AppearancePanel` → Density | Verify compact/default layout and keyboard/focus behavior. | TBD |
| SET-SHORTCUTS-MAP · u14 · Shortcuts / Key map | `ShortcutsPanel` | Verify conflict/unsupported platform and reset/documentation behavior. | TBD |
| SET-DATA-LOCAL · u14 · Data / Local-first | `DataPanel` → Local-first and data actions | Verify confirmation, cancellation and unavailable export/delete. | TBD |
| SET-DATA-DIAGNOSTICS · u14 · Data / Diagnostics | `DataPanel` → Diagnostics consent | Verify opt-in, persisted consent and refusal/error. | TBD |
| SET-DATA-CRASH · u14 · Data / Crash report | `DataPanel` → redacted crash report export | Verify export failure and redaction without credentials in evidence. | TBD |
| SET-SOV-MODE · u14 · Sovereignty / Sovereign mode | `SovereigntyPanel` → global sovereign flag | Verify policy explanation, persisted state and fail-closed enforcement status. | TBD |
| SET-SOV-AUDIT · u14 · Sovereignty / Egress audit | `SovereigntyPanel` → egress audit | Verify empty/loading/error and allowed-vs-blocked provenance. | TBD |
| SET-SESSIONS-LIST · u14 · Sessions | `SessionsPane` → `SessionsList` | Loading/error/empty and session deletion/selection need visual check. | TBD |
| SET-INBOX-DOWNLOADS · u14 · Downloads | `InboxPane` → `InboxPanel` | Download empty/loading/error and file-open failure require review. | TBD |
| SET-ACTIVITY-TRACE · u14 · Activity | `ActivityPane` → `ExecutionTrace` | Must match workspace Trace event scope and deep-link behavior. | TBD |
| SET-SPEND-BUDGET · u14 · Spend | `SpendPane` → `SpendPanel` and cost/budget views | Unknown/unavailable budget, actual zero, provenance and session identity must remain distinct. | TBD |
| SET-AGENTS-ROSTER · u14 · Agents | `AgentsPanel` | Verify running/stopped/failed/disabled and dispatch failure. | TBD |
| SET-FEATURES-SWITCHES · u14 · Features | `FeaturesPanel` | Verify persisted switch, next-tick status, unavailable and permission/error states. | TBD |

### Detached and native macOS surfaces

| Stable ID · owning node · surface | Source truth | Capability / gap | Owner |
|---|---|---|---|
| MAC-DETACHED-CHAT · u3 · Chat detached window | `lib/chatWindow.ts`, `ChatApp.tsx`, `?view=chat` | Independent top-level window carries the current session and persists geometry; verify focus, close, stale session and two-window recovery natively. | TBD |
| MAC-DETACHED-BROWSER · u6 · Browser child webview/tab | `desktop/src-tauri/src/browser.rs`, `components/browser/*` | Child webviews, opener-preserving auth popups, tab reparenting, download and media teardown; source has lifecycle guards, native journey still pending. | TBD |
| MAC-MENUS · u3 · App/File/Edit/View/Window/Help menus | `desktop/src-tauri/src/menu.rs` | About, Settings, hide/show/quit; New Chat/Close; standard edit; sidebar/reload/force reload; minimize/zoom; documentation. Verify each item invokes the intended renderer/native seam on macOS. | TBD |
| MAC-WIZARD · u3 · First-run wizard | `components/wizard/WizardShell.tsx`, `MomentWelcome`, `MomentIntent`, `MomentHardware`, `MomentMeet`, `MomentWebSearch`, `MomentCode`, `MomentChat`, `MomentCalibration`, `MomentCode` tests | Onboarding intent, hardware, meeting, web/code/chat setup and calibration; verify resume/skip/error and permission copy. | TBD |
| MAC-MIC-PERMISSION · u3 · Microphone/media permission | `App.tsx` media-capture command, `main.rs` `enable_media_capture_cmd`, voice components | WKWebView media capture is best-effort; unavailable mic must remain explicit and recoverable. Native permission prompt/device evidence pending. | TBD |
| MAC-SYSTEM-AUDIO · u3 · System audio permission | `desktop/src-tauri/src/system_audio.rs`, `audiocap/main.swift`, meeting recorder | Screen Recording permission is distinct from mic and capture failure; verify exit-2 guidance and partial transcript cleanup. | TBD |
| MAC-STARTUP · u3 · Startup/daemon connection | `App.tsx` config/wizard/bootstrap and store hydration | Splash/loading/wizard/app and retry behavior; verify daemon unavailable, stale config and restart without fabricated ready state. | TBD |

## iOS companion (second polish order)

| Stable ID · owning node · surface | Entry/subsurfaces and files | Capability / seam | States and current verification gap | Owner |
|---|---|---|---|---|
| IOS-PAIRING · u17 · Pairing and startup | `PermagentApp.swift`, `Views.swift` `PairingView`, `SplashView` | QR/link pairing, hub bootstrap/reconnect and identity refresh | Unpaired, connecting, paired, stale/failed hub states exist in source; verify first-run and real tailnet pairing. | TBD |
| IOS-TABS · u17 · Main tab shell | `PermagentApp.swift` `MainTabs` | Home, Chat, Notes, Decisions, In Flight, Control (`AppTab`) | Badge/identity/tab restoration are source-wired; verify compact devices, dynamic type and reconnect while switching tabs. | TBD |
| IOS-HOME · u17 · Home | `HomeView.swift` | Hub status, to-dos, recent activity, remote hands and deep links to goals/decisions/control | Healthy/unhealthy/checking, empty activity/todos and loading are represented; visual/native evidence pending. | TBD |
| IOS-CHAT · u17 · Chat | `Views.swift` `ChatView` | Streaming chat, stop, history sheet, model picker, decision strip and voice route | Busy/error/empty/history/model refusal states need device verification; stale reply plus new error must remain visible. | TBD |
| IOS-NOTES · u17 · Notes / dictation | `DictateView.swift`, `NotesView.swift`, `NoteComposer` | Quick dictation, notes library, project picker and Brain indexing | Mic denied, recording, transcribing, save/error, no-project and to-do extraction are explicit; verify microphone permission and offline failure. | TBD |
| IOS-MEETING · u17 · Meeting | `MeetingView.swift` | Meeting capture, project selection and confirmation | Permission, recording, partial/final transcript, stop/save/error need device evidence. | TBD |
| IOS-DECISIONS · u17 · Decisions | `Views.swift` `InboxView`, `ChatDecisionStrip` | Decision inbox and answer/defer actions | Empty/unread/loading/error and stale decision deep links need review. | TBD |
| IOS-GOALS · u17 · In Flight / goals | `Views.swift` `GoalsView` | Goal list, in-flight status and goal detail | Empty/loading/stale/completed states need visual review. | TBD |
| IOS-CONTROL · u17 · Control hub | `ControlHub.swift` `ControlHubView` | Navigation to Agents, Automations, Model, Features, Pronunciation, Voice identity | Each destination is a real `NavigationLink`; verify unavailable hub versus empty data and back-stack restoration. | TBD |
| IOS-AGENTS · u17 · Agents | `AgentsView.swift` | Running/background agents, dispatch and stop | Loading/no agents/running/failed/stop-in-flight need device review. | TBD |
| IOS-AUTOMATIONS · u17 · Automations | `SchedulesView.swift` | Scheduled jobs, run now, pause, stop | Empty/loading/action failure and confirmation semantics need review. | TBD |
| IOS-MODEL · u17 · Model | `ModelPickerView.swift` | Provider/model selection and persisted route | Missing key, refused route, loading and saved selection must not be collapsed to a key error. | TBD |
| IOS-FEATURES · u17 · Features | `FeaturesView.swift` | Initiative, Playbook, Concierge, Steward, Guard switches | Persisting/next-tick/error/unavailable states need device evidence. | TBD |
| IOS-PRONUNCIATION · u17 · Pronunciation | `PronunciationView.swift` and desktop pronunciation settings | Teach/forget pronunciation | Save/error/empty and cross-device refresh need review. | TBD |
| IOS-VOICE-ID · u17 · Voice identity | `ControlHub.swift` `VoiceIdentityView`, onboarding full-screen cover | Private enrollment, redo, forget, skip and speaker gate | Preparing/downloading/error/enrolling/enrolled/unknown are source states; verify mic permission and replay/revocation behavior. | TBD |
| IOS-VOICE · u17 · Voice conversation | `VoiceView.swift` full-screen route | Orb, live user transcript, assistant reply, model sheet, agent-controls sheet, playback/capture lifecycle | Loading/listening/thinking/speaking/error/cancel/reconnect and no-audio states require device evidence; no successful audio is inferred here. | TBD |
| IOS-VOICE-SHEETS · u17 · Voice sheets/modals | `VoiceView` model picker and agent-controls sheets; `ChatHistorySheet` | Model/agent/session controls | Verify sheets preserve turn identity and close safely during active capture/playback. | TBD |

## watchOS companion (same visual identity; third polish order)

| Stable ID · owning node · surface | Source | Capability / seam | States and current verification gap | Owner |
|---|---|---|---|---|
| WATCH-HOME · u18 · Watch home | `PermagentWatch/WatchHomeView.swift` | Chat and Dictate destinations through `WatchRelay` | Paired/unpaired notice and disabled actions are source-wired; verify compact layout and reconnect. | TBD |
| WATCH-CHAT · u18 · Watch Chat | `WatchChatView` in `WatchHomeView.swift` | Auto-listening recorder, relay chat, orb and response | Mic denied/listen error/thinking/reply/idle are distinct; verify late reply and repeated listen lifecycle. | TBD |
| WATCH-DICTATE · u18 · Watch Dictate | `WatchNoteView` in `WatchHomeView.swift` | Waveform, finish, transcribe/save and project selection | Recording/transcribing/saving/saved/choose-project/error; inspect remaining project list and verify real write/read. | TBD |
| WATCH-RELAY · u18 · Watch relay/recorder seam | `WatchRelay.swift`, `WatchRecorder.swift`, `WatchBridge`/`HubWatchRelay` | Pairing, audio transfer and hub calls | Transport failure and cancellation must not look like an empty note; device evidence pending. | TBD |

## Target manifest and companion audit

Read-only source check of `ios/PermagentMobile/project.yml`, the generated
`PermagentMobile.xcodeproj`, and both target plists found exactly three targets:
the iOS app `Permagent`, the embedded watch application `PermagentWatch`, and
the Foundation/unit-test bundle `PermagentTests`. The iOS app embeds the watch
target; the watch target includes `PermagentWatch/WatchHomeView.swift`,
`WatchRecorder.swift`, `WatchRelay.swift`, shared `WatchBridge.swift`, and the
listed brand resources. There are currently no WidgetKit, watch complication,
notification-service, Share, App Intents extension, or other companion target
directories in the manifest. `TalkToAgentIntent.swift` is app/watch source,
not evidence of a separate extension target.

The iOS plist declares microphone, camera, local-network, audio-background and
tailnet ATS exceptions; the watch plist declares microphone permission,
independent watch launch, and the iOS companion bundle ID. This confirms the
source paths and permission surfaces listed above, but not their physical-device
or App Store acceptance.

## Confirmed capability reverse joins and bounded gaps

This is a user-facing subset of the API client, not a dump of every daemon
route. The first rows are confirmed joins; the `GAP-*` rows are explicit
classification items rather than silently treating an unused API method as a
feature.

| Stable ID · candidate node | API/client producer | Confirmed home or current status | Disposition |
|---|---|---|---|
| API-CHAT · u5 | sessions, reply/cancel/events, attachments upload | Chat workspace, dock/detached ChatApp and store reply path | Joined; attachment rendering uses inline images and durable refs. |
| API-HARNESS-ACTIVE · u6 | `getActiveHarnessRuns`, budget projection, council APIs, terminal summary/completion | Build, Terminal, Council and Grow bridge | Joined; verify each state through the existing Build rows. |
| API-HISTORY · u14 | sessions/list/detail/cost/delete | Settings → Sessions and Chat session picker | Joined; same session identity must be preserved. |
| API-TOOLS · u14 | extensions list/probe/add, provider/key/secret-source APIs | Settings → Tools, Models, Keys, Search and wizard | Joined for list/probe/configuration; deletion remains a bounded gap below. |
| API-BRAIN · u12 | Brain search/memories, project memories/notes/code index, Librarian status/run | Brain, Project Details, World HUD and Settings Memory | Joined; provenance/empty/offline states remain verification work. |
| API-BROWSER · u6 | bookmarks/tab sets/history/inbox and native browser commands | Browser, Downloads/Inbox and detached child webviews | Joined; native lifecycle acceptance remains pending. |
| API-WORLD-AGENTS · u13/u14 | world agent state, Henry/Librarian status, agent roster/settings APIs | World agent HUD, Settings → Agents and Control/companion Agents | Joined; World ambient state must remain honest about simulated seats. |
| API-FINANCE-GROW · u9/u10 | project growth actions, finance/provider/key seams | Grow, Finance and project Build bridge | Joined; safe source/authority and provider failure evidence pending. |
| API-SOVEREIGNTY-DATA · u14 | sovereignty/egress, consent, crash export, inbox | Settings → Sovereignty, Data & privacy, Downloads | Joined; native redaction/permission verification pending. |
| GAP-HARNESS-HISTORY · candidate u6 | `api.getHarnessRunHistory()` → `/api/coding-sessions/harness-runs/history` | No current Command Center caller found; active runs are consumed by Build/store, but historical runs have no confirmed home. | Decide whether Build history is user-facing, route it there, or document this as internal/retire; do not claim covered. |
| GAP-EXTENSION-REMOVE · candidate u14 | `api.removeExtension()` → `DELETE /config/extensions/:name` | Tools panel currently lists/probes extensions but has no command-center caller for removal. | Decide whether removal belongs in Settings → Tools; if not, classify endpoint as internal/legacy. |
| GAP-WORLD-STRATA · candidate u13 | `api.getWorldStrata()` → `/api/world/strata` | No current Command Center caller found; World atmosphere reads `/api/brain/graph` and events instead. | Reconcile stale “Carved Cave” API comment with the actual World contract; wire or retire only after owner review. |
| GAP-ATTACHMENT-MGMT · candidate u5 | `getAttachmentUrl()` and `deleteAttachment()` are declared, while upload is used by the chat store | No current UI caller for direct durable attachment fetch/delete; MessageRenderer renders inline message images. | Determine whether these are server lifecycle/internal helpers or require a Chat attachment-management affordance. |

The absence checks above were performed against current source callers and are
not claims that the daemon endpoints are dead. They are intentionally separate
from confirmed UI joins so `u1-inventory` can resolve ownership before any
production edit.

## Top-level `api` method disposition (frozen denominator)

The following is the complete 118-method top-level `export const api` object
from `ui/command-center/src/lib/api.ts` at this inventory snapshot. Every
method has one disposition: a confirmed UI home, an internal/indirect consumer,
or an explicit unexposed gap. This table deliberately excludes direct
`apiFetch` calls and native/Tauri commands; those are tracked separately below.

| Method | Disposition / surface ID and node |
|---|---|
| `getHealth` | Internal liveness primitive; Settings currently calls `/status` through `apiFetch`, no separate home. |
| `listDevices` | `SET-DEVICES-PAIRING` / u14 |
| `pairDevice` | `SET-DEVICES-PAIRING` / u14 |
| `renameDevice` | `SET-DEVICES-PAIRING` / u14 |
| `revokeDevice` | `SET-DEVICES-PAIRING` / u14 |
| `getSessions` | `SET-SESSIONS-LIST`, Chat session picker / u14/u5 |
| `getSession` | Chat session restore / u5 |
| `getSessionCost` | `CC-BUILD` CostStatusline / u6 |
| `deleteSession` | Sessions and Chat session picker / u14/u5 |
| `createSession` | Chat session picker/store / u5 |
| `sendReply` | `CC-CHAT` / u5 |
| `cancelReply` | `CC-CHAT` / u5 |
| `sessionEventsUrl` | Chat SSE stream / u5 |
| `getIdentity` | Chat identity and `SET-AGENT-IDENTITY` / u5/u14 |
| `putIdentity` | `SET-AGENT-IDENTITY` / u14 |
| `getVoices` | `SET-AGENT-VOICE`, voice picker / u14/u5 |
| `getVoiceModelStatus` | Voice model settings / u14 |
| `downloadVoiceModels` | Voice model settings / u14 |
| `getVoiceDownloadProgress` | Voice model settings / u14 |
| `getPronunciations` | `SET-AGENT-VOICE`, `IOS-PRONUNCIATION` / u14/u17 |
| `getUnresolvedPronunciations` | Pronunciation settings / u14 |
| `savePronunciation` | Pronunciation settings / u14 |
| `deletePronunciation` | Pronunciation settings / u14 |
| `getConfig` | Startup, store hydration and Settings / u3/u14 |
| `getIntegrations` | `SET-FEATURES-SWITCHES` / u14 |
| `readConfig` | Settings capability/config panels / u14 |
| `readSecretConfig` | Keys, Search & tools and onboarding / u14/u3 |
| `getExtensions` | `SET-TOOLS-EXTENSIONS`, Search & tools / u14 |
| `probeExtension` | Tools/Search provider setup and wizard / u14/u3 |
| `upsertConfig` | Settings/configuration forms / u14 |
| `setModelRoute` | `SET-MODELS-PROVIDERS` / u14 |
| `getCouncilLatest` | Council dashboard card and World HUD / u4/u13 |
| `getActiveHarnessRuns` | `CC-BUILD`, store liveness / u6 |
| `getHarnessRunHistory` | `GAP-HARNESS-HISTORY`: no current Command Center caller; candidate Build history / u6 |
| `conveneCouncil` | Build Council escalation / u6 |
| `getCouncilMembers` | Features/Council settings / u14 |
| `putCouncilMembers` | Features/Council settings / u14 |
| `removeConfig` | Keys, Finance keys and provider forms / u14/u10 |
| `addExtension` | Search & tools extension registration / u14 |
| `removeExtension` | `GAP-EXTENSION-REMOVE`: no current Tools-panel caller; candidate confirmed Settings home / u14 |
| `getSovereignty` | `SET-SOV-MODE` / u14 |
| `setSovereignty` | `SET-SOV-MODE` / u14 |
| `getEgressLog` | `SET-SOV-AUDIT` / u14 |
| `ackBriefings` | World Henry briefing action / u13 |
| `getIncidents` | Settings Activity/incident triage / u14 |
| `resolveIncident` | Settings Activity/incident triage / u14 |
| `getSpend` | `SET-SPEND-BUDGET` / u14 |
| `getBudget` | Internal/redundant getter; `getSpend` carries the budget consumed by SpendPanel; no separate home. |
| `setBudget` | `SET-SPEND-BUDGET` / u14 |
| `getCrashConsent` | `SET-DATA-DIAGNOSTICS` / u14 |
| `setCrashConsent` | `SET-DATA-DIAGNOSTICS` / u14 |
| `setAnalyticsConsent` | `SET-DATA-DIAGNOSTICS` / u14 |
| `exportCrashReport` | `SET-DATA-CRASH` / u14 |
| `getProviders` | `SET-MODELS-PROVIDERS`, API keys and store resolution / u14/u5 |
| `getProviderModels` | Provider/model picker / u14 |
| `reloadConfig` | Provider configuration modal / u14 |
| `getSecretSources` | Keys/provider configuration and onboarding / u14/u3 |
| `setSecretSource` | Keys/provider configuration and onboarding / u14/u3 |
| `testSecretSource` | Keys/provider configuration and onboarding / u14/u3 |
| `codingSessionSummary` | Terminal close/summary path / u6 |
| `completeGrowthActionFromHarness` | Terminal → Grow project bridge / u6/u9 |
| `setProvider` | Startup wizard, store resolution and Grow provider setup / u3/u9 |
| `checkProvider` | Provider configuration modal / u14 |
| `getPacks` | Role-routing prompt and Models / u5/u14 |
| `applyPacks` | Role-routing prompt and Models / u5/u14 |
| `createCustomProvider` | Add custom provider modal / u14 |
| `removeCustomProvider` | Providers panel / u14 |
| `getSkills` | Skills load/store / u11 |
| `createSkill` | Skill execution/store path / u11 |
| `updateSkill` | Skill editor / u11 |
| `getSkillExecutions` | Skill execution history / u11 |
| `deleteSkill` | Skill detail/store path / u11 |
| `dismissSkillProposal` | Chat skill prompt/store / u5/u11 |
| `getSkillProposals` | Chat skill prompt/store / u5/u11 |
| `getWorkspaces` | Workspace shell/store / u3 |
| `getWorkspace` | Internal/unused single-workspace helper; bulk workspace load is the current path. |
| `getActiveWorkspace` | Workspace shell/store / u3 |
| `setActiveWorkspace` | Workspace shell/store / u3 |
| `updateWorkspaceLayout` | Workspace shell and save-error chip / u3 |
| `uploadAttachments` | Chat composer/store durable attachment refs / u5 |
| `getAttachmentUrl` | `GAP-ATTACHMENT-MGMT`: no current UI caller; classify lifecycle vs Chat affordance / u5 |
| `deleteAttachment` | `GAP-ATTACHMENT-MGMT`: no current UI caller; classify ownership/retention action / u5/u14 |
| `listProjectDocuments` | `PRJ-DE-DOCUMENTS` / u7 |
| `uploadProjectDocuments` | `PRJ-DE-DOCUMENTS` / u7 |
| `fetchProjectDocumentBlob` | `PRJ-DE-DOCUMENTS` DocumentViewer / u7 |
| `fetchGrowMediaBlob` | Grow media/results / u9 |
| `deleteProjectDocument` | `PRJ-DE-DOCUMENTS` / u7 |
| `listProjectMemories` | Project Details memories/activity/code panels / u7 |
| `listProjectNotes` | `PRJ-DE-NOTES`, Activity / u7 |
| `createProjectNote` | Project Notes and meeting dictation / u7/u5 |
| `deleteProjectNote` | `PRJ-DE-NOTES` / u7 |
| `indexProjectCode` | `PRJ-DE-CODE` / u7 |
| `provisionDictationModel` | Startup dictation provisioning / u3 |
| `transcribeAudio` | Dictation and meeting hooks / u5/u17 |
| `getBrowserBookmarks` | Browser BookmarksBar / u6 |
| `putBrowserBookmarks` | Browser BookmarksBar / u6 |
| `getBrowserTabSets` | Browser BookmarksBar/tab sets / u6 |
| `putBrowserTabSets` | Browser tab sets / u6 |
| `getBrowserHistory` | Browser history / u6 |
| `recordBrowserHistory` | Browser navigation / u6 |
| `getInbox` | Downloads/Inbox / u15/u14 |
| `fetchAuthed` | Chat awareness and InspectionPanel activity reads / u5 |
| `getSystemInfo` | Wizard hardware step / u3 |
| `getDevRoots` | Wizard and Settings code-root step / u3/u14 |
| `checkDevRoot` | Wizard and Settings code-root validation / u3/u14 |
| `pullOllamaModel` | Wizard hardware and long-running local model job / u3/u14 |
| `startOllama` | Wizard hardware / u3 |
| `getOllamaStatus` | Models and Agents settings, wizard hardware / u3/u14 |
| `getLibrarianSchedule` | Models/Agents settings and wizard / u3/u14 |
| `setLibrarianSchedule` | Agents settings and wizard / u3/u14 |
| `getLibrarianStatus` | World Librarian HUD / u13 |
| `getLibrarianRunStatus` | World Librarian HUD / u13 |
| `getHenryStatus` | World Henry HUD and agent state source / u13 |
| `getWorldStrata` | `GAP-WORLD-STRATA`: declared “Carved Cave” endpoint has no current caller; reconcile or retire / u13 |
| `searchBrain` | Brain search / u12 |
| `getBrainMemories` | Brain list/search / u12 |
| `getAgents` | World simulated-agent roster / u13 |
| `runLibrarianNow` | World Librarian HUD and Agents settings / u13/u14 |

## Direct source denominator: normalized `apiFetch` route expressions

The direct-call parser found 118 distinct literal/template route expressions
outside the top-level `api` object after normalizing request IDs/project IDs/
query builders to `{param}`/`?…`. Each is listed once below with its confirmed
surface owner. There are also 12 dynamic first-argument call sites (listed
below) whose route is selected by an existing helper or manifest value and
therefore cannot be honestly reduced to one source literal.
This table is source coverage, not a claim that each route deserves a new UI.

| Normalized route expression | Surface / node | Source family |
|---|---|---|
| `/api/agent/identity` | MAC-WIZARD / u3 | `components/wizard/WizardShell.tsx` |
| `/api/agents/roster` | DIRECT-AGENTS / u8/u13/u14 | `lib/agentsApi.ts` |
| `/api/agents/{param}` | DIRECT-AGENTS / u8/u13/u14 | `lib/agentsApi.ts` |
| `/api/agents/{param}/grants` | DIRECT-AGENTS / u8/u13/u14 | `lib/agentsApi.ts` |
| `/api/agents/{param}/secrets` | DIRECT-AGENTS / u8/u13/u14 | `lib/agentsApi.ts` |
| `/api/agents/{param}/work{param}` | DIRECT-AGENTS / u8/u13/u14 | `lib/agentsApi.ts` |
| `/api/brain/graph` | CC-BRAIN / u12 | `dashboard/Echo.tsx`, `world/atmosphere/worldSignals.ts` |
| `/api/browser/act/{param}` | DIRECT-BROWSER-BRIDGES / u6 | `hooks/useBrowserActBridge.ts` |
| `/api/browser/content/{param}` | DIRECT-BROWSER-BRIDGES / u6 | `hooks/useBrowserContentBridge.ts` |
| `/api/browser/snapshot/{param}` | DIRECT-BROWSER-BRIDGES / u6 | `hooks/useBrowserActBridge.ts` |
| `/api/cards/due` | CC-HOME / u4 | `lib/useDueTodos.ts` |
| `/api/dashboard` | CC-HOME / u4 | `dashboard/useDashboard.ts` |
| `/api/dashboard/card-types` | CC-HOME / u4 | `dashboard/cards/useCardRegistry.ts` |
| `/api/dashboard/layout` | CC-HOME / u4 | `dashboard/useLayout.ts` |
| `/api/decisions` | CC-WORLD / u13 | `world/agents/decisionSignals.ts` |
| `/api/finance` | CC-FINANCE / u10 | `finance/FinanceView.tsx`, `world/financeDesk.ts` |
| `/api/finance/notes` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/notes/{param}` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/picker/scan` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/picker/start` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/polybot/pause` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/polybot/scan` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/polybot/start` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/transactions/{param}` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/watchlist` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/finance/watchlist/{param}` | CC-FINANCE / u10 | `finance/FinanceView.tsx` |
| `/api/goals/active` | CC-WORLD / u13 | `world/agents/goalActivity.ts` |
| `/api/grow/higgsfield` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/grow/postiz` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/growth-results` | CC-GROW / u9 | `dashboard/cards/GrowthResultsCard.tsx`, `world/GrowthMeasurementHUD.tsx` |
| `/api/growth-results?…` | CC-GROW / u9 | `grow/GrowResults.tsx` |
| `/api/inbox/{param}/route` | SET-INBOX-DOWNLOADS / u14 | `inbox/inboxRouting.ts` |
| `/api/job-health` | CC-AUTOMATE / u11 | `world/areas/automate/scheduleActivity.ts` |
| `/api/onboarding/status` | MAC-WIZARD / u3 | `dashboard/LearnNext.tsx` |
| `/api/people` | CC-PEOPLE / u8 | `people/*`, `projects/PersonDetailModal.tsx` |
| `/api/people{query}` | CC-PEOPLE / u8 | `projects/PeoplePanel.tsx` |
| `/api/people/calendar/import` | CC-PEOPLE / u8 | `people/PeopleView.tsx` |
| `/api/people/directory` | CC-PEOPLE / u8 | `people/MergePersonPanel.tsx`, `PeopleGraphCanvas.tsx` |
| `/api/people/directory{param}` | CC-PEOPLE / u8 | `people/PeopleDirectory.tsx` |
| `/api/people/duplicates?…` | CC-PEOPLE / u8 | `people/MergePersonPanel.tsx` |
| `/api/people/merges/{param}/undo` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/activity` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/fields` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/meetings` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/meetings/{param}` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/merge` | CC-PEOPLE / u8 | `people/MergePersonPanel.tsx` |
| `/api/people/{param}/merge-preview?…` | CC-PEOPLE / u8 | `people/MergePersonPanel.tsx` |
| `/api/people/{param}/projects` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/relationships` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/people/{param}/relationships/{param}/{param}` | CC-PEOPLE / u8 | `projects/PersonDetailModal.tsx` |
| `/api/projects` | CC-PROJECTS / u7 | `projects/ProjectsView.tsx`, project consumers |
| `/api/projects/{param}` | CC-PROJECTS / u7 | `projects/ProjectsView.tsx`, project helpers |
| `/api/projects/{param}/analytics/connection` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/connection/test` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party/drain` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party/enable` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party/funnel` | CC-GROW / u9 | `grow/FunnelPanel.tsx` |
| `/api/projects/{param}/analytics/first_party/rotate` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party/stats` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/first_party/step_options?…` | CC-GROW / u9 | `grow/FunnelPanel.tsx` |
| `/api/projects/{param}/analytics/first_party/verify` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/analytics/stats?…` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/brand` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/cards` | CC-PROJECTS / u7 | Project Overview/Activity/Board consumers |
| `/api/projects/{param}/cards/{param}` | CC-PROJECTS / u7 | Goal/Card detail and board |
| `/api/projects/{param}/cards/{param}/approve` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/cards/{param}/auto-approve` | CC-PROJECTS / u7 | `lib/roadmapClient.ts` |
| `/api/projects/{param}/cards/{param}/cancel` | CC-PROJECTS / u7 | Goal/Card detail and board |
| `/api/projects/{param}/cards/{param}/dismiss-due` | CC-PROJECTS / u7 | `lib/useDueTodos.ts` |
| `/api/projects/{param}/cards/{param}/due-date` | CC-PROJECTS / u7 | Board/card detail and due todos |
| `/api/projects/{param}/cards/{param}/media/retry` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/cards?…` | CC-PROJECTS / u7 | Goal/Grow card queries |
| `/api/projects/{param}/columns` | CC-PROJECTS / u7 | Board/card/goal detail |
| `/api/projects/{param}/growth-actions` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/growth-actions/` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/growth-actions/generate` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/growth-inbox` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/intel/{param}` | PRJ-DE-ECOSYSTEM / u7 | `projects/EcosystemPanel.tsx` |
| `/api/projects/{param}/market` | PRJ-DE-MARKET / u7 | `projects/MarketPanel.tsx` |
| `/api/projects/{param}/meetings` | PRJ-DE-PEOPLE / u7 | `projects/PeoplePanel.tsx` |
| `/api/projects/{param}/people` | PRJ-DE-PEOPLE / u7 | Project People panel |
| `/api/projects/{param}/people/{param}` | PRJ-DE-PEOPLE / u7 | `projects/PersonDetailModal.tsx` |
| `/api/projects/{param}/publisher` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/publisher/connect` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/publisher/{param}` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/roadmap/goals` | CC-PROJECTS / u7 | `lib/roadmapClient.ts` |
| `/api/projects/{param}/roadmap/goals/{param}/dependencies` | CC-PROJECTS / u7 | `lib/roadmapClient.ts` |
| `/api/projects/{param}/roadmap/goals/{param}/remove` | CC-PROJECTS / u7 | `lib/roadmapClient.ts` |
| `/api/projects/{param}/strategy/{param}` | CC-GROW / u9 | `grow/GrowView.tsx` |
| `/api/projects/{param}/touch` | CC-PROJECTS / u7 | `build/useProjects.ts` |
| `/api/projects/{param}/verification-approval` | CC-PROJECTS / u7 | `projects/verificationApproval.ts` |
| `/api/projects?…` | CC-PROJECTS / u7 | `build/useProjects.ts` |
| `/api/public-apis/catalog?…` | SET-SOURCES-CATALOG / u14 | `settings/DataSourcesSection.tsx` |
| `/api/public-apis/{param}/enable` | SET-SOURCES-CATALOG / u14 | `settings/DataSourcesSection.tsx` |
| `/api/public-apis/{param}/key` | SET-SOURCES-CATALOG / u14 | `settings/DataSourcesSection.tsx` |
| `/api/runs` | CC-AUTOMATE / u11 | Automate roster and World signals |
| `/api/sessions/{param}/name` | CC-CHAT / u5 | `lib/store.ts` |
| `/api/tailnet/access` | SET-DEVICES-PAIRING / u14 | `settings/SettingsView.tsx` |
| `/api/tailnet/status` | SET-DEVICES-PAIRING / u14 | `settings/SettingsView.tsx` |
| `/api/version` | CC-VERSION-SKEW / u3 | `lib/version.ts` |
| `/automation/finding/{param}/action` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/automation/recovery/total` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/automation/run/{param}/findings` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/config/extensions` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/create` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/delete/{param}` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/list` | CC-AUTOMATE / u11 | Automate and World schedule activity |
| `/schedule/{param}/kill` | CC-AUTOMATE / u11 | Automate roster/actions |
| `/schedule/{param}/pause` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/{param}/reset_to_default` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/{param}/run_now` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/{param}/sessions?…` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/schedule/{param}/unpause` | CC-AUTOMATE / u11 | `automate/AutomateView.tsx` |
| `/status` | MAC-STARTUP / u3 | `settings/SettingsView.tsx` |
| `/api/projects/{param}/stack/{param}` | PRJ-DE-STACK / u7 | `projects/stackEntries.ts` |
| `/api/finance/fx?…` | CC-FINANCE / u10 | `finance/displayCurrency.ts` |

### Dynamic first-argument `apiFetch` call sites (12 call sites)

These are still finite source-denominator rows, not an assumption that the
dynamic route is covered. The source location and exact first argument are
kept so a future route/helper change fails the companion checker and receives
an explicit surface review.

| Source location | First argument | Disposition |
|---|---|---|
| `ui/command-center/src/components/brain/useBrainData.ts:52` | `endpoint` | CC-BRAIN / u12; query-keyed `/api/brain/graph` helper |
| `ui/command-center/src/components/dashboard/cards/ManifestCard.tsx:81` | `manifest.dataEndpoint` | CC-HOME / u4; backend card manifest, dynamic by design |
| `ui/command-center/src/components/dashboard/cards/ManifestCard.tsx:111` | `manifest.configure.endpoint` | CC-HOME / u4; backend card configuration action, dynamic by design |
| `ui/command-center/src/components/dashboard/cards/TimelineCard.tsx:82` | `journalUrl(kinds, actor)` | CC-HOME / u4; timeline journal helper |
| `ui/command-center/src/components/dashboard/cards/TimelineCard.tsx:121` | `journalUrl(kinds, actor, nextBefore)` | CC-HOME / u4; timeline pagination helper |
| `ui/command-center/src/components/finance/FinanceView.tsx:1209` | `path` | CC-FINANCE / u10; picker trade or position create/update |
| `ui/command-center/src/components/finance/FinanceView.tsx:1420` | `deletePath` | CC-FINANCE / u10; picker trade or position deletion |
| `ui/command-center/src/components/finance/FinanceView.tsx:1430` | `closePath` | CC-FINANCE / u10; picker trade or position close |
| `ui/command-center/src/components/grow/GrowView.tsx:501` | `path` | CC-GROW / u9; social card deletion |
| `ui/command-center/src/components/grow/GrowView.tsx:503` | `path` | CC-GROW / u9; social card patch |
| `ui/command-center/src/components/projects/stackEntries.ts:48` | `stackPath(projectId)` | PRJ-DE-STACK / u7; stack entry list |
| `ui/command-center/src/components/projects/stackEntries.ts:56` | `stackPath(projectId)` | PRJ-DE-STACK / u7; stack entry create/update |

## Native command denominator: 31 invoke names

The command-center source has 31 distinct `invoke` names outside test files.
Every name is assigned to a confirmed surface or an internal shell seam below;
the table is intentionally separate from the direct HTTP route denominator.

| Command | Surface / node | Source disposition |
|---|---|---|
| `act_on_ref` | MAC-DETACHED-BROWSER / u6 | Browser act bridge |
| `browser_go` | MAC-DETACHED-BROWSER / u6 | Browser history navigation |
| `browser_nav_state` | MAC-DETACHED-BROWSER / u6 | Browser navigation state |
| `close_browser` | MAC-DETACHED-BROWSER / u6 | Browser child lifecycle |
| `create_browser_webview` | MAC-DETACHED-BROWSER / u6 | Browser child lifecycle |
| `destroy_pane_window` | MAC-DETACHED-BROWSER / u6 | Detached pane lifecycle |
| `emit_activity` | CC-TRACE / u15 | Activity telemetry seam |
| `enable_media_capture_cmd` | MAC-MIC-PERMISSION / u3 | Media permission/capture seam |
| `get_daemon_token` | MAC-STARTUP / u3 | Internal authenticated startup seam |
| `get_page_content` | MAC-DETACHED-BROWSER / u6 | Browser content bridge |
| `get_page_snapshot` | MAC-DETACHED-BROWSER / u6 | Browser snapshot bridge |
| `get_pty_output` | CC-TERMINAL / u6 | Terminal stream |
| `haptic_success` | MAC-SHELL-FEEDBACK / u3 | Internal shell feedback |
| `hide_browser` | MAC-DETACHED-BROWSER / u6 | Browser child visibility |
| `kill_pty` | CC-TERMINAL / u6 | Terminal lifecycle |
| `list_pty_sessions` | CC-TERMINAL / u6 | Terminal roster |
| `navigate_browser` | MAC-DETACHED-BROWSER / u6 | Browser navigation |
| `raise_chat_above_main` | MAC-DETACHED-CHAT / u3 | Detached chat window |
| `read_audio_chunk` | NATIVE-MEDIA / u3/u5 | Audio capture seam |
| `read_dropped_file` | CC-DROPZONE / u3 | Drop-zone bridge |
| `reap_orphan_browsers` | MAC-DETACHED-BROWSER / u6 | Browser child cleanup |
| `reparent_browser` | MAC-DETACHED-BROWSER / u6 | Browser window ownership |
| `resize_pty` | CC-TERMINAL / u6 | Terminal layout |
| `save_tab_to_inbox` | MAC-DETACHED-BROWSER / u6 | Browser-to-Inbox action |
| `spawn_pty_session` | CC-TERMINAL / u6 | Terminal lifecycle |
| `start_system_audio` | NATIVE-MEDIA / u3/u5 | Meeting/system audio capture |
| `stop_system_audio` | NATIVE-MEDIA / u3/u5 | Meeting/system audio teardown |
| `system_audio_available` | NATIVE-MEDIA / u3/u5 | System audio capability check |
| `update_browser_bounds` | MAC-DETACHED-BROWSER / u6 | Browser child layout |
| `write_to_pty` | CC-TERMINAL / u6 | Terminal input |
| `zoom_browser` | MAC-DETACHED-BROWSER / u6 | Browser zoom |

## Portal and actionable-host reverse join

These are the independently opened or actionable hosts that are easy to miss
when only sidebar routes are inventoried. Each source path is assigned to the
existing surface that owns its state; no new surface is inferred from a shared
modal primitive.

| Source host / path | Disposition / surface | Capability and remaining gate |
|---|---|---|
| `ui/command-center/src/App.tsx:63-89` | CC-SHELL, CC-SKILLS, CC-GOALS, CC-PEOPLE, CC-DROPZONE, CC-NOTIFICATIONS / u3/u7/u8/u11/u15 | Always-mounted Settings, Skills, workspace error boundary, goal/person hosts, drop target and notification host; each already has a ledger row. Native focus/z-order acceptance remains later u16 work. |
| `ui/command-center/src/components/workspaces/WorkspaceRenderer.tsx:47-67` | ToolType-owned rows / u4-u14 | Dynamic panel switch and per-panel `ErrorBoundary`; source checker covers the ToolType denominator, while error copy and recovery remain per-screen verification. |
| `ui/command-center/src/components/notifications/NotificationHost.tsx:30-164` | CC-NOTIFICATIONS / u15 | Tray, toast, click-away, dismiss and deep-link/onActivate actions; target disappearance is an acceptance state, not an unowned capability. |
| `ui/command-center/src/components/chat/SkillPromptBanner.tsx:10-64` | CC-CHAT + CC-SKILLS / u5/u11 | Real skill proposal save/dismiss flow; proposal failure remains visible and is not a second Skills route. |
| `ui/command-center/src/components/chat/RoleRoutingPrompt.tsx:16-130` | CC-CHAT, CC-BUILD, SET-MODELS-PROVIDERS / u5/u6/u14 | Shared role recommendation/apply prompt has three consumers; provider/model authority remains Settings/daemon-owned. |
| `ui/command-center/src/components/chat/ImageMessage.tsx:11-29`, `chat/Lightbox.tsx:22-93` | CC-CHAT / u5 | Image lightbox is an in-message modal, not a detached capability; keyboard close and failed/missing image states remain chat verification. |
| `ui/command-center/src/components/settings/AddCustomProviderModal.tsx:145-284` | SET-MODELS-PROVIDERS / u14 | Provider creation/edit modal; route and persistence are already owned by Models, with focus/error verification pending. |
| `ui/command-center/src/components/settings/SettingsView.tsx:872-924` | SET-MODELS-PROVIDERS / u14 | Role-routing settings panel and provider/model rows; no independent route. |
| `ui/command-center/src/components/finance/FinanceView.tsx:882-907` | CC-FINANCE / u10 | Polybot/Fundamentals disclaimer dialogs gate the existing finance actions. |
| `ui/command-center/src/components/automate/AutomateView.tsx:2000-2010, 2096-2185` | CC-AUTOMATE / u11 | Bulk preview/confirm dialog owns the existing findings action API; no dead standalone modal. |
| `ui/command-center/src/components/dashboard/Dashboard.tsx:463-480` | CC-HOME / u4 | Home confirmation dialog for dashboard action; decision state remains the producer. |
| `ui/command-center/src/components/sessions/SessionsList.tsx:310-337` | SET-SESSIONS-LIST / u14 | Session deletion confirmation; session identity stays in the Sessions pane. |
| `ui/command-center/src/components/common/ErrorBoundary.tsx:1-89` | CC-SHELL / u3 plus panel owner | Shared rendering failure boundary; it is recovery chrome, not a hidden backend capability. |

## Backend producer reverse-join disposition

This is the source-level reverse join from supported daemon producers to a UI
home or an explicit internal/unexposed disposition. Compatibility aliases and
model/provider internals are grouped by their route module; the exact route
paths are retained so a future surface change cannot silently reclassify them.

| Backend source path and route producer | UI home / owner | Disposition |
|---|---|---|
| `crates/goose-server/src/routes/dashboard_cards.rs:208-212` — `/api/dashboard/card-types`, `/api/dashboard/system-stats`, `/api/dashboard/calendar`, `/api/dashboard/weather`, `/api/dashboard/weather/location` | CC-HOME / u4 | Covered through the daemon manifest registry and `ManifestCard` dynamic data/configure calls; not omitted just because the route is not a static api method. |
| `crates/goose-server/src/routes/backup.rs:129-130` — `/api/backups`, `/api/backups/run` | SET-DATA-LOCAL / u14 | Explicit unexposed gap: durable backup is supported but no current Command Center caller exists. Candidate Data action requires confirmation and recovery copy. |
| `crates/goose-server/src/routes/coding_session.rs:600-607` — `/api/coding-sessions/summary`, `/spend`, `/turn` | CC-BUILD / u6 | Summary is surfaced by Build; spend/turn are producer/ledger ingestion endpoints, not user controls. Internal-only for the latter two, with canonical CostStatusline ownership retained. |
| `crates/goose-server/src/routes/brain.rs:1194-1204` — `/api/brain/search`, `/graph`, `/memories`, `/memory`, backfill routes | CC-BRAIN / u12 | Search/graph/memories are covered; `/api/brain/memory` and both backfills are explicitly classified as detail/internal-maintenance gaps, not silently treated as UI features. |
| `crates/goose-server/src/routes/session.rs:669-708` — session search/export/import/insights/fork/extensions/user-recipe-values | SET-SESSIONS-LIST + CC-CHAT / u14/u5 | List/get/name/cost/delete are covered. Export, import, insights, fork, extensions and user-recipe-values have no current Command Center caller; retain as explicit session capability gaps. |
| `crates/goose-server/src/routes/projects.rs:2389-2469` — tags, intel deletion, people disassociation, memory association, document bytes/deletion and stack | CC-PROJECTS / PRJ-DE-* / u7 | Document, memory list, notes, code index and stack callers are covered. Tags, intel delete, person disassociation and project-memory association lack a confirmed current UI caller; assign to the corresponding nested Project Details row for follow-up. |
| `crates/goose-server/src/routes/config_management.rs:1766-1815` — provider catalog/templates, cleanup, prompts, init/backup/recover/validate/permissions, workspace trust | SET-MODELS-PROVIDERS, SET-TOOLS-EXTENSIONS, SET-PREF-CODE / u14 | Existing generic config/provider callers cover the active settings path. Provider catalog templates, prompt editing, config recovery/validation and workspace-trust mutations remain explicit unexposed/admin gaps; no backend-only control is claimed as visible. |
| `crates/goose-server/src/routes/public_apis.rs:32-36` — categories/enabled/catalog/enable/key | SET-SOURCES-CATALOG / u14 | Catalog/enable/key are covered; categories/enabled are backend discovery helpers used by the same source catalogue contract, not separate surfaces. |
| `crates/goose-server/src/routes/storage.rs:91` — `/permagent/storage/scan` | CC-AUTOMATE / u11 | Producer writes the existing findings store; no direct scan button is currently exposed. Explicit internal/scheduled producer disposition, with Automate Findings as the read home. |
| `crates/goose-server/src/routes/action_required.rs:39` — `/action-required/tool-confirmation` | CC-CHAT + CC-HOME / u5/u4 | Legacy per-tool confirmation transport; the Decision Inbox mirror is the discoverable home and the Chat control remains the live-turn consumer. |
| `crates/goose-server/src/routes/terminal_supervision.rs:144-145` — supervised output/session ingestion | CC-TERMINAL + CC-HOME / u6/u4 | Terminal owns the PTY producer and Decision Inbox owns gated approvals; no separate supervision tab is inferred. |
| `crates/goose-server/src/routes/gateway.rs:217-223`, `routes/tunnel.rs:74-76` — gateway/tunnel lifecycle | SET-DEVICES-PAIRING / u14 | Authenticated daemon/admin control with no current Command Center caller; explicit internal/admin gap, not a dead Settings button. |
| `crates/goose-server/src/routes/program.rs:72-74`, `routes/recipe.rs`, `routes/recipe_utils.rs` — program handoff and recipe transport | CC-BUILD + CC-AUTOMATE / u6/u11 | CLI/evaluation and agent recipe protocol; internal producer/transport, not a user-facing route absent a registered UI action. |
| `crates/goose-server/src/routes/local_inference.rs:693-717` (feature-gated) — local model catalog/download | SET-MODELS-LOCAL / u14 | Feature-gated backend capability; current Settings exposes Ollama/local status only. Explicit unavailable/unexposed feature disposition. |
| `crates/goose-server/src/routes/desktop_control.rs`, `routes/browser_content.rs`, `routes/browser_act.rs` — native desktop/browser bridges | CC-BROWSER / MAC-DETACHED-BROWSER / u6 | Covered by Browser bridge/native command rows; public/localhost transport is a producer seam, not an additional sidebar surface. |

The reverse join therefore has no unexplained producer omission in this
inventory snapshot. The rows marked “explicit gap” are implementation/product
decisions for later uN work, not missing inventory ownership; native acceptance,
visual verification and those product decisions remain later gates.

### Direct `apiFetch` and native command coverage (separate denominator)

These paths intentionally bypass the exported `api` object. The exact route and
native-command denominators immediately above are the source ledger; this
group table is only the compact reconciliation from those rows to product
areas. Dynamic project IDs, action IDs, and browser request IDs remain scoped
to their source call sites rather than being treated as separate capabilities.

| Stable group · candidate node | Confirmed source paths / command family | Current home or remaining review |
|---|---|---|
| DIRECT-DASHBOARD · u4 | `components/dashboard/useDashboard.ts`, `useLayout.ts`, card registry, `useDueTodos.ts`, `LearnNext.tsx` | Dashboard/Home cards, layout, due cards and onboarding; verify each manifest/config action has a capability row. |
| DIRECT-PROJECTS-GOALS · u7 | `ProjectsView.tsx`, `ProjectOverview.tsx`, `CardDetailModal.tsx`, `GoalDetailModal.tsx`, `verificationApproval.ts`, `publishSequence.ts`, `workspaceMeta.ts`, `roadmapClient.ts` | Projects, goal modal and project nested panels; source joins are confirmed, individual dynamic endpoint evidence remains per-row work. |
| DIRECT-PEOPLE · u8 | `PeopleView.tsx`, `PeopleDirectory.tsx`, `PeopleGraphCanvas.tsx`, `MergePersonPanel.tsx`, `PersonDetailModal.tsx`, `PeoplePanel.tsx` | People and person/project modal rows; verify identity, merge, meeting and calendar boundaries. |
| DIRECT-GROW · u9 | `GrowView.tsx`, `GrowResults.tsx`, `FunnelPanel.tsx`, calendar/media/publisher/analytics paths | Grow lenses, analytics, publishing and media; provider-not-configured vs no-data remains a verification obligation. |
| DIRECT-FINANCE · u10 | `FinanceView.tsx`, `displayCurrency.ts`, `world/financeDesk.ts` | Finance board, Polybot/Fundamentals and World finance desk; risky mutations remain native acceptance work. |
| DIRECT-AUTOMATE · u11 | `AutomateView.tsx`, `RunRoster.tsx`, `world/areas/automate/scheduleActivity.ts` | Automate recipes/schedules/findings and World activity; confirm/cancel/error state rows remain separate. |
| DIRECT-BRAIN · u12 | `brain/useBrainData.ts`, `world/atmosphere/worldSignals.ts`, `dashboard/Echo.tsx` | Brain graph and World atmosphere; source confirms `/api/brain/graph` rather than `getWorldStrata`. |
| DIRECT-AGENTS · u8/u13/u14 | `lib/agentsApi.ts`, `world/agents/stateSources.tsx`, `settings/agents/*` | Settings Agents and World roster/HUD; no additional user-facing target inferred. |
| DIRECT-AWARENESS · u5 | `awareness/PreTurnPreview.tsx`, `AwarenessIndicator.tsx`, `inspection/InspectionPanel.tsx` | Chat inspection/awareness; direct activity endpoints need privacy and offline-state review. |
| DIRECT-BROWSER-BRIDGES · u6 | `useBrowserContentBridge.ts`, `useBrowserActBridge.ts`, `Browser.tsx` | Browser content/act/snapshot, inbox save, navigation and zoom; native child lifecycle remains pending. |
| DIRECT-DICTATION · u5/u17 | `useDictation.ts`, `useMeetingDictation.ts`, `MeetingRecorder.tsx` | Desktop voice/meeting and iOS companion recording; permission and teardown evidence required. |
| DIRECT-DATA-SOURCES · u14 | `DataSourcesSection.tsx` public API catalog/enable/key endpoints | Settings Data sources; explicit consent and source failure must remain visible. |
| DIRECT-SETTINGS-TAILNET · u3/u14 | `SettingsView.tsx` `/api/tailnet/status`, serve/access commands and `/status` health read | Devices pairing and startup/health; direct endpoint is not a separate capability home. |
| NATIVE-MEDIA · u3/u5 | `enable_media_capture_cmd`, `system_audio_available`, `start_system_audio`, `stop_system_audio`, `read_audio_chunk` | App/Chat/Meeting permission and capture seams; native permission evidence pending. |
| NATIVE-BROWSER · u6 | `create_browser_webview`, `navigate_browser`, `browser_go`, `update_browser_bounds`, `hide_browser`, `close_browser`, `reparent_browser`, `reap_orphan_browsers`, `save_tab_to_inbox`, `get_page_content`, `get_page_snapshot`, `act_on_ref`, `zoom_browser` | Browser child webviews and detached pane; W3/W4 browser evidence is not native acceptance. |
| NATIVE-PTY · u6 | `spawn_pty_session`, `list_pty_sessions`, `get_pty_output`, `write_to_pty`, `resize_pty`, `kill_pty` | Terminal/Build; verify ownership, close/kill and replay state. |
| NATIVE-SHELL · u3/u5 | `raise_chat_above_main`, `destroy_pane_window`, `haptic_success`, `get_daemon_token`, `emit_activity`, native menu callbacks | Detached windows, shell menus, notifications/activity and haptics; each remains a native acceptance concern. |

## Unresolved implementation and acceptance work (deliberately not silently omitted)

The source inventory and bidirectional disposition are complete for this
snapshot. The following rows remain implementation/product or native acceptance
work; they must not be reported as passed merely because ownership is assigned:

- Workspace layouts are daemon-provided. `ToolType`, the renderer map, portal
  hosts and backend producer families are now source-joined by the lightweight
  checker and tables above; user/configured layout variants and missing-tool
  runtime behavior still need u3/u16 acceptance.
- `App.tsx` always-mounted hosts and the independently opened/actionable
  portals are listed in the reverse-join table. Their focus, failure, target
  disappearance and native z-order behavior still need per-screen acceptance.
- The Project workspace has the three actual `ProjectLens` values
  (`overview`, `details`, `kanban`) from `ProjectWorkspace.tsx`; Overview and
  Details child panels now have individual IDs. The reverse-join table records
  which nested producers are covered and which association/tag actions remain
  explicit product gaps.
- Settings nested sections were split from the current `SettingsView.tsx`
  section headings. Any future subsection, dialog, or provider-specific panel
  added outside `PANELS` must receive a new `SET-*` ID rather than being hidden
  under its parent category.
- Native browser popups, detached panes, system permission sheets, and menu
  validation are source rows only; W3/W4 browser harness evidence is not native
  macOS acceptance.
- The target-manifest audit found no additional iOS/watchOS extension or
  complication target beyond `Permagent`, `PermagentWatch`, and
  `PermagentTests`; this is a source fact, not device/App Store acceptance.
  Any future target or source group must add rows before platform completeness
  is claimed.
- Backend-only producers and internal APIs have an explicit home or internal /
  unexposed reason in the reverse-join table. That is inventory ownership, not
  proof that an explicitly unexposed feature should be implemented or that a
  later native control is discoverable and working.

`u1-inventory` source coverage is READY for review. The four API-object gaps
(`getHarnessRunHistory`, `removeExtension`, `getWorldStrata`, and attachment
management), plus the route-module gaps listed above, remain deliberate
follow-up rows and are not claimed complete.

The companion checker
`docs/orchestrator/ui-polish-surface-inventory.test.mjs` is intentionally a
small no-production Node test. It rejects duplicate IDs, unknown `uN` owners,
source-defined ToolType/settings names absent from this ledger, API-method
denominator drift, dynamic `apiFetch` call-site drift, and native command
denominator drift. It is a source-coverage guard, not a substitute for the
per-screen audit or native acceptance gates.

## Reuse candidates for the polish DAG

Prefer existing primitives and seams before introducing new visual or state
systems: Command Center `ViewHeader`, settings `H1`/row/toggle atoms, web
`Panel`, `StateBlock`, `ConfirmDialog`, `DetailModal`/`FormModal`,
`ErrorBoundary`, `JobProgress`, `Chip`, `Button`, `NotificationHost`,
`ChatDock`, `BrowserTabs`, `useLongRunningJob`, `usePollWhenVisible`,
`useAppNavigate`, `navigateToTool`, and theme tokens. `RaisedCard`,
`DesignPolicy`, `AppBackdrop`, and `ChatSurface` are iOS SwiftUI primitives,
not Command Center web components; their reuse belongs to the iOS owner. Reuse
the existing backend store/API slices and identity-aware navigation rather than
adding parallel local stores or timestamp joins.

## u2 shared-system scoped evidence

`StateBlock` and `FormModal` actionable explanatory copy now use the existing
`textMuted` semantic instead of `textDim`. Regression coverage is in
`ui/command-center/src/components/common/StateBlock.test.tsx` and
`ui/command-center/src/components/common/FormModal.test.tsx`; it checks the
rendered semantic token and preserves the retry invocation/disabled guard.

`ui/command-center/src/styles/semanticContrast.test.ts` is the numerical token
gate: for dark, aurora, and silver it computes WCAG relative luminance and
requires `textMuted` to be at least 4.5:1 on the actual body (`bg`) and card
(`surface`) fills. It also requires primary `text` to remain readable on all
three solid fills, and requires `textMuted` to outrank `textDim` on prose
surfaces. `surfaceHi` is intentionally excluded from the muted-prose rule: in
the dark theme its 4.49:1 result is a control/elevation surface, not a
readable-copy background. Disabled control opacity and quiet/decorative chips
are likewise not counted as explanatory prose; disabled explanations use the
readable `textMuted` path.

This closes the shared-token numerical criterion for this slice, not every
small `textDim` use in the product. DecisionInbox's user-facing loading,
connection, empty-state, history, and footer-action copy now uses
`textMuted`, with component regressions in
`ui/command-center/src/components/dashboard/decisions/DecisionInbox.test.tsx`;
technical state-binding/age metadata and the disclosure chevron remain dim by
design. Remaining feature-specific labels outside this audited surface are
separate u3/u4 follow-ups; native/visual acceptance remains pending and does
not block these shared primitives.

## u3 shell audit — compact rail, chat dock, and offline/discoverability (read-only)

This bounded audit follows the current source, not an assumed shell: the
compact rail is `components/sidebar/Sidebar.tsx` (`208px` open / `64px`
collapsed), the tooltip is portalled by `SidebarTooltip.tsx`, the dock is
`components/chat/ChatDock.tsx` (`384px` wide or a full-width fixed sheet below
640px), and detached chat is `ChatLauncher.tsx` + `lib/chatWindow.ts`.

| Surface / evidence | Confirmed behavior and existing tests | Bounded defect or acceptance gap | Owner / next gate |
|---|---|---|---|
| Compact rail rows | Every row is a shared `Button`; collapsed labels survive as `aria-label`; focus and hover enter the portalled tooltip; Cmd/Ctrl+1..N selects a workspace. `reservedRect.test.ts` covers tooltip/browser clearance and `rawButton.test.ts` covers the primitive-adoption exception. | No direct Sidebar render test covers collapse/expand, focus tooltip timing, active marker, or shortcut selection. The 260ms cold tooltip delay is a keyboard/discoverability visual check, not a source-proven failure. | u3 shell interaction test/native visual pass; no production change in this audit. |
| Settings/collapse discoverability | Settings remains a labelled row; open rail exposes “Collapse”, collapsed rail exposes an icon-only “Expand” with `aria-label`. App handles Cmd/Ctrl+, for Settings. | The Settings shortcut is implemented in `App.tsx` but is not surfaced in the sidebar tooltip/shortcut map; confirm whether the shortcut is intentionally undocumented before changing copy. | u3 shortcut/discoverability owner. |
| Chat dock | `ChatDock` reuses `ChatView`, closes through store state, keeps detached and docked surfaces mutually exclusive, and uses theme-aware surface/shadow. `ChatView.lifecycle.test.tsx` covers unmount cancellation and shared session-picker mounting. | Narrow mode is a full-width fixed sheet (`<640px`) but has no `role=dialog`, `aria-modal`, focus containment, Escape ownership, or scrim; this is a concrete keyboard/screen-reader gap if it remains an overlay rather than a page transition. No direct ChatDock lifecycle/focus test exists. | u3 shell accessibility owner; production fix intentionally deferred until u2 fan-in. |
| Detached chat | `createChatWindow()` carries the current session and geometry; launcher observes window creation/destruction and hides when dock/window is open. `chatWindow.ts` and related lifecycle comments are the source contract. | Native focus/close/stale-session/two-window recovery remain unverified; source-only evidence cannot claim parity. | macOS native acceptance row `MAC-DETACHED-CHAT`. |
| Chat offline/session creation | Message history failures retain the session ID and render an inline Retry (`MessageList.tsx`); that path is source-wired. | `ensureSession()` catches create failure and returns `null`; initial `ChatView` has no explicit session-unavailable/offline state, and `SessionPicker.handleNewSession()` closes the menu then swallows failure. A user can receive no visible feedback after a failed new-session attempt. Existing lifecycle tests cover offline identity rejection, not these failure states. | u3 chat error-state follow-up; add real component regressions before changing behavior. |
| Shell/workspace offline | `connectionStatus` transitions through connecting/connected/disconnected in `store.ts`; event-stream reconnect is bounded. `loadWorkspaces()` now latches `workspacesError`, preserves the last-known list, and `MainContent` renders an accessible `StateBlock` retry when no list exists. Regressions: `lib/workspacesLoad.store.test.ts`, `App.workspaces.test.tsx`. | No current Command Center consumer renders the lower-level `connectionStatus`; a preserved stale workspace list has no separate connection banner. Those are follow-up discoverability/native checks, not empty-state conflation. | u3 startup/offline follow-up; separate from native daemon restart acceptance. |
| Chat keyboard actions | `ChatInput` implements Enter-to-send, Shift+Enter newline, Stop while streaming, file attach, and shared Button states. `ChatInput.stop.test.tsx` covers cancellation/re-arm/error behavior; MessageBubble copy test covers keyboard-reachable actions. | Send/Enter and dock Escape/focus behavior lack direct shell-level regression coverage. Native compact/dynamic-type layout remains unverified. | u3 interaction tests + macOS visual/device pass. |

The audit records one remaining concrete source-level offline/discoverability
defect for later bounded implementation (session creation failure feedback),
plus one conditional accessibility defect (the narrow ChatDock is an overlay
without a dialog/focus floor). The workspace-load/empty conflation is fixed in
the store/MainContent seam with the focused regressions named above. The audit
does not treat missing visual/native evidence as a production bug.

## Verification boundary

Root's September 6 local headless visual attempt did not reach the interactive
shell: the actual page entered onboarding rather than exposing sidebar
controls. The evidence script now rejects splash/onboarding as shell success
and preserves the failure-state screenshot at
`/private/tmp/permagent-ui-shared-review.png`. No onboarding controls were
activated and no config was changed. This does not qualify U2 native visual
acceptance; use an authenticated current app or an explicitly isolated fixture
for that gate. Source review also shows config-read failure eventually enters
the wizard (`App.tsx` loading effect), a distinct follow-up from workspace-list
failure handling; do not infer first-run state solely from an unavailable hub.

The repository contains focused unit/component tests for many race, error,
navigation, browser, cost, project, people, grow, finance, skills, dashboard,
and settings paths. Those tests are useful regression evidence, not visual
acceptance. A follow-on polish pass should attach exact test commands and
native/browser/device evidence per row, including permission-denied,
reconnect, reduced-motion, dynamic-type/compact-size, detached-window, and
screen-reader checks. Rows marked above as pending must remain pending until
that evidence exists.
