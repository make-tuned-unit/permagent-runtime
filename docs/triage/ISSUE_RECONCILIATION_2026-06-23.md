# Issue Backlog Reconciliation — 2026-06-23

**Scope:** all 115 open GitHub issues reconciled against `origin/main` (HEAD `faf51c1f1`) and git history.
**Method:** evidence-not-assertion. Every DONE/PARTIAL/SUPERSEDED/DUPLICATE verdict carries a `file:line` or commit SHA / PR#. No citation → STILL VALID or UNCLEAR. Conservative bias: a false-DONE (→ wrongful close of real work) is far costlier than a false-OPEN.
**Constraint:** report only. No issue closed/edited/commented; no source touched.

> **Merge-state is the dominant trap in this repo.** Several "fixes" live on **unmerged** branches / open PRs, not `main`. A SHA existing in `git log --all` does **not** mean it shipped. Every DONE verdict below was checked with `git merge-base --is-ancestor <sha> origin/main`. Commits on `feat/goal-completion-evidence` (`ff0748b63`, `ac441ce53`), `feat/l2-terminal-signals` (`bc973badf`), `world-zone-nav*` (`c33b8dfa0`), and `docs/*` branches are **NOT on main**.

---

## 1. Verdict counts

| Verdict | Count |
|---|---|
| **LIKELY DONE** (on main — closeable now) | 12 |
| **STALE / OBSOLETE** (closeable now) | 1 |
| **SUPERSEDED** (closeable now) | 1 |
| **MOSTLY DONE** (closeable now, straggler → #282) | 1 |
| **DONE-PENDING-MERGE** (close when its open PR lands — do NOT close now) | 4 |
| **PARTIALLY DONE** (keep — real work remains) | 18 |
| **UNCLEAR — needs Jesse** | 6 |
| **STILL VALID** (genuinely outstanding) | 72 |
| **DUPLICATE** (issue↔issue) | 0 |
| **Total** | **115** |

**Headline:** **15 issues (~13%) are cleanly closeable now.** Another **4** are done but gated on an open PR merging. **6** need Jesse's call. The real outstanding backlog is **~72 valid + 18 partial = 90 issues** — the count *has* drifted, but mostly because partial/epic work is tracked as single open issues, not because the tracker is full of phantoms.

---

## 2. Full triage table

| # | Title | Verdict | Evidence (file:line / SHA / PR#) | Action |
|---|---|---|---|---|
| 36 | BATCH_MUTEX concurrency test for Librarian | STILL VALID | `scheduling.rs:121` mutex exists; no test found | keep |
| 45 | replace window.alert() in drop-to-chat with toast | STILL VALID | `ui/command-center/src/App.tsx:183` still `window.alert()` | keep |
| 47 | review/batch-merge dependabot PRs | STILL VALID | **16 dependabot PRs open now** (#435–#450) | keep |
| 50 | port Bedrock ReasoningContent fix from upstream | STILL VALID | cited `71b9d575a` **NOT on main**; no ReasoningContent commit on main | keep |
| 53 | librarian run-now blocks synchronously | STILL VALID | `scheduling.rs:528-561` awaits `run_batch()` synchronously | keep |
| 60 | Goals as first-class primitive | PARTIALLY DONE | `goal_state.rs` GoalState enum; `cards.rs` seed_goal_columns; PR #424 | keep |
| 61 | Worker registry + capability protocol | PARTIALLY DONE | `worker_probe.rs`, `agent_identity.rs` WorkerPersona, `goal_state.rs:168` select_best_worker | keep |
| 62 | Lifecycle handoff protocol | PARTIALLY DONE | `goal_transition.rs` advance_goal_checked; `goal_engine.rs` GoalOutcome; PR #424 | keep |
| 63 | Kanban goals view in Command Center | STILL VALID | columns seeded in DB; no dedicated Kanban UI component | keep |
| 64 | mobile client over Tailscale | STILL VALID | no iOS app code | keep |
| 65 | push notification relay (mobile) | STILL VALID | no relay component | keep |
| 66 | notification routing engine | STILL VALID | no policy/rules engine; escalations go to Decision Inbox only | keep |
| 67 | goal-to-tasks decomposition logic | PARTIALLY DONE | `goal_state.rs` topological_order; `orchestrator.rs` decompose_roadmap; PR #372 | keep |
| 68 | Librarian batch checkpoint / pause-resume | STILL VALID | no checkpoint mechanism in `librarian.rs` | keep |
| 69 | Epic: Projects as workspaces | PARTIALLY DONE | `spectral_schema.rs:574` projects table; `cards.project_id`. UI layer absent (~15%) | keep |
| 72 | Projects details — services/credentials | STILL VALID | no services/credential schema | keep |
| 73 | Projects details — resources/people/activity | STILL VALID | not implemented (CRM #256 shipped but uncoupled) | keep |
| 74 | Projects Brain integration | STILL VALID | no project-scoped Brain filtering | keep |
| 75 | Project-scoped Henry/Librarian | STILL VALID | not implemented | keep |
| 76 | Cross-project flows | STILL VALID | not implemented | keep |
| 77 | Librarian FACTS truncated-ID edge case | STILL VALID | no preprocessing found; 0.07% rate, deferred | keep |
| 86 | agent progression mechanic (XP/levels) | STILL VALID | design only, no code | keep |
| 92 | backfill event_at timestamps | STILL VALID | no `event_at` column; `created_at` used uniformly | keep |
| 100 | auth rule for read-only API endpoints | STILL VALID | `/api/agents`,`/api/version` still public in `routes/mod.rs`; #367 didn't codify rule | keep |
| 105 | Librarian leaves the mezzanine | **STALE / OBSOLETE** | cave killed + floor-follow rebuilt, PR #418 (`5c8639512`, on main) | **close** |
| 122 | cleanup fns don't fire at startup (write-lock) | STILL VALID (**live**) | `state.rs` spawns prune/consolidate at startup; contention unfixed | keep |
| 131 | wire session_id into recall remember_with | PARTIALLY DONE | `brain_ops.rs:128` RememberOpts lacks session_id field — **Spectral-pin-gated** | keep |
| 143 | extract ambient context from reply.rs/session_events.rs | **LIKELY DONE** | `reply.rs:341` + `session_events.rs:524` both call `brain_ops::inject_ambient_context` (`brain_ops.rs:166`) | **close** |
| 145 | auth on /events (WS) + /sessions/{id}/events (SSE) | STILL VALID | both in public block `routes/mod.rs:63-64` | keep |
| 149 | revisit signal score merge strategy | STILL VALID | `consolidation.rs` fixed strategy; no configurable enum | keep |
| 151 | seed Spectral recognition bench w/ session data | STILL VALID | Spectral-driven; no Permagent action needed yet | keep |
| 159 | cross-provider tool_use IDs fail validation | STILL VALID (**live**) | no cross-provider ID transform found | keep |
| 161 | richer Recipe authoring (params/sub_recipes/retry) | STILL VALID | `routes/recipe.rs` CreateRecipeRequest only session_id+author | keep |
| 164 | Automate Installed section refinements | STILL VALID | cosmetic; no truncation/collapse-persist fix | keep |
| 166 | subscription-backed chat — CC + Codex CLI as providers | UNCLEAR | PR #424 added ExternalCliEngine as **goal-dispatch worker**, not chat-inference provider — scope mismatch | needs-Jesse-input |
| 167 | Persona settings 'Failed to load' + save not persisting | UNCLEAR | `useSettings.ts` load/save present; no fix commit; repro unconfirmed | needs-Jesse-input |
| 169 | Hermes recency-windowed repetition + SkillProposed visibility | STILL VALID | `tasks/mod.rs:170` repetition query has no recency WHERE clause | keep |
| 170 | standardize build target path | STILL VALID | `daemon.rs:61` find_permagentd_binary divergent; no --target-dir unification | keep |
| 181 | dashboard card types (weather/calendar/stats) | STILL VALID | `dashboard/cards/registry.ts` only 5 types | keep |
| 182 | dashboard card extensibility (skill-pack registration) | STILL VALID | registry hardcoded; no manifest | keep |
| 185 | Silver theme light variant | **LIKELY DONE** | `styles/tokens.ts:169-184` SILVER_COLORS + ThemeId 'silver' + switcher | **close** |
| 187 | migrate ontology.toml entities to Brain storage | STILL VALID | `ontology.toml` still 356 `[[entity]]`; no entities table — **Spectral-pin-gated** | keep |
| 191 | Epic: Social media scheduling (permagent-social) | STILL VALID | no `permagent-social` crate in workspace | keep |
| 193 | neon accent drift #00D5FF vs #00D9FF | STILL VALID (**decided**) | `tokens.ts`/`Terminal.tsx` use #00D5FF; `world/palette.ts`/`constants.ts` use #00D9FF. **RESOLVED (Jesse 2026-06-23):** global accent = `#00D5FF`; Henry needs a DISTINCT trim because `#00D9FF` *collides* with it — see #87. Normalize non-Henry surfaces to #00D5FF; keep Henry's trim deliberately separate. Cross-link #87. | keep (build the fix) |
| 198 | Epic: Voice layer | PARTIALLY DONE | shipped #354/#404/#407/#410/#434/#417; open: #398/#452/#244 L2-3 | keep |
| 210 | execution receipts / heartbeat for goals | STILL VALID | dispatch_evidence on unmerged `feat/goal-completion-evidence`; no receipts on main | keep |
| 211 | store capability snapshot at dispatch | STILL VALID | no capability_snapshot field in card metadata | keep |
| 212 | tag sessions w/ worker_key for tie-break | STILL VALID | worker_key stored (`orchestrator.rs:764`); `select_best_worker` still alphabetical tie-break | keep |
| 213 | extract orchestrator dispatch to free fn | STILL VALID | `dispatch_goal` still a method on Orchestrator | keep |
| 224 | misattributes API-key 401 as 'Gmail auth' | STILL VALID (**live**) | no provider-401-vs-page-auth distinction code | keep |
| 225 | slow session-list query (2-5.6s) | **LIKELY DONE** | PR #374 (`e41e9fb31`, on main) lean SessionSummary projection 711KB→55KB; idx_messages_session exists | **close** |
| 230 | cluster consolidation FK constraint failed | **LIKELY DONE** | PR #260 (`037d385c0`, on main) orphan-cleanup migration + child-table deletes | **close** |
| 242 | storage cleanup recovery total per-run only | STILL VALID | no cumulative metric; cosmetic, deferred | keep |
| 244 | Voice custom pronunciation (Layers 1-3) | PARTIALLY DONE | PR #410 ships **Layer 1 only** (seeded lexicon); L2/L3 open | keep |
| 250 | dedicated Failed-state column | STILL VALID | GoalState has no Failed variant; exhausted goals park in Triage | keep |
| 251 | post-creation roadmap editing | STILL VALID | no insert/remove/reorder goal fns | keep |
| 252 | auto-approve override to skip Review | PARTIALLY DONE | impl cited `ff0748b63` is **on open PR #454, NOT main** | keep |
| 254 | Epic: Voice/Agent Terminal Control (3-tier) | PARTIALLY DONE | Tier 1 = PR #409 (`17d3dcdd5`, merged) `project_launch`; Tiers 2-3 unbuilt | keep |
| 255 | Epic: Unified Agent-Managed Workspace | PARTIALLY DONE | CRM #256 shipped (#363); cross-surface/linking open | keep |
| 256 | CRM as Brain-backed People view | **LIKELY DONE** | PR #363 (`04bb13633`, on main) people table + entity_uuid + GET /api/people (`routes/people.rs:61`) | **close** |
| 257 | cross-surface entity/relationship model | STILL VALID | no entity_project_links / entity_relationships tables | keep |
| 258 | focus-dependent Tab cycling for panes | STILL VALID | no impl; design-needed | keep |
| 259 | Spectral FKs should ON DELETE CASCADE | UNCLEAR | external dep pinned `2c1f6bf`; can't verify Spectral #160/#161 from this repo | needs-Jesse-input |
| 266 | Voice drill-into-project unreliable | UNCLEAR | PR #264 wired project_resolve; original flakiness unconfirmed-fixed | needs-Jesse-input |
| 267 | Henry over-narrates actions | **LIKELY DONE** | PR #404 (`24cbd731c`, on main) anti-narration clause in voice reply prompt | **close** |
| 271 | migrate legacy surfaces to design tokens | MOSTLY DONE | 11/12 file classes clean; `TerminalManager.tsx` retains 3 dark-* refs | **close** (straggler→#282) |
| 276 | pruning.rs deletes w/o child cleanup (orphans) | PARTIALLY DONE | `pruning.rs:61` bare DELETE; interim sweep PR #260 mitigates — **Spectral-pin-gated** | keep |
| 281 | Lane B: sherpa rerun-if-changed; stale sigabrt-bisect | PARTIALLY DONE | PR #375 (`23a19aa17`) dropped sigabrt-bisect.yml; sherpa upstream remains | keep |
| 282 | Lane D: legacy-class stragglers outside #271 | STILL VALID | ~19 hex in `AutomateView.tsx`; 259+ hex patterns across UI | keep |
| 283 | Lane E/C: ghost keys; dangerStrong token; keyring set_var | STILL VALID | `ConfigureProviderModal.tsx:86` sets '' not delete; no dangerStrong; `base.rs:1026` set_var | keep |
| 295 | debug daemon binaries lack rpath for sherpa | UNCLEAR | DYLD_FALLBACK workaround used; no build.rs rpath fix | needs-Jesse-input |
| 306 | Epic: Mesh — Pods + Forum | STILL VALID | M0 spike not started; gates M1-M4 | keep |
| 318 | Git Steward repo-hygiene worker | **LIKELY DONE** | PR #328 (`5b3a84cf2`, on main) steward safety core + ext + recipe + destructive gate | **close** |
| 319 | Overnight Local Review Sweep | STILL VALID | not started; depends Mesh M0.5 | keep |
| 320 | Cross-Model Council (Claude+Codex) | STILL VALID | #424 provides CLI dispatch infra; review-loop logic unbuilt | keep |
| 321 | Mesh prototype spike (70B WAN) | STILL VALID | no spike code | keep |
| 326 | #297-B Tauri auto-updater delivery infra | STILL VALID (ship-blocker) | no tauri-plugin-updater in Cargo.toml; no updater block in tauri.conf | keep |
| 327 | #299-upload crash-report destination | PARTIALLY DONE | PR #332 local capture done (`crash_capture.rs`); upload dest deferred | keep |
| 335 | major dep bumps pending migration (ctor/thiserror/dirs) | **SUPERSEDED** | tracker only; PRs #236/#235/#21 closed; depends on human migration | **close** |
| 339 | Reader re-ingest needs Brain::forget(key) | STILL VALID | no forget() API — **Spectral-pin-gated** | keep |
| 353 | Epic: Agent-led self-knowledge + onboarding | **LIKELY DONE** | PR #361 brief + PR #364 Phase 2 teaching/tour (`tour_tool.rs`) + #419/#420, all on main | **close** |
| 357 | web-search agent-guided setup skill | STILL VALID | skill md embedded but daemon rebuild not shipped | keep |
| 358 | web-search packaging needs npx/Node | **LIKELY DONE** | PR #376 (`0795243d9`, on main) bundles Node + MCP via copy-mcp-runtime.sh | **close** |
| 360 | Epic: Initiative layer | PARTIALLY DONE | PR #380 (`60f2abbe8`, on main) Phase 1; Phase 2/3 open | keep |
| 366 | browser-context /api auth handoff (tunnel) | STILL VALID | #367 removed x-secret-key; tunnel still injects unvalidated header | keep |
| 377 | migrate to @brave/brave-search-mcp-server | STILL VALID | `searchProviders.ts` still pins deprecated server-brave-search@0.6.2 | keep |
| 378 | Phase B in-app dev self-update | PARTIALLY DONE | PR #329 skew detection (#297A); updater plugin + CI artifacts deferred | keep |
| 381 | revive wizard hardware-scan step | STILL VALID | no MomentHardware in WizardShell moments[] | keep |
| 383 | nameplate shows 'Aria' not Henry | **LIKELY DONE** | PR #382 (`aa402a08e`, on main) roster.ts → 'Henry' + useOrchestratorName | **close** |
| 385 | voice preview 'operation not supported' | **LIKELY DONE** | PR #407 (`5a8ff8a1d`, on main) Web Audio decodeAudioData path | **close** |
| 386 | World View zone click→camera nav | DONE-PENDING-MERGE | PR #415/#416 (`c33b8dfa0`) **NOT on main** (dup PRs, world-zone-nav*) | keep until PR merges |
| 387 | entity-description producer (Librarian pass) | STILL VALID | no entity-summary pass; open PR #265 partially related (entity render) | keep |
| 391 | World View arrow-key manual control | **LIKELY DONE** | PR #382 (`aa402a08e`, on main) nudgeAgent in motion.ts | **close** |
| 392 | EPIC: File intake hub | PARTIALLY DONE | #393 inbox shipped PR #422; #394/#395 open | keep |
| 394 | drag-drop file onto chat → Reader | STILL VALID | Reader endpoint exists; Tauri drag-drop interception unwired | keep |
| 395 | route file from inbox to surface | STILL VALID | routing model not designed | keep |
| 398 | Voice interrupt / barge-in | STILL VALID (**live, high-value**) | no TTS cancel mechanism wired | keep |
| 399 | EPIC: Terminal supervision | PARTIALLY DONE | S0 done (#424); spec in open PR #425; S1-S7 not started | keep |
| 400 | Terminal supervision L2 (observe PTY) | DONE-PENDING-MERGE | PR #423 (`bc973badf`) **NOT on main** (feat/l2-terminal-signals) | keep until PR merges |
| 401 | Terminal supervision L3 escalate-only | STILL VALID | no code; depends #400+#425 | keep |
| 402 | Terminal supervision L3 auto-advance | STILL VALID | machinery ready (`decisions.rs:183` tier_for_action_class); needs Jesse policy | keep |
| 427 | S1 launch stream-json CC in visible tab | STILL VALID | no S1 code on main; gated on #425 | keep |
| 428 | S2 gate parser + session registry | STILL VALID | parser exists `claude_code.rs:119`; registry unbuilt | keep |
| 429 | S3 gate → Decision Inbox bridge | STILL VALID | no session_gate kind in decisions table | keep |
| 430 | S4 CC-tool → action_class → tier | STILL VALID | infra ready; blocked on Fork-2 policy | keep |
| 431 | S5 L3 relay write control_response | STILL VALID | `write_to_pty` exists `terminal.rs:217` (no auth guard); gated | keep |
| 432 | S6 project-state memory + librarian_work_queue | STILL VALID (**blocked**) | no project_id on Brain `memories` table; no work queue | keep |
| 433 | S7 "Take Control" | STILL VALID | no code; Tier 1 light, Tier 2 deferred fork | keep |
| 452 | Voice can't capture multi-part instruction | STILL VALID (**live, high-impact**) | no STT endpointing/buffering fix | keep |
| 453 | Kanban consolidate overlapping columns | STILL VALID | `cards.rs:113,153-158` seeds BOTH DEFAULT + GOAL columns | keep |
| 455 | Verifier diffs root_path not worktree | **DONE-PENDING-MERGE / live on main** | bug live on main `verification/mod.rs:119-123`; fix in **open PR #454** (`ff0748b63`) | keep until #454 merges |
| 456 | no per-goal 'done' criterion gated | PARTIALLY DONE | checks machinery on main (`verification/checks.rs`); auto-seeding in open PR #454 | keep |
| 457 | per-project publish sequence (push≠live) | STILL VALID | no publish_sequence schema/logic | keep |
| 458 | Evidence panel reads wrong root_path record | PARTIALLY DONE | dispatch_evidence capture in **open PR #454** (`ac441ce53`); informed-reject unbuilt | keep |
| 460 | Henry browser control (open tab + navigate) | STILL VALID | `navigate_app` is app-internal only; no external URL/tab tool | keep |

---

## 3. RECOMMENDED CLOSURES (15 — strong on-main evidence)

Each verified `git merge-base --is-ancestor <sha> origin/main`. Ready-to-paste close reasons:

| # | Close reason |
|---|---|
| 105 | Obsoleted by World View v2 rebuild (PR #418) — cave killed, free-roam agents now floor-follow; the old vertical-off-path constraint no longer exists. |
| 143 | Done — ambient-context blocks extracted to `brain_ops::inject_ambient_context`; both `reply.rs:341` and `session_events.rs:524` call it. |
| 185 | Done — Silver theme shipped: `styles/tokens.ts:169-184` SILVER_COLORS + `ThemeId 'silver'` + Settings switcher. |
| 225 | Done — root cause was client-side over-fetch, fixed by lean `SessionSummary` projection (PR #374, 711KB→55KB); `idx_messages_session` already present. |
| 230 | Done — FK constraint failures fixed by PR #260: orphan-cleanup migration + explicit child-table deletes in prune/consolidate. |
| 256 | Done — CRM People v1 shipped (PR #363): `people` table + opaque `entity_uuid` + GET /api/people. Linking/enrichment is #257, separate. |
| 267 | Done — anti-narration clause added to voice reply prompt (PR #404). (Behavioral confirmation at next voice dogfood.) |
| 271 | Done (~95%) — design-token migration complete for the listed surfaces; remaining `TerminalManager.tsx` straggler tracked under #282. |
| 318 | Done — Git Steward scheduled hygiene worker shipped (PR #328): steward safety core + platform ext + recipe + native destructive-op gate. |
| 335 | Tracker only — dep-bump PRs (#236/#235/#21) are closed pending human code migration; not actionable as an issue. Reopen when migration is scheduled. |
| 353 | Done — self-knowledge epic Phase 1 (brief/descriptor, PR #361) + Phase 2 (teaching steps/guided tour, PR #364) shipped; Phase 3 not in original scope. |
| 358 | Done — bundled app now ships Node runtime + MCP servers (PR #376); web search works in the packaged app. |
| 383 | Done — nameplate resolves live persona (PR #382): `roster.ts` 'Aria'→'Henry' + `useOrchestratorName`. |
| 385 | Done — voice preview routed through Web Audio `decodeAudioData` (PR #407); WKWebView blob rejection gone. |
| 391 | Done — arrow-key/WASD manual control restored when zoomed (PR #382, `nudgeAgent` in `motion.ts`). |

---

## 4. DONE-PENDING-MERGE — do NOT close yet (gated on an open PR)

These are implemented but **the code is not on `main`**. They auto-resolve when their PR merges; closing now would close work that could still be lost.

| # | Status | Open PR / branch |
|---|---|---|
| 386 | zone click→camera nav complete | PR #415 **and** #416 point to the **identical commit** `c33b8dfa0` — true duplicates. **Recommend: keep #415 (original), close #416** (the `-2` re-push). |
| 400 | L2 process-exit observe signal complete | PR #423 (`feat/l2-terminal-signals`); titled "#400-partial" |
| 455 | worktree-correct verifier fix complete | PR #454 (`feat/goal-completion-evidence`) |
| 458 | dispatch-evidence capture + panel (partial) | PR #454 (same) — informed-reject still unbuilt |

PR #454 also carries partial work for **#252** (auto-approve allowlist) and **#456** (completion-check seeding). Merging #454 should reference `Closes #455` and progress #252/#456/#458.

---

## 5. UNCLEAR — NEEDS JESSE (6)

| # | Question |
|---|---|
| 166 | PR #424 made Claude Code / Codex CLIs **goal-dispatch workers** (`ExternalCliEngine`), not **chat inference providers**. Does that satisfy #166's intent, or is in-chat CLI inference a separate, still-open ask? |
| 167 | Does "Persona settings: Failed to load + save not persisting" still reproduce? No fix commit found; needs a live Settings→Persona→Save→reload test to confirm. |
| 193 | Which neon-cyan is canonical — `#00D5FF` (UI tokens) or `#00D9FF` (World View)? Need the pick before normalizing to one constant. |
| 259 | Does the current Spectral pin `2c1f6bf` (2026-05-27) include Spectral's #160/#161 ON-DELETE-CASCADE migration? Can't verify from this repo (external dep). |
| 266 | Did PR #264 actually fix the voice drill-into-project **flakiness** (not just the feature)? Needs live re-test. |
| 295 | Is `DYLD_FALLBACK_LIBRARY_PATH` the accepted permanent workaround for debug-binary rpath, or should `build.rs` set the rpath? (CLAUDE.md documents the env-var workaround.) |

---

## 6. STILL VALID — the real backlog (72)

Genuinely outstanding, no implementation evidence:

`#36 #45 #47 #50 #53 #63 #64 #65 #66 #68 #72 #73 #74 #75 #76 #77 #86 #92 #100 #122 #131 #145 #149 #151 #159 #161 #164 #169 #170 #181 #182 #187 #191 #210 #211 #212 #213 #224 #242 #250 #251 #257 #258 #282 #283 #306 #319 #320 #321 #326 #339 #357 #366 #377 #381 #387 #394 #395 #398 #401 #402 #427 #428 #429 #430 #431 #432 #433 #452 #453 #457 #460`

Plus **18 PARTIALLY DONE** (open work, keep): `#60 #61 #62 #67 #69 #131* #198 #244 #252 #254 #255 #276 #281 #327 #360 #378 #392 #399 #456` (`*#131` listed once).

**True backlog size ≈ 90 active issues** (72 valid + 18 partial), once 15 closures + 4 merge-gated + 6 unclear are resolved.

---

## 7. LIVE BUGS still real on `main` (surfaced loudly)

A reconciliation that only closes stale issues but misses live bugs is half-useful. These are **confirmed still-broken on main**:

1. **#455 — Verifier diffs `project.root_path`, not the worker's worktree.** `verification/mod.rs:119-123` literally resolves `working_dir` from `project.root_path`. A goal executed in a detached worktree (the #424 dispatch model) is verified against stale local `main` → false-fail / false-pass. **Fix exists in open PR #454 — merge it.** This is the single most important live finding: it breaks orchestrator dogfooding end-to-end.
2. **#452 — Voice multi-part instruction capture.** Long spoken instructions are fragmented by STT endpointing; later clauses dropped. Cripples voice-driven goal dispatch. No fix.
3. **#398 — No voice barge-in.** Henry can't be interrupted mid-response; compounds #267's verbosity. No cancel path wired.
4. **#159 — Cross-provider tool_use ID validation.** Conversation history fails across providers (e.g. Kimi→Anthropic 400). Workaround = fresh session per provider.
5. **#224 — 401 misattribution.** Agent reports a dead provider API key as a "Gmail auth" failure — wrong diagnosis surfaced to user.
6. **#122 — Startup cleanup silently fails.** prune/consolidate spawned at daemon boot hit SQLite write-lock contention and don't run; manual CLI cleanup is the workaround.

---

## 8. Epic status roll-up

| Epic | Status | Detail |
|---|---|---|
| **#353** self-knowledge | ✅ **COMPLETE — closeable** | Phase 1 (#361) + Phase 2 (#364) shipped; Phase 3 out of scope |
| **#392** file intake | ~33% | #393 inbox done (PR #422); #394 drag-to-chat + #395 route-to-surface open |
| **#399** terminal supervision | Design done, **build not started** | S0 done (#424); spec in **open PR #425**; S1-S7 unbuilt; **S6 hard-blocked** (no project_id on Brain memories, no librarian_work_queue) |
| **#254** voice/agent terminal control | ~33% | Tier 1 done (PR #409); Tiers 2-3 await #399 |
| **#198** voice | Phase 1 shipped | open: #398, #452, #244 L2-3 |
| **#360** initiative | Phase 1 done (PR #380) | Phase 2/3 open |
| **#69** projects-as-workspaces | ~15% (schema only) | UI layer (#63,#72-76) entirely open |
| **#255** unified workspace | ~Phase 1 | CRM #256 done; cross-surface (#257) + #191 open |
| **#306** Mesh | Not started | M0 spike gates all; Steward sub-issue #318 done |
| **#191** social scheduling | Not started | no `permagent-social` crate |

**Only #353 is wholesale-closeable as a completed epic.** #399 is the one to watch: **half-designed and stalled behind an unmerged spec PR (#425) + two hard schema blockers for S6.**

---

## 9. Surprises / drift notes

- **The biggest drift source is unmerged work, not phantom work.** Four "done" features (#386, #400, #455, #458) sit in open PRs; an agent that trusts `git log --all` would wrongly close them. Merge state must be checked per-SHA.
- **#386 has two duplicate open PRs** (#415 and #416), same change, branches `world-zone-nav` / `world-zone-nav-2`. Close one.
- **PR #454 is a multi-issue PR** touching #455/#456/#458/#252 + "Discuss-with-Henry" but its body only loosely maps them — exactly the tracker-drift root cause flagged in CLAUDE.md. When merged it should carry explicit `Closes #455` etc.
- **#47 looks closeable but isn't:** 16 fresh dependabot PRs (#435–#450) opened today. The chore recurs; consider an auto-merge policy rather than closing.
- **#50 looked done** (a Bedrock ReasoningContent commit exists in `git log --all`) **but it is not on main** — likely an upstream/other-branch commit. Still valid.
- **#225 closes cleanly but the original hypothesis was wrong:** it was never a missing index (the index existed); the fix was a client-side payload-size reduction (PR #374). Close reason should reflect that.
- **5 issues are Spectral-pin-gated** (#131, #187, #259, #276, #339) — unfixable from this repo without bumping pin `2c1f6bf`. Cluster them so a single pin bump can address several.

---

## 10. Close commands (commented — review, then paste per issue)

```bash
# === RECOMMENDED CLOSURES (15) — verified on origin/main ===
# gh issue close 105 -c "Obsoleted by World View v2 rebuild (PR #418): cave killed, free-roam agents floor-follow; old vertical-off-path constraint gone."
# gh issue close 143 -c "Done: ambient-context extracted to brain_ops::inject_ambient_context; reply.rs:341 + session_events.rs:524 both call it."
# gh issue close 185 -c "Done: Silver theme shipped — styles/tokens.ts:169-184 SILVER_COLORS + ThemeId 'silver' + Settings switcher."
# gh issue close 225 -c "Done: root cause was client-side over-fetch, fixed by lean SessionSummary projection (PR #374); idx_messages_session already existed."
# gh issue close 230 -c "Done: FK constraint failures fixed (PR #260) — orphan-cleanup migration + child-table deletes in prune/consolidate."
# gh issue close 256 -c "Done: CRM People v1 shipped (PR #363) — people table + opaque entity_uuid + GET /api/people. Linking is #257."
# gh issue close 267 -c "Done: anti-narration clause added to voice reply prompt (PR #404)."
# gh issue close 271 -c "Done (~95%): design-token migration complete for listed surfaces; TerminalManager.tsx straggler tracked in #282."
# gh issue close 318 -c "Done: Git Steward hygiene worker shipped (PR #328) — safety core + platform ext + recipe + native destructive-op gate."
# gh issue close 335 -c "Tracker only: dep-bump PRs (#236/#235/#21) closed pending human migration; reopen when scheduled."
# gh issue close 353 -c "Done: self-knowledge epic Phase 1 (PR #361) + Phase 2 teaching/tour (PR #364) shipped; Phase 3 out of scope."
# gh issue close 358 -c "Done: bundled app ships Node + MCP servers (PR #376); web search works in packaged app."
# gh issue close 383 -c "Done: nameplate resolves live persona (PR #382) — roster.ts 'Aria'→'Henry' + useOrchestratorName."
# gh issue close 385 -c "Done: voice preview routed through Web Audio decodeAudioData (PR #407)."
# gh issue close 391 -c "Done: arrow-key/WASD manual control restored when zoomed (PR #382, nudgeAgent in motion.ts)."

# === DO NOT CLOSE until the PR merges (then they auto-close via Closes #) ===
# 386 -> merge PR #415 or #416 (close the duplicate)
# 400 -> merge PR #423
# 455 -> merge PR #454   (LIVE BUG on main until then)
# 458 -> merge PR #454 (partial)
```

---

## 11. Closure-readiness flags (Jesse follow-up, 2026-06-23)

### Code-done vs visually-verified (before closing #353 / #383 / #391)

| # | Type | Verified by | Before closing |
|---|---|---|---|
| **#383** nameplate name | **Render-correctness** (not pure logic) | `tsc + vite build` only (PR #382). Logic is sound: `useOrchestratorName()` reads `getIdentity().first_name`, only the `isHenry` roster entry is overridden. | One visual eyeball: World View 3D nameplate + hover tooltip show the **configured** name (Henry), and `HenryHUD` no longer falls back to 'ARIA'. No automated render test exists. |
| **#391** arrow-key control | **Behavioral / interaction** (not pure logic) | `tsc + vite build` only. `nudgeAgent()` applies delta + drops autonomous path + honors Librarian ring-lock. | One hands-on check: **zoom into an avatar**, press arrow/WASD — the avatar moves and faces travel direction while keys held. The bug was a no-op wiring, only observable third-person; not CI-catchable. |
| **#353** self-knowledge epic | **Code-done** (shipped descriptors + tour) | PRs #361/#364, on main; `self_knowledge/mod.rs` exists. | See rule note below — closing the epic is safe *for the code*, but the cross-cutting enforcement guard is unbuilt. |

**Recommendation:** #383 and #391 are frontend-only and low-risk, but neither is pure logic with test coverage — give each a 30-second GUI check before closing. #353 is code-complete.

### Does closing #353 lose the "ship the descriptor in the same PR" standing rule?

**No — the rule survives, but its enforcement is only partial.**
- **Persistent home:** documented at `docs/architecture/SELF_KNOWLEDGE_BRAIN.md:153` — *"Updates to Permagent features must include corresponding updates to the self-knowledge corpus. This is a process discipline, not a technical guarantee. Recommendation: a CI check that fails when feature code changes without corresponding self-knowledge updates."* Closing the epic does not delete this.
- **Partial compile-enforcement (real today):** platform **tools** extend `PlatformExtensionDef`, which is compile-enforced (a missing descriptor field is an `E0063` build error). That guard is genuine and stays.
- **Gap (the part that's only process discipline):** **worker** and **surface** descriptors live in hand-maintained static slices — `WORKER_DESCRIPTORS` / `SURFACE_DESCRIPTORS` (`self_knowledge/mod.rs:131,137`). A new worker/surface *can* ship without a descriptor and still compile.
- **The recommended CI check is NOT built:** `.github/workflows/ci.yml` has zero self-knowledge references.

**So:** safe to close #353 (the build is done). But the *enforcement* of the standing rule is itself unfiled work — recommend a **new issue: "CI guard: fail when capability code changes without a self-knowledge descriptor update"** so the rule doesn't decay once the epic is closed. (Not filed here — report-only.)

### §7 live-bug triage table (for prioritization)

| # | Bug | Severity | Fix path known? |
|---|---|---|---|
| **#455** | Verifier diffs `project.root_path`, not the worker's worktree → orchestrator verification false-pass/fail | **Critical** — breaks #424 dogfooding end-to-end | **Yes — fix already written in open PR #454.** Just merge (after review). |
| **#452** | Voice multi-part instruction fragmented by STT endpointing; clauses dropped | **High** — cripples voice goal-dispatch | Partial — hypothesis is STT endpoint window; needs Phase-0 spike on sherpa endpointing + buffer-before-dispatch. No code yet. |
| **#398** | No voice barge-in / interrupt mid-response | **High** — blocks natural voice UX; compounds #267 | Known approach — add cancellable TTS path + space/click→halt→return-to-listening. No code yet. |
| **#159** | Cross-provider conversation history: tool_use IDs fail validation across providers | **Medium** — fresh-session-per-provider workaround exists | Known approach — strip/transform provider-specific tool_use IDs on cross-provider replay. Not started. |
| **#224** | Agent misattributes provider 401 (dead API key) as "Gmail auth" failure | **Medium** — wrong diagnosis to user; not data-loss | Known approach — distinguish provider-auth 401 from target-page auth in error attribution. Not started. |
| **#122** | Startup cleanup (prune/consolidate) silently fails on SQLite write-lock contention | **Low/Medium** — manual CLI cleanup workaround; #121 ingest filter mitigates | Known approach — defer cleanup until after Brain init / share connection. Not started. |

**Parked per Jesse (resolve later with repro/recall, not chased now):** #166, #167, #259, #266, #295.
