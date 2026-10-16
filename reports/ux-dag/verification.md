# UX DAG implementation and verification

Canonical source: `world/production-20261006` in `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`. This continues the recovered Claude work. UI checkpoint: `57d8e125`. The live application, live profile and port 3001 are unchanged. No install, deployment, publishing, messages, recordings, pairing or worker activation is part of this verification.

## Implemented contracts

| DAG | Page / behavior | Verification path |
| --- | --- | --- |
| D01 | Home has bounded previews. Projects Overview has a brief, two security previews, five current tasks and four activity rows, with full destinations. Details selects Stack, Knowledge, People & market or Settings. Local manifests populate detected stack with source evidence; manual account records stay separate. | Earlier Home/Projects receipt; `DetectedStackPanel`, Overview, Details and task/security navigation suites. New daemon full-router/auth test and dedicated staging HTTP test below. |
| D02 | History distinguishes loading, empty and failed refresh; old records survive a refresh error and late requests cannot win. Closing notification tray preserves unread items. Activation reads only the selected notification; Mark all read is explicit and does not resolve decisions. | Session store, SessionsList and NotificationHost tests. |
| D03 | Watcher ignores terminal/archived stale cards; stable evidence suppresses repeated model calls, knowledge and briefings. A quiet successful review records its evidence without an insight. Failure is distinguishable from no change. Only Watcher metadata keys are updated. | Watcher Rust SQL/hash/metadata/parser tests; Overview stable-id and failure tests. Rust execution remains an open gate until recorded below. |
| D04 | Projects search includes shelved projects; status select provides a keyboard alternative to dragging. Board selects Active, History or All; a terminal-card deep link reveals its scope and keeps its project. | Projects drilldown, Kanban card-detail, task and navigation suites. |
| D05 | People keeps Graph primary, You at its center and the list alternative. Filtering preserves layout/camera inputs. Selecting a person opens context before editing; drafts survive hiding the edit form. A narrow inspector fills its workspace and closes back to the graph. | People graph, geometry, selection, PeopleView and PersonDetail suites; synthetic browser review. |
| D06 | Automate selects Automations, Activity or Skills. Active work is distinct from idle schedules. Healthy status is quiet; problem destinations retain their existing wiring. | Automate wiring/run/resume/delete/detail suites; browser view selection. |
| D07 | Grow actions are compact until reviewed; evidence, prompt and execution controls remain accessible. Results separates project/all-project scope. Strategy selects a pillar or Brand while preserving drafts. Calendar connections are disclosed. Analytics leads with measured pageviews and discloses source setup, coverage and funnel. | Grow action/result/strategy/calendar/analytics suites. Subscription selectors say `w/ Subscription`; Permagent says `API / Local`. Chevron inset visually checked; launch commands unchanged. |
| D08 | Finance selects Household, Holdings or Research. Empty holdings clearly describe a local ledger; optional desks remain intentional. | Finance tests retain import, currency, trade-recording and opt-in boundaries; narrow Silver browser review. |
| D09 | Existing Build project/run/idle guidance and recent-run destinations are preserved and regression-tested. History preview uses container width. Downloads disclose filing options and prefill an existing project association. Activity defaults to work events with source/status filters and an explicit raw trace. Spend discloses cap editing. | Build, sessions, inbox routing, trace, History and Spend suites. |
| D10 | Brain graph requests time out truthfully, preserve cached data on failure, and reject stale responses. A renderer initialization failure offers List and retry. World places and canonical task/decision/result destinations are preserved. | Deadline, Brain, full World routing/interaction suites; synthetic rendering. The cause of the earlier installed-app blank graph is still unproven. |
| D11 | All 13 Settings pages reviewed. Narrow layouts use a labeled page picker. Infrequent setup is disclosed; Tools and Memory are searchable; Agents has timeout/retry. Privacy treats unknown/unconfirmed status as unknown and waits for daemon confirmation before describing a changed boundary. | Settings, Agents, common control and Privacy regression suites; narrow Privacy browser review. |
| D12 | Stable sidebar/deep links and explicit communication controls retained. Notification read state is separate from decision resolution. Recovered voice long-answer behavior remains intact. | Navigation, notification, chat/voice/meeting and event-deduplication suites. |

Settings individually: Preferences retains current defaults; Persona discloses pronunciation/integration; Agents exposes search and recovery; Memory adds search; Autonomy uses plain-language policy descriptions; Tools orders enabled entries and offers search/retry, with duplicate capability names disclosed; Models discloses local installation/downloads; API keys retains connected providers first; Devices discloses pairing/network configuration; Services discloses financial services and the public-data catalog; Appearance discloses visual effects; Shortcuts keeps its readable reference; Privacy discloses logs/diagnostics/backups and fixes unknown/save-failure states.

