# Permagent flow matrix — 2026-10-07 resume

October 8 continuation: [Agent Desk redesign and bounded CUA](AGENT_DESK_REDESIGN_2026-10-08.md) adds compact profile/disclosure, narrow close/reopen, all four lessons, three demonstration branches and shared-chat navigation evidence. It does not complete native, capture or remaining multi-step matrix rows.

Source audit: `b55bf214feeb41bd5c9708cfafe628f98c491608`, branch `world/production-20261006`. This matrix combines a source-derived test plan with the bounded runtime evidence log below. Unchecked items still need complete current-build Computer Use coverage; a partial observation does not complete a multi-step row. Earlier evidence is explicitly historical and may be Chrome, visible Chromium, or older native r28. Source paths below are relative to `ui/command-center/src/components/` unless stated otherwise.

## Evidence and execution rules

- `B` = coordinator browser CUA at `:5301`, detailed in [browser acceptance](CUA_BROWSER_2026-10-07.md); partial B evidence is not native acceptance.
- `H` = handoff / `reports/QA_FLOW_WALK_2026-10-07.md` says the area was walked; exact subflows may not have been covered. `N28` = historical native r24–r28 evidence described by CHECKPOINT. `S` = inspected source only. `P` = current runtime verification pending. Do not turn an H/S item green without current screenshot/interaction evidence.
- `R` = read-only navigation or reversible local view state; `L` = local persistent preference/artifact or controllable visual simulation; `W` = live daemon write, network action, credential change, capture, execution, finance or destructive operation. W identifies an effect, not an automatic permission demand. Exercise W against a named disposable fixture or an already-authorized real target, and record the effect/cleanup. Do not approve unrelated real work merely to check a button.
- Refind the UI after every tab, subview, modal, expansion, navigation, scroll or viewport change. Take a fresh state/screenshot; locate controls from the current view; perform one coherent interaction; verify the resulting state. Keep the inspected window visible: hidden-tab pause previously caused false black-canvas and delayed-navigation bugs.
- Start at 1280×800 logical native size, then inspect minimum 1024×640 for clipping. Record app identity, source/build revision, backend identity, viewport, action, observed result and screenshot path per batch. Restore settings changed for QA.
- Sidebar workspaces come from daemon data (`sidebar/Sidebar.tsx`), not a fixed tab array. Inventory the actual live sidebar first. Tool registry supports dashboard, projects, people, build, grow, finance, automate, memory, world, terminal, browser, chat, skills and trace (`workspaces/WorkspaceRenderer.tsx`, `../lib/store.ts:139`). Include any extra workspace exposed by the user's profile.

## Recommended CUA pass order

1. Record exact running native identity/version and sidebar; Home → Projects → People → Build → Grow → Finance → Automate → Brain.
2. Record modal → cancel → History; Notifications; all Settings panes; chat dock/pop-out idle behavior. These validate shell fixes before entering the heavier 3D view.
3. World arrival → overview → six Places → day/night → complete walking routes → all agent identities/dock controls → desk tabs and all demonstrations → legacy campus as a separate branch → restore compact/System.
4. Revisit Grow Results, Automate Learned, History and Settings to expose second-visit/scroll/navigation regressions. Narrow-window pass and app/window hide-regain pass.
5. Separate fresh-config wizard pass and isolated non-mic voice error-state pass. Native latest staged smoke after rebuild; re-walk changed surfaces after fixes.

