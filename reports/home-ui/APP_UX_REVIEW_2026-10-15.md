# Permagent: information architecture and page-by-page UX review

Reviewed 2026-10-15. This is an expert review, not a completed user study. Recommendations below are Permagent-specific design judgments informed by the sources; Apple does not prescribe these exact page layouts or item limits.

## Product direction

**Connect the source, let the agent understand it, show the person what matters.** A person should primarily supply intent, access, corrections and consequential decisions. They should not maintain a second copy of information already present in their code, connected services, conversations or project records.

The People graph is an explicit product strength and user preference: **keep the user at the center, keep Graph as the main experience, and preserve the enjoyment of seeing relationships grow.** Improve its surrounding controls and person inspector. Do not replace it with a conventional CRM table or turn low recorded activity into a judgment about a relationship. The List remains an accessible, efficient alternative.

Three levels of information should have distinct jobs:

1. **Orient:** where am I, what changed, what needs me, what is happening next?
2. **Inspect:** select a specific person, task, finding, recommendation or record; see the relevant context and evidence together.
3. **Configure:** change connections, policies, schedules and advanced options when needed.

This is not a rule that everything must fit on one screen. A document, transcript, board or evidence library can be long. An overview should not grow indefinitely with its underlying database. Preserve useful visual experiences, including People and World; remove unnecessary reading and setup effort around them.

## Research and implications