## Automated gates

- Earlier focused batch: 47 files / 380 tests passed.
- Expanded affected surfaces: 125 files / 1,014 tests passed.
- Graph/filter, analytics disclosures, Tools and inbox batch: 5 files / 29 tests passed.
- Privacy failure/save and Activity filtering: 2 files / 2 tests passed.
- Subscription wording and action wiring: 2 files / 40 tests passed.
- DAG validator: 6 tests passed, covering cycles, missing dependencies, missing evidence and premature completion.
- Complete final UI gate: **564 files / 4,622 tests passed**, exit 0, in 150.80 seconds (`full-ui-release-gate-v2.log`). This includes the final Analytics project-switch/failure and History container-width regressions. Earlier runs surfaced a raw-tooltip convention violation, an outdated billing-label assertion, a spacing-token violation, and a load-sensitive World startup timeout. The code/assertions were corrected, and the final suite ran with three workers; no test timeout was weakened.
- Final TypeScript: **passed**, exit 0 (`typecheck-final.log`).
- Follow-up responsive regression: **3 files / 31 tests passed**, including a wide-window/narrow-container History resize case; Analytics follow-up: **3 files / 20 tests passed**; Privacy follow-up: **1 test passed**. The named Settings container query was checked at both 900px (rail only) and 480px (picker only). A narrow Analytics failed-read fixture shows unknown status and retry, not a false empty-source claim.
- Production UI build: **passed**, guard exit 0; Vite completed in 27.05 seconds after TypeScript. Output is external at `/Volumes/PermagentBuilds/permagent/targets/world-production-20261006/ux-ui-dist`. Log/guard receipt: `/Volumes/PermagentBuilds/permagent/logs/20261015-234316-89580.log` and matching `.json`. Vite reported existing large-bundle warnings; no deployment or installation followed.
- Daemon attempt 1: guarded `cargo check -p permagent-daemon --lib --no-default-features --features rustls-tls` stopped at the **one-hour build deadline**, child exit 143. External log/guard receipt: `/Volumes/PermagentBuilds/permagent/logs/20261015-224305-99647.log` and matching `.json`. No compiler error was reported before termination, but compilation did not finish. Minimum free space was 42.10 GiB external and 9.64 GiB internal, above the enforced floors. No daemon test result is inferred from dependency progress.
- Daemon attempt 2 completed with two compiler errors: the stripped feature set omitted the `posthog` module required by an existing telemetry route, and the current SHA-256 output type did not support the new Watcher `LowerHex` formatting. The check now includes the existing `telemetry` feature; Watcher uses `hex::encode`.
- Corrected daemon library check: **passed**, exit 0, in 2m 44s with `--no-default-features --features rustls-tls,telemetry`. Log/guard receipt: `/Volumes/PermagentBuilds/permagent/logs/20261015-235705-93655.log` and matching `.json`. One existing unused-import warning remains in `routes/devices.rs`. This is a library compile check, not an executed test result.
- Executable Rust regressions and isolated staging HTTP: **blocked by the disk guard, not passed**. The default-feature test build ran for about 53 minutes, then stopped with child exit 143 when internal free space reached 7.91 GiB, below the enforced 8 GiB floor. Log/guard receipt: `/Volumes/PermagentBuilds/permagent/logs/20261016-000441-1017.log` and matching `.json`. No compiler error appeared before termination; the test executable was not produced and no tests ran. External free space stayed above 40 GiB. Internal space briefly recovered to about 11 GiB, then fell below 8 GiB again, so the 12 GiB startup minimum prevents a safe retry. Resume with the same guarded command after space is available; do not weaken thresholds or stop unrelated work. The real HTTP/browser gate has not run.
- Runner configuration: The macOS runner below obtains the exact executable from Cargo JSON, resolves only explicitly linked dylibs within the external debug profile, signs those generated libraries and runs the test binary. It reuses the recovered successful daemon-test configuration: default features, unchanged dependency profiles, debugger tables disabled only for `permagent` and `permagent-daemon`, two test threads. Test assertions and all outer guards remain enabled. An initial cold test-profile attempt was stopped through its owned guard after this reusable configuration was identified.

## Reproducible commands

From `ui/command-center`:

```sh
npx vitest run --maxWorkers=3 --minWorkers=1
npx tsc --noEmit
```