## Shell and Home

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] SH1 | Boot/splash → connected app; no stale loading screen; active sidebar and content agree | R | `../App.tsx`, `splash/BootScreen.tsx` | H → P |
| [ ] SH2 | Unpaired browser explains pairing; invalid/expired claim explains recovery; already-paired browser can continue | W pairing; R failure display | `../lib/daemonAuth.ts`, `../App.tsx` | Q1/Q2/Q24 source-only fixes → P |
| [ ] SH3 | Collapse/expand sidebar; hover/focus tooltips; workspace keyboard shortcuts; current nav remains identifiable | L | `sidebar/Sidebar.tsx` | H → P |
| [ ] SH4 | Switch tab with Record open; switch immediately while World loads; verify content AND selected nav | R | `../App.tsx`, sidebar | Q16 unverified fix; Q25 withdrawn hidden-tab issue → P |
| [ ] SH5 | Notifications tray, read item/detail navigation, close/Escape; distinguish mark read/dismiss from inspect | R/W read state | `notifications/` | H → P |
| [ ] H1 | Scroll every Home card: focus/weather/CPU, Needs You Review, manifest, active jobs, growth, calendar, recent/todos; no overlap or clipping | R | `dashboard/Dashboard.tsx`, `dashboard/cards/` | Q10 fixed; Q11 historically open → P |
| [ ] H2 | Open Needs You/decision detail, evidence and discussion; close without committing live approval | R/W approve/hold/reject/discuss | `dashboard/decisions/DecisionInbox.tsx`, `DecisionReview.tsx` | H broad coverage → P |
| [ ] H3 | Customize, add-card picker, layouts gallery, template preview/cancel, card menu and restore layout after optional edit | L layout save; W calendar import/access | `dashboard/{CustomizeButton,AddCardPicker,LayoutsGallery,LayoutPreviewOverlay,DashboardOverflowMenu}.tsx` | H; Q8 writes-on-read fixes → P |
| [ ] H4 | Calendar access/empty/error states, focused Day/Week/Month ranges and event details; distinguish today-only fallback from a verified empty future range | R/W EventKit permission/import | `dashboard/cards/{useShellCalendar,CalendarAccessPanel}.tsx` | Calendar/Home source integrated; native range acceptance P |
| [ ] H5 | Focus a Home card; other cards compact; collapse/Escape restores overview; keyboard focus, reduced motion, narrow headers and calendar expansion | R local presentation | `dashboard/{Dashboard,homeFocus}.tsx`, `homeFocus.css` | Source and focused tests passed; current native P |
| [ ] H6 | Agent navigation to Home card type/overview, project lens, World place and Record picker; delayed mount, missing destination and replay behavior | R navigation only; no automatic capture | `../hooks/useAppNavigate.ts`, `../lib/appNavigationRequests.ts` | Allowlisted source and focused tests; actual native navigation P |

## Projects, People and Build

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] P1 | Projects portfolio: active/status groups, expand paused/archived, select project, back, switch project | R; W dragging project status | `projects/{ProjectsView,ProjectWorkspace}.tsx` | H → P |
| [ ] P2 | Project Overview: status/progress/next work; open linked goal and return | R | `projects/ProjectOverview.tsx`, `goals/GoalDetailModal.tsx` | H → P |
| [ ] P3 | Details: Stack, verification approvals, Documents/viewer, Notes, Memories, Code index, People, Ecosystem, Market, Resources; expand and scroll every panel | R inspect; W edit/remove/index/approval configuration | `projects/ProjectDetails.tsx` and named panel files | S inventory, H broad → P |
| [ ] P4 | Kanban: skeleton → columns; horizontal scroll through Review/Done; open card detail; card actions; add draft/cancel | R | `projects/ProjectsView.tsx:551`, `CardDetailModal.tsx` | Q20 fixed → P |
| [ ] P5 | Disposable card/goal lifecycle: add, edit due date, move, cancel/delete only fixture; evidence and publish sequence UI | W | `projects/{CardDetailModal,PublishSequencePanel,VerificationApprovalPanel}.tsx`, goals | No complete prior lifecycle claim → P |
| [ ] PE1 | People Graph/List, search/filter, graph key, select person, inspect detail/relationships/project links and close | R | `people/{PeopleView,PeopleDirectory,PeopleGraphCanvas}.tsx`, `projects/PersonDetailModal.tsx` | H; Q12 hidden-tab false alarm withdrawn → P |
| [ ] PE2 | Person add/edit/merge/relation controls on disposable identities; cancellation preserves live records | W | `people/MergePersonPanel.tsx`, person modal | S → P |
| [ ] B1 | Build project selector/chip, terminal/browser pane toggles, resize splitter, recent runs detail, cost status and council detail | R/L; project launch can spawn process | `build/{BuildView,ProjectChip,RecentRunsPanel,CouncilEscalation}.tsx` | H; Q13 fixed → P |
| [ ] B2 | Terminal existing tabs/focus/cycle, new/close disposable tab; browser tabs/address/back/forward/reload/bookmarks; no crash without Tauri | L process/browser navigation; W typed commands | `terminal/TerminalManager.tsx`, `browser/Browser.tsx`, `build/CycleTabsButton.tsx` | Q6 browser guard fixed → P |
| [ ] B3 | Agent-run start/cancel and run output/errors using a bounded disposable task; reopening existing real run is read-only until input | W | build/terminal and supervised session controls | No new result → P |