| Source reviewed | Guidance relevant to this product | Application to Permagent |
| --- | --- | --- |
| [Apple: Design foundations from idea to interface](https://developer.apple.com/videos/play/wwdc2025/359/) — transcript, especially Structure, Navigation and Content | Establish purpose and orientation before styling; group content by meaningful relationships, progress or time; use components appropriate to the information. | Define a job for each tab. Resolve repeated content and contradictory labels before polishing cards. |
| [Apple: Design principles](https://developer.apple.com/design/human-interface-guidelines/design-principles) | Focus, familiarity, clear feedback and recoverability support a coherent experience. | Preserve the graph's personality while making selection, actions and outcomes predictable. |
| [Apple: Layout](https://developer.apple.com/design/human-interface-guidelines/layout) | Importance, alignment and grouping establish hierarchy; layouts must adapt as space changes. | Put decisions and meaningful changes first. Collapse a secondary inspector before squeezing primary content. |
| [Apple: Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars) | Recognizable destinations, concise grouping and shallow hierarchies make navigation understandable. | Keep stable tab names and deep links. Show labels when space permits; do not make users learn thirteen abstract icons. |
| [Apple: Split views](https://developer.apple.com/design/human-interface-guidelines/split-views) | A list and its selected detail provide context without repeated navigation; selection and sizing should remain clear. | Use list/detail for findings, recommendations, automations and records. On narrow screens open the selected detail with a clear return path. |
| [Apple: Disclosure controls](https://developer.apple.com/design/human-interface-guidelines/disclosure-controls) | Reveal detailed options when relevant, with labels that explain what opens. | Summarize security on Overview and open its findings intentionally. Do not replace every long card with another unlabeled accordion. Apple's single-disclosure-button guidance concerns that control, not an absolute ban on multiple expandable list rows. |
| [Apple: Entering data](https://developer.apple.com/design/human-interface-guidelines/entering-data) | Collect available information from the system; use defaults, choices and immediate validation to reduce unnecessary input. | Discover stack from manifests; derive repo links from known connections; propose a brief from existing records; ask only for unknown account identity or intent. |
| [Apple: Settings](https://developer.apple.com/design/human-interface-guidelines/settings) | Good defaults reduce configuration; avoid setup questions the system can answer; keep task-specific controls in context. | Settings owns infrequent preferences and access. Pages show connected status and a contextual repair link instead of the full setup form. |
| [Apple: Onboarding](https://developer.apple.com/design/human-interface-guidelines/onboarding) | Make necessary onboarding focused and interactive; teach near the relevant action. | Project creation starts with a folder/repository or a non-code project. Discover information in the background and show useful results before optional setup. |
| [Apple: Generative AI](https://developer.apple.com/design/human-interface-guidelines/generative-ai) | Offer specific value, truthful progress and ways to refine or revert results. Distinguish generated output and preserve user agency. | Label discovered facts with source and time; distinguish declared dependencies from connected accounts; allow correction; never overwrite a manual value silently. |
| [Apple: Searching](https://developer.apple.com/design/human-interface-guidelines/searching) | Make important search discoverable and its scope clear; a consistent entry point helps people find content. | Search Projects, People and records locally; make global search distinguish project, person, task, memory and conversation results. |
| [Apple: Lists and tables](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables) | Structured rows support scanning and comparison, with detail available on selection. | Use rows for tasks, findings and saved records; reserve large cards for content that benefits from a preview or visual identity. |
| [Apple: Charting data](https://developer.apple.com/design/human-interface-guidelines/charting-data) | Highlight the question a chart answers, keep the initial presentation understandable, and provide context and accessible alternatives. | Grow leads with a meaningful trend and its period. Distinguish zero, missing data and insufficient evidence. Graph views retain a readable alternative. |
| [Apple: Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars) | Prioritize frequent actions; avoid overcrowding and make control meanings clear. | A page gets one visually prominent action. Less frequent actions move into contextual menus, without hiding essential decisions. |
| [Apple: Notifications](https://developer.apple.com/design/human-interface-guidelines/notifications) | Notifications should be timely, concise and valuable; repeated alerts for the same thing are disruptive. | Watcher reports a material change once, with a destination and next step. Routine checks belong in history. |
| [Apple: Loading](https://developer.apple.com/design/human-interface-guidelines/loading) | Show progress and available content while work continues; avoid unexplained blank screens. | Loading a list must not briefly claim there are no records. Background discovery should explain what is being read without blocking navigation. |
| [Apple: Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility) | Legible contrast, larger text, keyboard operation, meaningful labels and reduced motion benefit everyone; color alone cannot carry meaning. | Keep visible labels for status, keyboard alternatives to graph/drag actions, adaptable text and reduced-motion graph transitions. Verify actual rendered contrast rather than guessing from screenshots. |
| [NN/g: Progressive disclosure](https://www.nngroup.com/articles/progressive-disclosure/) | Put frequently needed information first; make routes to secondary information obvious; excessive layers create their own navigation burden. | A compact Overview links to named destinations. Avoid a maze of nested accordions or moving daily actions into Settings. |
| [NN/g: Recognition and recall](https://www.nngroup.com/articles/recognition-and-recall/) | Visible context and recognizable options reduce dependence on memory. | Keep the selected project/person visible, label unfamiliar icons, and offer concrete agent actions instead of requiring users to invent a prompt. |

Apple's current Markdown HIG pages were read where its JavaScript documentation shell was not extractable. The WWDC session was reviewed through its transcript, not watched. No copying of Apple's visual styling or adoption of a new UI framework is implied.

## Review method and evidence

The installed native application was inspected using read-only navigation and screenshots; component source was reviewed to identify conditional states and behavior. The installed artifact and source worktrees are not identical. Home's new implementation was separately inspected in an isolated browser harness using synthetic fixtures, including narrow, busy, empty and error states. No live user screenshots or personal records are included in this report.

Evidence labels below: **N** = native page inspected; **S** = source inspected; **F** = synthetic rendered fixture checked. Source inspection is not visual acceptance. Several native pages temporarily omitted content from accessibility extraction despite showing it visually; this requires a focused accessibility investigation, not a claim that all those controls are definitively inaccessible. Brain's graph remained visually blank during this visit. Agents remained loading during the visit. Both need reproduction with request timing before assigning a cause.

Notifications was source-reviewed because opening and dismissing its tray marks notifications read. Recording, sending a chat, changing settings, pairing devices, publishing and running workers were not exercised against the live app.

## Page-by-page review

### Navigation and Home

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| Global sidebar — N/S | Many icon-only destinations require recognition learned over time. Record is an action among navigation destinations. | Preserve destination IDs and shortcuts. Offer a labeled rail at comfortable widths and retain explicit collapse control. Group everyday work, exploration and utilities without hiding active work or changing tabs automatically. Keep Record clearly an action. |
| Home overview — N/S/F | The installed page repeated attention information across several cards and had excessive vertical space. | One decision entry, then Today (calendar and commitments), then Activity (in-flight work and recent conversations). Custom arrangements remain supported. The resumed implementation does this; a conversation ending is no longer styled as a verified success. |
| Home focus/card expansion — S/F | A necessary place for richer detail; it should not be the default for every card. | Keep Expand/Collapse, keyboard return and local state continuity. Name the destination and preserve its reading position. |
| Home layouts/customize — S/F | Useful personalization can become onboarding work if required before the page makes sense. | Ship a useful default. Keep gallery and customization optional. Migrate only exact old defaults on read; do not rewrite a user's custom layout. |
| Decision Inbox — S/F | A large pending queue can become another unranked workload. | Group by consequence and source; show request, reason, scope, age and exact available action. Keep all pending items reachable. Never equate opening an item with resolving it; handle stale/resolved items explicitly. |
| Calendar/connection states — S/F | An empty calendar can occupy a large card with multiple paragraphs. | One clear state and a next step; permission and troubleshooting detail on demand. Distinguish no events from not checked, inaccessible and failed. |

### Projects

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| All Projects — N/S | A large card grid gives every project similar weight. The header teaches dragging to shelve projects before helping find current work. | Search, recent/pinned projects, and a compact active collection. Each project gets purpose plus a meaningful attention or work cue. Keep full collection and paused/archive access. Supply a menu/keyboard alternative to drag. |
| Project Overview — N/S | Summary, up to eight full security findings, Watcher prose, key facts, activity, the full task list and publishing configuration create an effectively unbounded page. | Short purpose; a bounded attention/security summary; a few current tasks; recent meaningful changes. Full findings remain one clear action away. Full tasks belong on Kanban; identifiers, timestamps and publishing configuration belong in Details. Responsive single-column ordering must prioritize attention. |
| Project Details — N/S | Ten different panels, including approval thresholds, are shown together. Empty panels tell users to add information the system may already possess. | A stable local section chooser: **Stack & connections**, **Knowledge**, **People & context**, **Project settings**. Start with the detected stack and connected sources. Keep sections independently reachable without a long scroll or nested disclosures. |
| Stack & connections — N/S | Manual service entry despite a connected code folder. | Read known manifests automatically on entry/root change; show service, evidence file and read status. Manual account labels and service links remain editable. Unknown does not mean absent; partial scans say so. Reading manifests must not run repository commands or parse secret files. |
| Knowledge: Documents, Notes, Memories, Code — N/S | The same document can appear as a note, linked memory and indexed source; long excerpts compete. | One selected collection at a time, compact titles with source/date, a detail reader, and source links. Offer note composition on intent. Index permitted project material automatically after connection; show progress/failures. Retain correction and removal controls. |
| People & context: People, Ecosystem, Market — N/S | Three unrelated empty states demand setup. | Associated people first; sourced ecosystem observations second. Market appears when a series exists or the user asks to track one. Suggest associations with evidence; do not invent relationships, competitors or forecasts. |
| Project settings: key facts, approval, publishing — N/S | Low-frequency configuration currently dominates glanceable pages. | Keep project identity, verification rules and publish steps in a named section. Resource links stay with Stack & connections. Detect facts; keep security policy and deployment steps explicit. Do not auto-grant execution privileges to make setup look simpler. |
| Kanban — N/S | Many columns and terminal-state cards increase horizontal and vertical scanning. | Keep the board for managing the whole workflow. Offer active-work filtering and terminal-state history with counts. Review/failed items remain discoverable. Card selection preserves project and board position. |
| Card/goal detail — S | Rich execution metadata is valuable after selecting a task. | Lead with outcome, current state and next required action. Evidence/artifacts and history follow; raw trace and advanced metadata are secondary. Preserve exact provenance and distinguish complete execution from verified result. |

### People

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| People graph — N/S | The centered graph is distinctive and personally meaningful. Initials, crowded clusters and quiet-contact fading can make interpretation harder. | **Preserve the graph and the central You node.** Fit the network around the inspector; reveal names on focus/hover and selection; highlight the selected person's connections; use restrained new-connection cues with reduced-motion support. Keep the network stable while filtering. Explain what an edge means and its evidence. |
| People list — N/S | Useful search and filters already exist. Repeated “never/no interactions” language dominates sparse records. | Preserve the alternative list. Use neutral “No recorded activity” once per row where useful. Sort/filter by a user's concrete task, not an opaque relationship score. |
| Person detail — N/S | Selecting a person opens roughly fifteen editable fields before meetings and recent context. | Read-first inspector: name/context, latest interaction, upcoming meeting, follow-up, shared connections/projects. Edit reveals contact fields. Enrichment uses already connected sources, labels provenance and lets the user correct it; public online lookup remains a deliberate action. |
| Add/merge person — S | Manual identity work can duplicate known people. | Prefill known fields, show possible matches and the concrete fields a merge will retain. Keep merges reviewable and do not merge merely because names match. |

### Build

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| Build workspace — N/S | A shell and empty browser can occupy almost the entire page; the agent's current work is a small status line. | Lead with selected project and current run/next action. Keep terminal and browser as working tools, independently resizable. Reopen the project's known preview when appropriate; show a concise idle start state without requiring model configuration first. |
| Recent runs / run detail — N/S | Collapsed history is useful, but message counts are not outcomes. | Compact run rows: task, real state, time and artifact/result. Selected run shows evidence, changes and logs. Keep history collapsed when actively working. |
| Browser tabs/bookmarks/downloads — N/S | Several control layers and an empty-state instruction to configure a search template add effort. | Stable browser controls and project context; sensible search default; one download result with suggested filing destination. Do not auto-publish or send downloaded content. |

### Grow

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| Actions — N/S | One recommendation includes rationale, instructions, a full coding-agent prompt, model choice and measurement controls. Several near-duplicate recommendations can coexist. | Ranked compact rows with expected benefit and evidence date. Selecting one opens rationale and proposed implementation. A concrete “Start this change” action uses the configured agent; prompt inspection remains available. Dedupe related recommendations and close those superseded by verified work. |
| Results — N/S | Selected-project results and cross-project performance share one long page. | Project results first: measuring, helped, no detectable change, insufficient evidence. A separately selected all-projects scope shows aggregate results. Keep uncertainty and overlapping changes visible; a successful commit is not proof of growth. |
| Strategy — N/S | Long narrative sections repeat value proposition, channels, positioning and aspirational metrics. | Short current strategy with dated evidence and assumptions, then a selected section editor. Agent drafts from the project brief, code/product context and analytics; user corrects intent and approves strategy changes. Distinguish targets from measured facts. |
| Calendar — N/S | Connection setup, a post editor, schedule controls and media feedback all appear inline. | Calendar/list of drafts and scheduled posts; selected post opens editor. Connected-account status is compact. Show a clear blocked state before scheduling when a publishing connection is missing. |
| Analytics — N/S | Repeated headline metrics, multiple breakdowns, two funnels and setup instructions create a long page. | One consistent period and source/freshness line; a few key metrics and one trend; selected breakdown/funnel below. Suggest a conversion funnel from observed events and site code, explain the proposal, and allow correction. Missing events remain missing, not zero. |

### Finance and Automate

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| Finance overview — N/S | Holdings, scanner controls, optional desks, credentials, household imports, watchlist and note forms compete. | Separate **Household**, **Holdings**, **Research** within Finance. Keep display currency and data freshness consistent. Show the selected area's useful summary before setup. |
| Household — N/S | Import is the only meaningful action before transactions exist. | One import/connect entry, progress, then categorized transactions and cash-flow summary. Agent proposes classifications and flags uncertain entries for review. |
| Holdings/trade ledger — N/S | Empty ledger displays currency zeros that can be mistaken for a connected portfolio. | State coverage explicitly: local recorded trades versus linked data. Keep trade recording as an intentional editor and preserve the distinction from trading. |
| Research/picks/watchlist — N/S | Scanner operations and lengthy methodology dominate empty content. | Watchlist and dated observations first; selected observation shows methodology and evidence. Connection/configuration lives behind its status. Optional trading-related desks remain explicit opt-ins. |
| Automate overview — N/S | Jump links resemble tabs but all sections remain on a long page. “Agents at work” lists idle schedules, followed by overlapping recent activity and automation lists. | Actual views for **Automations**, **Activity**, **Skills**. Show urgent failures once, active runs when present, then schedule rows with last outcome and next run. Idle jobs do not need a large at-work card. |
| Automation detail/create — S | Schedule mechanisms and runtime identifiers are exposed early. | Intent, readable schedule/timezone, inputs and last result first. Agent proposes schedule and task from intent; user can review before activation. Cron and raw configuration are advanced. |
| Learned/installed skills — N/S | Verbose generated names, match counts and installed capabilities compete with user-owned automations. | Searchable library grouped by purpose, showing what a skill does and where it came from. Suggestions stay distinct from installed/enabled behavior; inspect details before activation. |

### World and Brain

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| World scene/navigation — N/S | The spatial environment adds personality; Places, collaborator controls, appearance, movement and work feed are separate overlays. | Keep the environment optional and enjoyable. One predictable inspector for the selected place/agent, an accessible directory, and a direct return to the underlying record. Walking animation must not imply that real work is executing. |
| Arrival table — S; camera destination N | Can repeat Home's goals, decisions and requests. | A concise welcome/current-work summary and brief composer; link to the same authoritative decision/task records. |
| Garden archive — S | Memories, notes, librarian activity and inspection can compete. | Selected source/record drives the inspector. Keep evidence and source navigation together; background librarian logs secondary. |
| Maker workshop — S; scene N | Goals, spend, skills, results and requests span several information types. | Current work and verified result shelf first; selected work opens its evidence. Keep learning/capability details secondary. |
| Observatory — S | Forecasts, Council reports and worker activity need distinct meaning. | One selected question/series with time range, uncertainty and supporting evidence. A refused forecast is an informative result. |
| Observatory garden — S | Reflective scenery and saved hypotheses have different purposes. | Preserve the quiet environment. Make hypotheses easy to retrieve, with clear provenance; sky/scenery is not a performance indicator. |
| Conservatory — S | Growth results can become a second analytics dashboard. | A concise interpretation of measured interventions, linked to Grow Results. Plant appearance must not imply business health unless an explicit validated mapping exists. |
| Brain graph — N/S | During this visit the graph area remained blank; controls were visible but not available through AX extraction. | Reproduce loading/render behavior and provide an honest recoverable state. Preserve graph exploration, selected-node context and a complete list alternative. Do not imply empty knowledge while data is pending. |
| Brain list/search/detail — S | Rich source information is available; it can become a long collection of excerpts. | Search and scoped filters first, compact result rows, selected evidence reader. Explain why a result matched, source/date and linked entities. Correct/forget actions remain discoverable. |

### History, notifications and communication

| Page / evidence | Finding | Proposed structure and automation |
| --- | --- | --- |
| History Sessions — N/S | First load briefly said “No sessions yet” while the header already showed a nonzero total. It later populated. Source has an empty-state condition without initial loading state. | Show initial loading explicitly. Keep conversation/agent/system filters and search. Selected preview should use the actual available content width, not only a very wide window threshold. |
| History Downloads — N/S | Every file repeats three routing buttons; the header explicitly leaves all routing to the user. | File list and preview, with suggested project/source association from context. Offer one reviewable filing action and preserve manual destination choice. Detect duplicates before creating repeated records. |
| History Activity — N/S | Raw event trace includes navigation/config churn, which is hard to interpret as work. | Meaningful outcomes grouped by run/project first; raw trace is a diagnostic view. Add source/status filters and stable timestamps. |
| History Spend — N/S | Budget configuration fills much of the initial page before the project breakdown. | Period total and budget status first, then project/session breakdown. Edit caps intentionally. Keep currency, attribution gaps and period explicit; do not conflate token totals with user value. |
| Notifications tray/toasts — S | Source marks all read when an item is activated or the tray is dismissed, including unrelated items. Repeated Watcher notifications can accumulate. | Distinguish viewed, read and resolved. Mark the actual selected item; offer explicit Mark all read. Dedupe the underlying event, retain unresolved decisions, and group low-priority updates. |
| Text chat / agent activity — S | Chat can expose tool detail and implementation process as primary content. | Answer/outcome first with artifacts and required decision. Expand tool evidence and trace on request; keep cancel/retry and current activity clear. Agent should use relevant project/memory context automatically within access already granted. |
| Voice — S | Long requested answers were previously cut by ordinary reply budgets; spoken and textual output need continuity. | Short by default, honor explicit depth requests, provide text transcript and stop controls. The resumed voice patch passed its backend tests; native acceptance is still separate. |
| Meeting recorder / saved meeting — S | Source picker, capture, save and triage are distinct states and must remain understandable. | Show selected source and project before Start, audible-source feedback during recording, then transcript/summary and proposed tasks. Autofill likely project/participants with a correction path; never begin capture merely by opening the recorder. |

### Every Settings page

| Page / evidence | Finding | Proposed structure |
| --- | --- | --- |
| Preferences — N/S | Repository roots, identity/defaults and notification kinds share explanatory prose. | Group personal defaults, project discovery roots and notification preferences. Concise labels, defaults from the system where possible, contextual explanations. |
| Persona — N/S | Name/voice/tone share a long page with pronunciation tools and optional on-chain identity. | Name, voice and tone first. Pronunciation opens as a focused editor. Optional identity integrations remain a secondary section. |
| Agents roster/detail — N/S | The roster did not finish loading during the visit. Source divides background workers, goal workers and capabilities, then detail includes grants, secrets and work review. | Searchable roster with purpose, availability and last meaningful outcome. Selected agent shows behavior/schedule; access and diagnostics are secondary. Show load failure and retry instead of indefinite loading. |
| Memory — N/S | Twenty source controls each carry repeated explanatory status and navigation. | Overall coverage and exclusions, then grouped searchable sources. Source detail carries counts, recent evidence and controls. Do not equate “not yet fed” with broken. |
| Autonomy & guardrails — N/S | Jargon and disabled upcoming modes obscure which mode actually runs. | Current behavior and consequence first; clearly label available controls and unavailable modes. Link exact pending decisions. Remove metaphorical “more rope” wording. Preserve approval enforcement. |
| Tools — N/S | A large capability-name block repeats the detailed tool inventory; a link still says “Search & tools.” | Connected/enabled tools with search and detail. Repair only missing dependencies needed by the selected task. Use current destination names. |
| Models — N/S | Routing, provider setup, two local-model systems and agent roster pointers compete. | Current routing summary; recommended configured defaults; advanced role overrides and local downloads on demand. Explain effective fallback adjacent to the affected role. |
| API keys — N/S | Connected-first tabs already help; repeated introductory prose and custom-provider work add noise. | Keep Connected first, add/search provider when requested, configuration in one focused form. Explain storage and use accurately rather than promising keys never leave the device. |
| Devices — N/S | Hub explanation, paired devices, tailnet setup, pairing and messaging gateways create a long configuration page. | Current connection health and device list first. Pair device opens a focused flow. Advanced network and messaging connections are separate named sections. |
| Services — N/S | Search credentials, financial desks, email OAuth setup and a large public API catalog are combined. | Connected services first; searchable add-service catalog grouped by task. Explain required access at connection time. Avoid asking ordinary users to navigate a public API taxonomy to make the agent useful. |
| Appearance — N/S | Theme, brand animation and density are all visible. | Theme and density first, optional visual effects secondary. Honor system motion/contrast preferences and make overrides unambiguous. |
| Shortcuts — N/S | A concise reference is already appropriate. | Preserve grouped shortcut reference. Keep labels synchronized with actual behavior; don't advertise unavailable rebinding. |
| Privacy & data — N/S | Core boundary controls share a long page with extensive provider/URL diagnostics. | Plain current data boundary and applicable controls first. Logs are a searchable diagnostic destination; backups/export are a separate group. Replace internal “provider choke point” wording with user consequences. |

## Watcher: proposed useful behavior

The source currently checks project notes/cards over a rolling seven-day window, counts stack entries, identifies old unarchived cards, generates a short note, stores it and sends a briefing. It can repeat unchanged facts; the stale-card query includes terminal states. UI deduplication by card IDs cannot prevent repeated model calls, memory entries or notifications.

Proposed contract:

- Observe a defined source; record when it was checked and what actually changed.
- Emit a finding only when it introduces a material delta, an unresolved risk that crossed a meaningful threshold, or a requested digest.
- Give each finding a stable identity, evidence, time, reason it matters and a concrete next action. Use normal product language such as “Changes to review”; retain Watcher as attribution/detail.
- Link to the canonical task/finding instead of creating another parallel to-do list. Exclude completed/cancelled work from stale-work nudges.
- Quiet is a valid successful outcome. No change should not create another memory and interruption.
- Never label a failed/partial check “all clear.” Keep check health separate from project health.
- Preserve source facts and manual corrections; agent proposals remain distinguishable from actions actually taken.

## Automation boundaries and first implementation

| Agent can derive | Person supplies or decides | Required feedback |
| --- | --- | --- |
| Frameworks, languages, declared services, repository metadata | Which source to connect; missing account identity | Evidence path, scan time, partial/error state; no claim of verified deployment |
| Draft purpose/brief from existing project material | Desired outcome and corrections | Draft attribution, source links, explicit manual override |
| Contextual associations and possible duplicates | Ambiguous identity/relationship decisions | Why the match was suggested; no silent merge |
| Funnel candidates, duplicate actions, implementation evidence | Business goal and consequential publishing/deployment | Proposed change, evidence, real execution/result state |
| File destination, schedule defaults, routine labels | Uncertain destination or schedule intent | Reviewable suggestion and correction/undo where supported |

First implementation priority is the explicit Projects feedback: bound Overview, group Details by task, and automatically derive the stack. Home's recovered redesign is verified separately. Later page changes should follow the contracts above, with one coherent page verified before expanding scope.

## Acceptance and evaluation

These are proposed product test targets, not measured outcomes or universal research constants:

- At a typical desktop workspace, a project overview with hundreds of records still shows its purpose, attention summary and current work before the user must scroll through full records. More records do not produce proportionally more overview height.
- A connected code project displays detected stack facts without manual service entry; missing root, malformed manifest, partial scan and request failure have distinct truthful states. Switching projects cannot show another project's detected data.
- Every overview preview has an obvious destination for the complete collection. No truncated security report loses access to its evidence or action.
- A person can select someone in the centered graph, understand the relationship and find a follow-up without entering edit mode. Selection remains recognizable after filters or window resizing.
- No-change Watcher checks produce no duplicate alert or knowledge entry; completed work is not called stale.
- Verify empty, loading, failed, stale, populated, long-text and large-data conditions. Test narrow content width, dark/light themes, keyboard-only navigation, focus return and reduced motion. Measure contrast from rendered styles.
- Conduct task-based user sessions: locate the project needing attention; explain a finding; connect a repository and correct one detected fact; find a person through the graph and inspect a shared connection; choose a growth action; recover a previous result; change a budget. Record completion, wrong turns, repeated reading and confidence. A heuristic review cannot substitute for this.

## Delivery status

Research covers every primary tab, principal subpage and all 13 Settings pages. The original evidence labels remain historical; new implementation and rendered review are recorded in [the UX DAG verification](../ux-dag/verification.md).

Home and Projects were consolidated into `world/production-20261006`. The subsequent DAG implements the page structure, loading/read-state fixes, People inspector and stable graph filtering, selected Automate/Grow/Finance areas, History controls, Settings disclosure/recovery, and Watcher evidence policy. The final UI suite passes 564 files / 4,625 tests; TypeScript and the production UI build pass. The daemon library suite passes 1,208 tests (five ignored; the isolated HTTP/browser test passes separately). Real browser review also caught and verified a fix for an oversized central You marker in sparse networks. See [dag.json](../ux-dag/dag.json) and the [verification receipt](../ux-dag/verification.md) for exact coverage, an interrupted staging repeat followed by a clean pass, and native/live limitations.

This remains an expert review, not a human usability study. Wider agent provisioning, automatic funnel inference, download classification/durable deduplication and inferred account identities are recommendations beyond the implemented UI/discovery contracts. No release or live installation is claimed.