From the canonical repository, when the external build lane is free:

```sh
/Volumes/PermagentBuilds/permagent/run-build npm --prefix ui/command-center run build -- --outDir /Volumes/PermagentBuilds/permagent/targets/world-production-20261006/ux-ui-dist
/Volumes/PermagentBuilds/permagent/run-build cargo check -p permagent-daemon --lib --no-default-features --features rustls-tls,telemetry
/Volumes/PermagentBuilds/permagent/run-build python3 reports/ux-dag/run_rust_tests.py
/Volumes/PermagentBuilds/permagent/run-build python3 reports/ux-dag/run_rust_tests.py --filter watcher_insights
/Volumes/PermagentBuilds/permagent/run-build python3 reports/ux-dag/run_rust_tests.py --filter detected_stack_route
/Volumes/PermagentBuilds/permagent/run-build python3 reports/ux-dag/run_rust_tests.py --http-staging
python3 -m unittest discover -s reports/ux-dag -p 'test_*.py'
python3 reports/ux-dag/check_dag.py --complete
```

`--complete` must fail while any DAG gate is open. The dedicated HTTP test uses the complete production router, bearer middleware, isolated AppState/database, real TCP on `127.0.0.1:3002`, a synthetic manifest and real detector. It refuses a busy staging port, never falls back to the live daemon, and shuts down its owned server. It is explicitly ignored in ordinary test runs so it cannot unexpectedly contend with staging. An ignored test is not a passing acceptance check.

That dedicated staging test also contains real HTTP project-status PATCH/readback, task creation/list readback, person creation/directory lookup, and the Automate/History list-envelope assertions. These are direct HTTP checks; browser navigation and notification state remain covered separately by UI regressions. Creating a synthetic record does not run an automation, send a message or publish content. Until the test executes successfully, these are authored assertions rather than acceptance results.

Optional real-browser gate: run the guarded runner with `--browser`. It prints a private review directory (not its credential), executes the direct HTTP assertions, then holds :3002 for at most ten minutes. Start `PERMAGENT_UX_BROWSER_DIR=<printed-directory> node scripts/ux-staging-server.mjs` from `ui/command-center` and open `http://127.0.0.1:5201/ui/scripts/ux-staging.html`. This uses production components and real same-origin proxied API calls. The proxy targets only :3002, allows edits only to the two synthetic records, and rejects other mutations. Review Projects and detected stack; set that project's status to Active; open People and save its Notes as `UX browser wiring verified`; review Automate and History using `?page=automate` and `?page=history`. Create the printed directory's `finish` file after the walkthrough. The Rust test then asserts that the status and notes persisted and stack discovery created no manual services. The runner removes the private credential file on exit. This is an authored gate until an execution result is recorded; it is not a native WKWebView installation.

The new in-process route regression additionally tests valid id/slug, missing/wrong authentication, missing project/root, secret exclusion, and byte-for-byte preservation of an existing manually corrected stack record. The earlier isolated detector harness passed five tests; that does not validate the full daemon.

## Rendered review

Local synthetic components use `scripts/ux-review.html`, served by `PERMAGENT_REVIEW_UI_PORT=5200 node scripts/home-review-server.mjs`. Reads are intercepted, writes rejected, WebSocket replaced and there is no daemon proxy. Unknown fixture endpoints return explicit errors; these are not evidence of a live service failure.

Checked in Chrome: desktop People graph and person context; narrow Silver inspector with clear Close; desktop Automate selection and active-run separation; narrow Finance Household/Holdings distinction; narrow Privacy navigation/disclosures and a failed status read; Settings at 900px and 480px; Brain List/Graph keyboard selection and a visibly populated synthetic graph; narrow Grow action expansion and the compact subscription selector with inset chevron; Analytics metric/disclosure hierarchy and truthful empty-source copy; Results project/all-project scope selection; narrow Strategy section selection with a Brand draft retained after switching away and back. Earlier Home/Projects fixtures covered busy/empty/error/narrow layouts, retained knowledge drafts and security review. Automated geometry tests cover large graphs, available width and reduced motion. This is not a claim of fresh native WKWebView acceptance, a full contrast audit or a task-based human usability study.

## Remaining boundaries

Full native installation and live acceptance are separate, unauthorized steps. The original Brain blank state has not been reproduced with a proven cause; renderer failure and deadline recovery are preventive fixes. Automatic funnel inference, inferred account identity, download semantic classification/durable deduplication and wider connection provisioning remain research recommendations, not implemented automation. No guessed identity or account connection is presented as detected fact.