## Grow, Finance, Automate and Brain

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] G1 | Grow project selector; Actions, Results, Strategy, Calendar, Analytics; revisit Results after other views; slow/error/retry displays | R | `grow/GrowView.tsx`, `GrowResults.tsx` | H; Q19 slow route unresolved, UI timeout added → P |
| [ ] G2 | Actions categories/card details/evidence/verify; text ends at word boundary and useful content is accessible | R; W accept/run/send-to-worker/verify writes | `grow/{GrowActions,ActionCard,ActionVerify,growActionTabs}.tsx` | Q14 daemon truncation remains → P |
| [ ] G3 | Strategy expansion, calendar post detail/media/tracking, analytics period/funnel/connections/publisher settings; cancel edit forms | R; W publish/schedule/delete/connection edits | `grow/{StrategyLens,CalendarLens,PostMedia,TrackingRail,GrowAnalytics,PublisherSettings,AnalyticsConnectionPanel}.tsx` | H broad → P |
| [ ] F1 | Finance full scroll: holdings/lot details, picks when enabled, household, watchlist, notes, display currency and research links | R/L currency | `finance/FinanceView.tsx` | H → P |
| [ ] F2 | Optional Polybot/Picker enable dialogs open/cancel, configuration hints, key fields masked; do not enable desk to merely inspect | R open; W enable/start/stop/scan/credentials | `finance/FinanceView.tsx:894`, `{PolybotKeys,FundamentalsKey}.tsx` | H broad → P |
| [ ] F3 | Add/remove lot, exit lot, household/watchlist/note editing on fixtures; no real trading or portfolio changes assumed | W | `finance/FinanceView.tsx` | S → P |
| [ ] A1 | Automate Recent Activity, Automations, Learned, Installed; search, job health and run detail; sticky strip at every scroll position | R | `automate/{AutomateView,SectionNav,RunRoster,JobHealthStrip}.tsx` | H; Q21 visual fix unverified → P |
| [ ] A2 | Leave/revisit Learned twice; skills/proposals/installed extension detail; execution history | R | `automate/LearnedList.tsx`, `skills/{SkillsPanel,SkillDetailPanel,SkillExecutionHistory}.tsx` | Q23 timeout/cache fix → P |
| [ ] A3 | New automation modal Recurring/One-time/Interval, cron presets/custom and validation; cancel draft | R | `automate/AutomateView.tsx:2717` | H broad → P |
| [ ] A4 | Create/pause/resume/run/delete disposable job; accept/dismiss proposal/install/edit skill only on controlled target | W | automate and skills | S → P |
| [ ] BR1 | Brain Graph/List, search/clear restores mode, people/projects/topics subtype/memory filters, legend, time slider, selected entity/memory panel and links | R/L view | `brain/{BrainView,BrainList}.tsx` | H; Q15 bottom padding unverified → P |
| [ ] BR2 | Scroll final list row at time slider; memory source expansion and related project/person navigation | R | `brain/BrainView.tsx` | S → P |
| [ ] BR3 | Forget/undo behavior against disposable memory; never delete live memory just for coverage | W | `brain/BrainView.tsx:160` | S → P |

## History, chat, recorder and voice without microphone capture

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] HI1 | History Sessions, Downloads, Activity, Spend; search/filter/sort/period/detail where exposed; empty/missing numeric data display; return/back | R | `history/HistoryView.tsx`, `sessions/SessionsList.tsx`, `history/SpendPanel.tsx` | H; Q18 fixed real Chrome → P |
| [ ] HI2 | Session open/export, download preview/local save, activity detail; budget cap editor inspect/cancel | R/L export; W rename/delete/caps | same | H broad → P |
| [ ] C1 | Open chat dock in wide/minimum window, agent/session/model menus, composer reachability, resize/close/reopen; no offscreen transition | R/L | `chat/{ChatDock,ChatLauncher}.tsx` | H; Q22 not reproduced → P |
| [ ] C2 | Native chat pop-out, switch app tabs, close pop-out returns consistent dock; drafts and attachments add/remove before send | L; W attachment processing may upload | chat and `../lib/chatWindow.ts` | S → P |
| [ ] C3 | Bounded text-only QA message round-trip; visible sent/pending/reply/model/error/cancel behavior | W real session/model spend | chat, `../lib/store.ts` | H previous ~5s reply → P |
| [ ] R1 | Record meeting modal, project list/search/select, cancel/Escape; open Record then History to verify Q16 | R | `voice/MeetingRecorder.tsx` | H → P |
| [ ] R2 | Existing recorder drafts: inspect warning/recovery controls; avoid saving/dismissing real draft without intended action | R/W save/discard | same | S → P |
| [ ] V1 | Idle voice button tooltip/accessible label; typing spaces in focused composer does not trigger voice; idle pop-out mirror stays idle | R | `voice/VoiceButton.tsx`, `VoiceHost.tsx` | Explicit handoff gap → P |
| [ ] V2 | Settings Persona Voice picker opens, available/unavailable entries, cancel; pronunciation list expand/add draft/cancel and footer clearance | R; L/W selection/save/preview | `voice/{VoicePicker,PronunciationSection}.tsx` | Explicit handoff gap → P |
| [ ] V3 | Models Voice route readout shows effective session/configured/default model truthfully; inspect only | R | `settings/SettingsView.tsx:95` | S → P |
| [ ] V4 | Isolated permission-denied/no-device/connecting/ready/error/retry state coverage; test capture only in explicitly scoped capture pass | W if live activation | `../hooks/useVoice.ts:1475`, `voice/voiceStatus.ts` | No runtime evidence → P |

**Critical voice distinction:** idle mic click calls `activate()`, which immediately acquires a microphone via `getUserMedia` before opening the socket. It is not a non-mic connection check. Hands-free and Record Start also activate capture. Existing source tests can cover synthetic states, but those are not native interaction evidence. Voice preview may call a TTS provider even without mic capture.

## Settings — all 13 panes

Every pane: load, scroll to bottom, Page Down/keyboard focus, expand sections, open/cancel subordinate dialogs, leave/revisit, and verify chat launcher never covers controls. Most controls auto-save; inspect before toggling.

| Check | Pane/subflows | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] S1 | Preferences: owner/linked person; Defaults/open-on-launch; code folders picker/manual path | R inspect; W owner/folder writes, L launch | `settings/SettingsView.tsx:324,422`, `DevRootsSection.tsx` | H → P |
| [ ] S2 | Persona: Identity, Voice, Tone, pronunciation rows and bottom Remove control | R inspect; W identity/voice writes | `SettingsView.tsx:196`, voice files | H; Q3/Q17 fixed → P |
| [ ] S3 | Agents: each roster member, gates/config, skills/Librarian awaiting, expand/collapse | R inspect; W worker gates and settings | `settings/agents/AgentsPanel.tsx` | H → P |
| [ ] S4 | Memory: feed/source controls, Manage, learn-from toggles | R inspect; W forget/import/source toggles | `SettingsView.tsx:494`, `MemoryFeedPanel.tsx` | H → P |
| [ ] S5 | Autonomy & guardrails: approval strip/default modes/exceptions, Spend link | R inspect; W global approval policy | `SettingsView.tsx:604` | H → P |
| [ ] S6 | Tools: extension list/detail, available/blocked states and configuration | R inspect; W tool/extension enable/install | `SettingsView.tsx:759` | H → P |
| [ ] S7 | Models: providers, Chat/Voice/Harness, Ollama, agent routing; pickers cancel; local errors | R inspect; W routing/download/service start | `SettingsView.tsx:1089`, `LocalModelsSection.tsx` | H → P |
| [ ] S8 | API keys: provider list, add custom provider modal, masked credentials, validation/cancel | R inspect; W save/test/remove credentials | `ProvidersSection.tsx`, `AddCustomProviderModal.tsx` | H → P |
| [ ] S9 | Devices: hub/companion, paired devices, pair QR/link, tailnet, messaging gateways/Gmail controls and bottom reachability | R inspect; W pair/revoke/connect/network settings | `SettingsView.tsx:1804`, `MessagingGatewaysSection.tsx`, `GmailConnectSection.tsx` | H; Q2/Q24 source-only followup → P |
| [ ] S10 | Services: Search providers, Polybot credentials, Fundamentals, Data sources | R inspect; W external keys/integration toggles | `SettingsView.tsx:1356`, `SearchToolsSection.tsx`, `DataSourcesSection.tsx` | H → P |
| [ ] S11 | Appearance: System/silver/dark, Möbius, density; try/restore reversible choices | L | `SettingsView.tsx:1380` | H → P |
| [ ] S12 | Shortcuts: every group readable, shortcut behavior where non-mutating | R | `SettingsView.tsx:1471` | H → P |
| [ ] S13 | Privacy & data: sovereign mode, diagnostics, crash export, egress/auth audit, backups, on-chain identity opt-in; inspect expanded views | R/L local export; W consent/global routing/restore/identity | `SettingsView.tsx:1499,1635,1738`, audit/backups/on-chain files | H → P |

## World — compact habitat

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] W1 | Arrival eye-level frame: host face visible, camera outside body, nameplate clear, planted floor/vista continuity; orbit/zoom | R | `world/forum/{ForumView,HabitatScene,HabitatNameplate}.tsx` | W2/W7/W8 historical defects; corrections merged → P |
| [ ] W2 | Overview ↔ Arrival view; entire connected terrace fits; inspect terrain edge, daytime star sky, warm lighting and foliage close-up | R | same, `HabitatSky.tsx` | H/N28; realism still open → P |
| [ ] W3 | Places open/select each: Arrival table, Garden archive, Maker workshop, Observatory, Observatory garden, Conservatory; close with focused Escape then reopen | R | `ForumPlaceRail.tsx`, `habitatLayout.ts:69` | H/N28 partial → P |
| [ ] W4 | Agent desk → World settings → System/Warm daylight/Lantern night; return System; compare water, foliage, silhouettes, shadow bands and sky | L | `ForumView.tsx:560` | N28 day/night; current P |
| [ ] W5 | Walk each destination, east/west ascent, water bridge, upper terrace to observatory bridge and back; height grounded, no edge drop or geometry clipping | L simulation | `ForumWalk.tsx`, `walkCollision.ts`, `habitatLayout.ts` | Source full-route tests; native complete bridge walk unmeasured → P |
| [ ] W6 | Walk hints visible; arrow keys/Shift, mouse/drag look, E nearby agent/archive, Escape; select body exits walk and frames actual position | L | `ForumView.tsx:508`, `ForumWalk.tsx` | W3/W6 fixes require live verification → P |
| [ ] W7 | Approach agents from multiple directions without entering heads; check nameplate/dock/desk use truthful consistent live status | L | `HabitatNameplate.tsx`, `HabitatAgentDock.tsx`, shared agent status | W1/W6 open questions → P |
| [ ] W8 | Select all 12 agents via body and Collaborators menu; distinct portrait/clothes/body; Details opens matching Meet; camera tracks selected actual body | R/L | `HabitatAgentDock.tsx`, `HeroAgentPortrait.tsx`, `agentProfiles.ts`, `../agents/roster.ts` | N28 subset; current P |
| [ ] W9 | Movement disclosure: Move agent → place/path, Cancel destination, Stop, Follow/Stop following, To workstation; close/reopen cancels targeting; work feed does not enable Follow | L simulation, no job dispatch | `HabitatAgentDock.tsx:43` | N24–N28 subset accepted → P |
| [ ] W10 | Ambient social observe, user move/Stop/Follow precedence, seated conversation precedence, leave/revisit and hidden/reduced-motion behavior | L | `habitatSocial.ts`, `ForumView.tsx`, `HabitatAgentDock.tsx` | N26 behavioral evidence; current P |
| [ ] W11 | Desk Meet: role, reported status, Working style, source meaning, orchestrator passport, Try a question and How agents work navigation | R draft preparation | `ForumView.tsx:568` | H → P |
| [ ] W12 | Learn: all numbered lessons, Use this question prefill; back to Meet keeps correct identity | R | `ForumView.tsx:573` | H broad → P |
| [ ] W13 | Evidence Arrival table: both choices and explanations, revisit reset | R local fictional demo | `HabitatLearning.tsx:22` | merged demonstrations; new live P |
| [ ] W14 | Evidence Garden archive: records expansion, both claims, explanations, revisit reset | R local fictional demo | `HabitatLearning.tsx:38` | new live P |
| [ ] W15 | Evidence Water workshop: Shade and Flow × each prediction; Inspect result; change/reset clears prior output | R local fictional demo | `HabitatLearning.tsx:46` | new live P |
| [ ] W16 | Brief Query/Job/Capability; empty disabled, draft outcome/criteria, Export brief and reported no-dispatch result | L local download | `ForumView.tsx:574` | N22 export; current P |
| [ ] W17 | Brief Ask/Ask to run job/Develop capability and Chat typed submit route into shared live conversation; test with bounded QA request only | W model/runtime dispatch | `forumQuery.ts`, `ForumConversation.tsx` | No blanket task-dispatch prior claim → P |
| [ ] W18 | Sit with orchestrator/leave; table camera and globe visibility; typed conversation panel and idle voice controls; closing returns to exploration | L camera; W text send/mic activate | `OrchestratorConversation.tsx`, `ForumView.tsx` | N12/N26 partial → P |
| [ ] W19 | Hide title/place/feed/hints, Restore cards; desk/dock/Places at narrow width; World → Settings → World state consistent | L | `useDismissible.ts`, `ForumView.tsx` | H/N24 partial → P |

## World — previous campus branch and additional views

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] WC1 | World settings → Previous campus; all its Places (Commons, Gallery, Build, Brain, Automate, Mesh gate, Reading grove, Maker court, Council garden, Arrival gardens, Conservatory, Maker hall, Debating theatre, Harbor); refind actual list and include any conditional additions | R/L | `world/forum/districts.ts`, `ForumView.tsx` | S inventory, old historical World only → P |
| [ ] WC2 | Campus Walk; Fly the skiff, arrow/Space/C/Shift, safe land/E disembark/Escape; return Compact garden | L simulation | `ForumFlight.tsx`, `ForumView.tsx:540` | S → P |
| [ ] WC3 | Mesh gate Enter/Return, walking through ring, real disconnected state clearly stated; no invented peers | R/L | `MeshAgora.tsx`, `ForumView.tsx:564` | S → P |
| [ ] WC4 | Place Open tool link routes to real matching shell workspace; unavailable destination notice; legacy World HUD if live route exposes it | R | `ForumAppView.tsx`, `world/{WorldView,WorldHUD}.tsx` | S → P |

## Fresh configuration wizard — separate profile/daemon required

A second bundle identifier alone does not isolate configuration: baseline and r29 used shared daemon `127.0.0.1:3001` and live configuration. The newer `record-r1` explicitly embeds review port `3002` and a separate compiled runtime root; package verification confirms that target, while actual isolated startup and a genuinely fresh wizard profile remain unverified. Do not flip live `wizard_complete` or overwrite live identity. Record the disposable backend/config identity before the wizard pass; a review bundle is not by itself proof of fresh configuration.

| Check | Flow and expected observation | Effect | Source | Prior → now |
|---|---|---|---|---|
| [ ] Z1 | Welcome/Get started; six-step progress, Back/Escape, keyboard tab/focus, 1024×640 and 1280×800; draft survives quit/resume | L draft | `wizard/WizardShell.tsx`, `StepFrame.tsx`, `wizardDraft.ts` | Fresh browser 5302 normal setup completed; native/keyboard/size/resume cases P |
| [ ] Z2 | Model Hosted provider: provider picker, key help/ref modes, empty/invalid key, test saved/new key, choose model, retry | W test/save/provider request | `wizard/StepModel.tsx` | S → P |
| [ ] Z3 | Model On this Mac: unavailable/up, Start Ollama, list/use model, download progress/cancel and failed/retry; avoid unnecessary model download | W service/download/config | same | S → P |
| [ ] Z4 | Model Decide later → Continue without a model; honest no-reply message persists in Review | L/W isolated config | same | S → P |
| [ ] Z5 | Agent blank/default, chosen name, too-long validation, every tone; continue/back retains draft | L; completion writes identity | `wizard/StepAgent.tsx`, `persona.ts` | S → P |
| [ ] Z6 | Code folders scan/look again, candidate selection, manual valid/invalid path, Add/remove, no-code Skip; saves only chosen roots | W isolated config | `wizard/StepFolders.tsx` | S → P |
| [ ] Z7 | Extras expand/collapse Web search and Local memory; Skip; service picker/key test failure; Ollama unavailable/progress/cancel readouts | W services/keys/models when used | `wizard/StepExtras.tsx` | S → P |
| [ ] Z8 | Review every summary/Change returns to Review; first message remains draft; Start completes exactly once | W identity + wizard_complete | `wizard/StepReview.tsx`, `WizardShell.tsx:81` | S → P |
| [ ] Z9 | Isolated save failure shows Retry/Continue anyway; successful restart bypasses wizard; continue-anyway warns setup may return | W controlled failure path | `WizardShell.tsx:190` | S → P |

## Current runtime evidence log

The following observations are reconciled from the dated CUA reports. Keep untested, blocked, failed and passed distinct. A source check, unit test, screenshot harness or old installed build is not a current native pass. Checkboxes remain unchecked because the reported interactions do not cover every requirement in their rows.

| Batch / build / surface | Items | Result / defect | Evidence |
|---|---|---|---|
| Source audit at b55bf214 | All rows inventoried | Runtime P | This matrix; no app UI controlled by audit worker |
| Installed baseline; `/Applications/Permagent.app` | H1, P2–P4, PE1, B1, G1, A1, BR1, R1, S2 — partial | Home opened; project Overview/Details/Kanban and card open/Escape; People Graph/List loaded 31 people; Build panels and Runs → History; all five Grow views loaded; Automate activity/detail; Brain Graph/List; Record project search/select/cancel; Persona pronunciation visible. Old build only; broader row requirements remain pending. | [Baseline CUA](CUA_BASELINE_2026-10-07.md) |
| Integration browser `:5301`; Chrome | SH4, W11 | Cramped Meet identity column and a Home click that stayed in World observed. Portrait and independent Button lifecycle fixes followed; the browser interaction cause was not established. Browser retest stopped when an unrelated Print window appeared. | [Baseline CUA](CUA_BASELINE_2026-10-07.md) |
| Native review r29; baseline daemon/helper reused | SH1, BR1, W1–W4, W11–W15 — partial | Startup and Brain restore; World arrival/overview, desk/settings and Warm daylight; readable stacked Meet label; Learn chapter change; both Arrival answers; Garden records/correct claim; one Water shade/prediction/inspect sequence. Night horizon remained dark/empty; daytime procedural cliff card failed visual review. Removal in source is not a rebuilt pass. | [r29 CUA](CUA_REVIEW_R29_2026-10-07.md) |
| Dedicated practice host; successful 20-second WebRTC run | Record transport fixture only | Both peers connected; both receivers decoded 98 video frames. Near audio received 1,002 packets with nonzero energy; far audio received the selected silence track. Permagent was not recording. Earlier connection failures remain recorded. | [r29 transport evidence](CUA_REVIEW_R29_2026-10-07.md#practice-webrtc-transport--subsequent-successful-run) |
| Record baseline microphone and direct-WAV checks | R3 below — incomplete/failed | Physical-headphone mic trial cannot prove fixture capture. Direct 24 kHz and 16 kHz WAV uploads both truncated later sentences. New native capture and corrected backend transcription remain pending. Frontend test results are not end-to-end acceptance. | [Record acceptance status](CUA_REVIEW_R29_2026-10-07.md#record-acceptance-status) |
| Running daemon and Settings picker | S7 — availability subflow only | Additive Haiku 5.5 provider registered; official inventory returned 14 models including Haiku 5.5; CUA picker showed the model. Defaults unchanged. No inference or long-session acceptance. | [Haiku runtime evidence](HAIKU_55_LIVE_REGISTRATION_AUDIT_2026-10-07.md#applied-and-verified-by-coordinator) |
| Integration browser `:5301`; Chrome, source hot reload, daemon 3001 | H4–H5, P2–P4, PE1, G1, A1–A3, BR1, HI1, SH5, S7 — partial | Home focus/state and Council fixes retested; project lenses/Edit Discard; People Graph/List/detail; five empty Grow lenses; Automate Learned and three draft modes; Brain controls; four History tabs; notifications; exact Haiku entry. No all-flow or native claim. | [Browser acceptance](CUA_BROWSER_2026-10-07.md) |
| Same browser; World and Build at 1289×618 | W3–W6, W11–W13, W16–W17, W19, B1–B2 — partial | Six Places/expected collaborators; day FAL archive/garden; Meet/Learn/Evidence/empty Brief; disconnected Chat; Walk W/D/Escape, no collision acceptance; System restored. Dock/rail overlap fixed and retested. Build three/all 64 history bounded with both panes visible; terminal desktop-required fallback CUA-confirmed. | [Browser acceptance](CUA_BROWSER_2026-10-07.md) |
| Same browser; Record picker | R1 only | Project without source cannot start; microphone + project enables Start; Cancel performs no capture. Capture/ASR/triage/cards remain pending. | [Browser Record picker](CUA_BROWSER_2026-10-07.md#record-picker) |

### Earlier build/test checkpoint — not runtime acceptance

As of 2026-10-08 01:40 UTC (October 7 local):

| Gate | Recorded state | Boundary |
|---|---|---|
| Coordinator full frontend | 476 files / 3,809 tests passed | Predates late card-source UI; does not complete any CUA row |
| Late card-source/type gate | Test-only Promise generic corrected after TS2322; coordinator TypeScript exit 0 and 14/14 tests passed | Late UI still absent from `record-r1`; native P |
| Native Rust | 4 system-audio + 3 runtime-target tests passed | Actual capture/isolated startup pending |
| `record-r1` native stage | Build exit 0, 128.39 s; 615 exact embedded assets; signed ARM64 | Excludes late `MeetingCardSource`; runtime review pending |
| Backend | Cold 30-minute guard stop; warm 64-second coordinator cancellation for schema fix | Neither attempt passed; daemon build session 13729 still running at checkpoint |
| Latest CUA lookup | `cgWindowNotFound` for Record Review, World Review and Practice | Desktop-state question pending; no new app-flow result inferred |

Exact stage identity, hashes and pending work are in [current checkpoint](CURRENT_CHECKPOINT_2026-10-07.md). Source is frozen for review; late card provenance needs a later frontend/native refresh.

### Record scope added after the original inventory

| Check | Flow and acceptance requirement | Current evidence |
|---|---|---|
| [ ] R3 | Record a synthetic two-participant practice call; preserve each speaker and late corrections/negations; end/save without losing the tail | Fixture WebRTC transport verified separately. Old microphone trial inconclusive. New isolated ASR retained all eight semantic checks across three 16k WAV fixtures. Actual Chrome-scoped capture returned verified-source READY/exit 0 but two WAVs were all-zero PCM despite WebRTC audio receipt; no ASR was attempted on silence. Native end-to-end acceptance remains open. |
| [ ] R4 | Save complete transcript to the selected project; durable Orchestrator triage catalogs notes and proposes relevant Kanban todos without starting their execution | Fresh original and two paraphrase HTTP variants passed 120/120 checks across six projects/twelve commitments: full notes, Brain catalogue, local-only provenance, stable replay and unexecuted cards. This supersedes earlier missing-commitment/Brain failures for these scenarios. Actual native capture-to-triage remains pending. |
| [ ] R5 | Retry save/triage after interruption; no duplicate note/cards, wrong-project leakage or repeated execution; local-only routing remains enforced | Source/test checks are distinct from live acceptance. Controlled native retry and project/privacy checks pending. |
| [ ] R6 | Dismiss Record receipt, reopen a generated card, inspect its durable source, open the correct project note and actual triage session; handle missing/deleted source | Late source affordance exists; latest coordinator TypeScript and 14 targeted tests passed. Browser 5302 passed generated-card Source meeting → actual triage session → complete source note with linked memory. Native interaction and missing/deleted-source UI remain P. |

The full source-derived matrix remains open. Browser observations above narrow the gaps but do not replace isolated fresh-wizard coverage, native calendar ranges, complete World routes/forms, capture-to-triage acceptance or final rebuilt scenery. The successful observations above do not imply that every flow passed.

### Latest isolated runtime checkpoint

Browser 5302 completed normal pairing and fresh setup via Decide later, RecordQA/Balanced, folder/Extras skips and Start; existing local model configuration stayed unchanged. On this Mac appeared after provider discovery completed. A source loading status and deferred-provider choice-preservation regression now pass (wizard 9/9, TypeScript 0); native r2 predates that small status change. Review displayed an ellipsis at Start, which remains a follow-up observation.

The refreshed native r2 package verifies 614 assets. Original/paraphrase A/paraphrase B HTTP scenarios each pass 40 checks; guarded-runtime disk-floor interruptions remain an operational limit, and actual scoped capture failed with silent PCM. See [current checkpoint](CURRENT_CHECKPOINT_2026-10-07.md) for receipts and exact boundaries. No overall Record or all-flows pass is claimed.

### Final closeout evidence — October 8

- Final daemon build passed 383.24 seconds (minimum 23.914 GiB); native r3 passed 48.29 seconds and verified 614 assets. Final stage is retained; r2 was removed. See [final handoff](HANDOFF_2026-10-08.md) for hashes, runtime guard and cleanup boundaries.
- Final-build synthetic meeting HTTP original scenario passed 40/40, after the earlier three variants' 120 checks. Aggregate 160 spans different builds; only the last 40 used the final daemon. Three older missing note-memory associations were repaired by exact idempotent save replay without inference.
- CUA 5302 QA Amber showed two tasks, completed Source meeting receipt, actual Orchestrator transcript/two-todo JSON, full note with negations/tail and one linked memory; Kanban 2 Backlog/0 Doing/0 Done. View in Brain showed correct body only as preview with source unknown. Browser provenance navigation passes for this fixture; native capture remains failed (zero PCM), and no native Record pass is claimed.
- Normal 5301 Settings rechecks passed named notification/Remote access/service switches and proper Gmail Not connected/Remote tunnel Off after Vite auto-reload. World Shared Chat connected with existing conversation and composer; no new send/response round trip is claimed.
- Home gallery Try Relationships → Keep current and Add card → Close/Done left layout unchanged; Calendar Month focus/collapse rechecked. [Browser screenshots and details](CUA_BROWSER_2026-10-07.md).

All unchecked multi-part rows remain open where requirements exceed these observations. Native flows, duplex capture, permission states, source/graph resolution and deeper branches remain explicit gaps.
