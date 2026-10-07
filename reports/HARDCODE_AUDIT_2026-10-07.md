# Hardcode audit: developer-specific behaviour in shipped Permagent

Worktree: /Users/j/Documents/dev/permagent-worktrees/world-production-20261006 (read-only, 2026-10-07).
Scope: crates/*/src, crates/goose/assets, ui/command-center/src, ui/desktop. Test code is listed in the appendix.

## Existing guards (not duplicated)
- crates/goose/src/config/identity_name_guard.rs: "Henry" in code (not comments, not tests) and "Jesse" anywhere, over *.rs/*.ts/*.tsx under crates/*/src and ui/command-center/src. It is case-sensitive and whole-word.
- scripts/policies/shipped-content.json, run by scripts/shipped-content-check.mjs. It has 3 rules and 1 exception, and is limited to prompt/automation/recipe/skill/tutorial/app_catalog globs:
  - `operational-home-root` bans /Users/, /home/, ~/Documents/dev, ~/dev and $HOME/dev.
  - `projects-tool-home-guess` bans ~/Documents/dev and ~/dev in projects.rs.
  - `unsupported-machine-topology` bans "your Mac mini" and "your tailnet".
- Neither guard scans .toml, .json, .md, .html, or tauri.conf.json. Neither scans "Hank", surnames, project names, places, wallet addresses, ports or model ids. Everything below falls outside both.

Severity: blocker = breaks or misleads another user's instance; major = visible developer-specific default; minor = internal.

---

## A. Identity and persona leaks

A1 [BLOCKER] The developer's agent's on-chain/Chitin identity is the identity source for every install.
- ui/command-center/src/services/identity.ts:114 and :125 fetch `https://chitin.id/henry-malcolm.json` and `https://chitin.id/henry-malcolm`.
- identity.ts:274-288 `DEFAULTS` holds `did:chitin:henry-malcolm`, name `'Hank'`, an arweave avatar URL, `sbtId: 54`, `passportId: 38105`, owner wallet `0x95Ab1B24f8c0C70E59687f742C79F97a9277996f`, `arweaveTxId` and `bornAt: 2026-03-30`.
- ui/command-center/src/config/chain.ts:5-9 hard-codes `SBT_CONTRACT`, `PASSPORT_CONTRACT`, `AGENT_OWNER`, `SBT_TOKEN_ID=54n` and `PASSPORT_TOKEN_ID=38105n`. `hydrate()` reads them on every instance and overlays them on the merge base.
- Consumers: components/world/HenryIdentityTab.tsx, world/forum/ForumPassport.tsx (shows the DID slug as "Chitin ID"), and world/forum/ForumView.tsx:255 `passportName = useIdentityStore(s => s.data?.name)`.
- Effect: another user's World passport and identity tab show the developer's agent's name, DID, wallet and soul status. The pointers are Hank's, not the user's.
- Fix: add config keys `identity_provider` (default `none`) and `chitin_slug`, `sbt_contract`, `sbt_token_id` and `passport_token_id`, all empty by default.
  - With no slug set, the passport shows "No on-chain identity configured" and makes no network call.
  - DEFAULTS become all-empty/null.
  - Remove chitin.id, mainnet.base.org and arweave.net from the Tauri CSP unless the key is set.
  - Move chain.ts constants to config.

A2 [BLOCKER] The World falls back to the literal "Hank" (legacy name, not caught by the guard because the guard only checks "Henry").
- ui/command-center/src/components/world/forum/ForumView.tsx:256 `configuredName = useOrchestratorName() ?? namedByUser ?? passportName`. `passportName` comes from A1.
- ForumView.tsx:527 `<HankConversation name={configuredName || 'Hank'} />`.
- ForumView.tsx:568 `Sit with {configuredName || 'Hank'}`.
- world/forum/HabitatAgentDock.tsx:50 `name === 'Your orchestrator' ? 'Hank' : name`.
- Fix: use the persona API (`useOrchestratorName`, falling back to `PrimaryPersona` display name from /api/agent/identity) and otherwise the neutral "Your orchestrator" (already used at ForumView.tsx:522 and HeroGardenKit.tsx:153).
  - Drop `passportName` from the chain.
  - Rename HankConversation.tsx and its CSS classes `hank-*` (cosmetic, minor).
  - Add "Hank" to a guard (see section L).

A3 [minor] The primary-persona default "Aria" is duplicated instead of read from `PrimaryPersona::default()`.
- crates/goose-server/src/proactive.rs:178.
- crates/goose-server/src/agent_state_tick.rs:39.
- ui/command-center/src/components/wizard/WizardShell.tsx:61 sets `name: 'Aria'` when the user skips naming.
- Documented default (crates/goose/src/config/agent_identity.rs:267), so acceptable. The server pair should call `PrimaryPersona::default().first_name`, so there is one source.

A4 [minor] ui/command-center/src/CyborgTest.tsx:14-18 ships a dev view (main.tsx:9, `?view=cyborg-test`) with literal labels `Aria`, `Felix`, `Nova` and `AGENT_TRIM.henry/felix/nova/aria`. These are decorative sim agents removed from the roster (world/agents/roster.ts:7). Fix: gate the view behind `import.meta.env.DEV` or delete it.

A5 [clean] Pronouns: no "he/his/him" in user-visible UI strings or prompts. The only occurrences are comments (Sidebar.tsx:404, WorldAgents.tsx:42, agent_identity.rs:1000). World roster (world/agents/roster.ts) uses `name: 'Agent'` for the orchestrator and gets the real name from /api/agent/identity (stateSources.tsx:82-87). Forum agentProfiles.ts uses only product roles.

---

## B. Personal data shipped in the repo

B1 [MAJOR, privacy] crates/goose/assets/ontology.toml (2297 lines, 388 entities) is the developer's real knowledge graph, git-tracked.
- Person entities include `henry malcolm` (:5), `jesse sharratt` (:113), `sophie sharratt` (:1109), `mel schembri` (:317), `dennis adams` (:1139), `craig beaton` (:1397), `graham wong` (:1415), `ashley lecroy` (:1433), `claudia chender` (:1463), `susan leblanc` (:1469), `penny arsenault` (:1733), `robin hood` (:329), `rowan` (:1235), `jude` (:1241).
- Other entities: `halifax` (:419), `halifax regional municipality` (:1187), `nova scotia` (:89), `nova scotia grant applications` (:1133), `nova scotia pilot` (:1571), `african nova scotian` (:1169), `getladle` (:209), `plekk` (:1061), `lauft` (:1097), `atlas atlantic` (:311, aliases :312), `atlas atlantic supabase` (:365), `atlas atlantic website updates` (:1751), `atlas atlantic rebrand summary` (:1823), `ns department of community services` (:1127), `canada council` (:1115), `marsh canada` (:1373), `henry-sells`, `zeroclaw`.
- Runtime does not load this file. It uses `Paths::brain_ontology()` (config/paths.rs:67). It is `include_str!`'d only by integration tests (crates/goose-server/tests/*.rs, crates/permagent-brain-tests/tests/common.rs).
- Result: personal names, family and clients are published, though not seeded into other users' brains.
- Fix: move to crates/goose/tests/fixtures/ontology.sample.toml with synthetic entities and repoint the 7 test includes. Keep the real one out of git.

B2 [major] crates/goose/src/prompts/session_name.md:5 uses the example "help me set up analytics for kinrows" in the shipped prompt. Fix: change to a neutral name (`acme-web` is already used in the next example).

B3 [minor] crates/goose-server/src/voice/ort_kokoro_backend.rs:202 has a TTS pronunciation entry `("kinrows", "kˈɪnrəʊz")`. Fix: remove it, or load pronunciations from a user lexicon file.

B4 [minor] ui/command-center/src/lib/qrDecode.verify.md:21 holds a real tailnet host `jesses-mac-mini-2.tail4429f1.ts.net`. Fix: replace with `example.tailnet.ts.net`.

B5 [major] ui/desktop/src-tauri/tauri.conf.json:63 sets `signingIdentity: "Developer ID Application: Jesse Sharratt (Z58XSBM78S)"`. ui/desktop/src-tauri/Cargo.toml:6 sets `authors = ["Jesse Sharratt"]`. A fork cannot build a signed bundle without editing the file. Fix: read the identity from env `APPLE_SIGNING_IDENTITY` (Tauri supports it) and remove it from the committed config. Authors can become "Permagent contributors".

B6 [major] tauri.conf.json:82-86 updater `endpoints` is `https://github.com/make-tuned-unit/permagent-runtime/releases/latest/download/latest.json` with a fixed `pubkey`. That is correct for the official build, but a self-built fork will auto-update to the upstream binary. Fix: leave it for the official build and document the fork override. Consider `updater.enabled` as config.

B7 [major] Cargo.toml:88 pins `spectral` to `https://github.com/make-tuned-unit/spectral` rev 7025328e. Cargo.toml:119-124 documents a local path patch. If that repo is private, no outside user can build. Fix: publish it or vendor it, and say so in the README.

---

## C. Paths, hosts, ports

C1 [major] crates/goose/src/picker.rs:121 hard-codes `root.join("Picker/pre_surge_scanner")`. The service is the developer's private Flask app (`csv_web_server.py`, picker.rs:1134). The `picker_root` key overrides it, and the dev-root list is user-configured, which is good. :89 `DEFAULT_BASE = http://127.0.0.1:8080` (`picker_url` overrides). The user-visible detail at :202 and :212 narrates developer history ("The Librarian's nightly llama-server split moved off this port to :8081 on 2026-08-25 ... old qwen38-split.sh").
- Fix: the Picker is an optional integration. Default `picker_url` to unset, so there is no probe and no card unless configured. Replace the checkout-layout guess with a required `picker_root`. Strip dated dev narrative from the user-facing strings.

C2 [major] crates/goose/src/cost_router/fallback.rs:473, :912 and :983-989 treat the provider `qwen38_split` (`http://127.0.0.1:8081`) as a rung in the fallback ladder. It is declared in providers/declarative/qwen38_split.json ("Local two-machine llama-server split ... Start it with ~/.local/llama/qwen38-split..."). It is the developer's two-Mac-mini setup (memory: paused). Also providers/declarative/llama_swap.json:14 defaults to :8080.
- Fix: keep the generic `llama_swap`/`openai_compatible`; remove the split provider and rung, or require `QWEN38_SPLIT_HOST` explicitly (no default).

C3 [minor] crates/permagent-eval/src/swebench.rs:461 `ssh_target: "m1"` default, :53 `DEFAULT_DOCKER_HOST ~/.colima/...`. This is the dev eval crate. Fix: `ssh_target: Option<String>`, error if unset.

C4 [minor] Ollama `http://localhost:11434` is a hard-coded constant with an `OLLAMA_HOST` override in some places and not others.
- No override: goose-server/src/verification/verifier.rs:16, goose-server/src/routes/ollama.rs:13, goose-cli/src/commands/doctor/checks.rs:8, goose-cli/src/commands/setup.rs:81 and :140, goose/src/config/worker_probe.rs:269, goose/src/mesh/mod.rs:241, permagent-harvester config :80/:32.
- Overridden: fallback.rs:938 reads `OLLAMA_HOST`.
- Fix: route all through `providers::ollama` / `recommend::ollama_base_url(OLLAMA_HOST)`. A default is fine, a non-overridable constant is not.

C5 [minor] Comments and docs that name `~/Documents/dev/*`, `/Users/j/...`, `~/dev/Picker` etc. (non-code, safe, listed for sweep): brain_ops.rs:463, polybot.rs:62, picker.rs:3/99/102, projects.rs:100, storage_health/scanner.rs:156/159, config/dev_roots.rs:7-32/103, permagent-eval/src/runner.rs:165, librarian.rs:2122 (`/Users/j/.local/llama/logs`), finance.rs:13, terminalReattach.ts:104 (`GetLadle`), TerminalManager.tsx:480, skillHumanize.ts:132, GrowAnalytics.tsx:296-297 (Evntally/GetLadle), automation/workspace_snapshot.yaml:35 (`~/dev/canon`), crates/permagent-harvester/harvester.example.toml:18 (`~/Documents/dev/permagent-runtime`). `.claude/commands/dispatch.md:34` uses `~/dev/permagent-runtime`.

C6 [minor] ui/command-center/src/components/dashboard/decisions/mockDecisions.ts:145,227,239 contain `/Users/jesse/dev/...`. They are bundled (client.ts:184 imports it) but only used when `VITE_DECISIONS_MOCK=1`. Fix: lazy import, or sanitise the paths.

---

## D. Region and locale assumptions

D1 [major] crates/goose/src/finance_statements.rs:216-265 `categorize()` has Nova Scotia / Maritime merchant rules: `nslc`, `nspower`, `ns power`, `eastlink`, `sobeys`, `superstore`, `no frills`, `tim hortons`, `shoppers`, `skip`, `e-transfer`, `rogers`, `bell`.
- Result: statement imports from other countries are mostly "uncategorized", and "shell"/"skip"/"bell" over-match.
- Fix: ship a locale-neutral default (rent, grocery, uber, coffee, restaurant, pharmacy ...) and load regional rules from `~/.permagent/finance_categories.toml`. Also fix the substring over-match.

D2 [minor] ui/command-center/src/components/finance/money.ts:62-63 `DISPLAY_CURRENCIES` is only USD and CAD. Fix: derive from `Intl.supportedValuesOf('currency')` plus the daemon's rates.

D3 [clean] Weather: the card is blank until the user sets a location (dashboard_cards.rs:1005-1010, the geocoded location is stored per user). There are no Halifax or Atlantic time defaults. Cron uses the local zone.

D4 [minor] finance.rs:256 says "15:30 America/New_York". That is the US market close, correct as a product fact (the Picker). Treat it as a fixed US-equities assumption and document it.

---

## E. Vendor and model lock

E1 [MAJOR] The measured defaults need keys a new user has not configured.
- crates/goose/src/config/model_roles.rs:311 and :322 chat is `custom_deepseek` / `deepseek-chat`.
- model_roles.rs:326 and :336 harness is `openai` / `gpt-5.4-mini`.
- config/voice_model.rs:52 and :54 voice is `custom_deepseek` / `deepseek-chat`.
- ui/command-center/src/components/settings/SettingsView.tsx:91-92 duplicates the voice default.
- Applies when no model is configured, so a user with only an Anthropic or Ollama key gets a role routed to a provider they never set up.
- `assess_harness_coordinator` (model_roles.rs:135) grants `TrustedDefault` only to openai/gpt-5.4-mini. Every other model needs graduation evidence.
- Fix: resolve the default from "configured providers that have credentials", ordered by a cost table, and use these measured picks only as the preference among configured ones. Make the TrustedDefault id a config list.

E2 [major] Local-model names hard-wired with no override: reader/mod.rs:90 `SUMMARY_MODEL = "qwen2.5:7b"` (used at :821) and goose-server/src/concierge.rs:41 `DRAFT_MODEL` (used at :235).
- Overridable: playbook/synthesis.rs:48, initiative/driver.rs:67, verification/verifier.rs:18, and routes/librarian/scheduling.rs:37 (the schedule JSON).
- Hard-wired/other: agent_identity.rs:988 `availability_check: "model_loaded:qwen2.5"`, platform_extensions/app_perception.rs:382 and :401, permagent-harvester config.rs:81-82.
- Result: the Reader summary and Concierge draft fail silently on a machine without qwen2.5:7b. The Librarian shows "unavailable" unless that exact model is pulled.
- Fix: one config key `local_default_model` (or "first installed model from the capability probe"); replace all 9 constants with a lookup; availability check becomes "any local model".

E3 [minor] strix.rs:670 `DEFAULT_STRIX_LLM = "openai/gpt-5.4-mini"` is configurable via `strix_llm`. `strix_llm_secret_name` (:683-693) is a closed vendor list (anthropic, openai, groq, gemini, deepseek, moonshot). Others are left unset. Fix: resolve the key from the provider registry.

E4 [minor] ui/command-center/src/lib/modelTiers.ts:18-21 recommends the qwen3 line by RAM tier. Acceptable as a recommendation. SettingsView.tsx:1279 hard-codes the hint 'ollama pull qwen2.5:3b' and :1275 "Ollama at localhost:11434".

E5 [minor] crates/goose/src/posthog.rs:16-17 embeds a PostHog project key (`phc_RyX5...`) and `us.i.posthog.com`, the upstream goose/Block project. It is opt-in (`is_telemetry_enabled` defaults false), but an opted-in user's events go to a third party's project. Fix: read key and host from config/env and default to no sink.

E6 [clean] tauri.conf.json CSP (connect-src, :53) is localhost:3001 plus fonts.googleapis/gstatic, chitin.id, mainnet.base.org and arweave.net. The last three come from A1. `img-src` and `frame-src` allow any http(s). No personal hosts. `ui/command-center/src-tauri/tauri.conf.json`, referenced by config/chain.ts:11-12, does not exist (stale comment).

---

## F. Onboarding, seeds, scheduled jobs, dashboard

F1 [clean] onboarding_seed.rs (178 lines): all three seed memories and the how-to are generic ("Welcome to Permagent ...").
F2 [clean] automation/*.yaml starters (model_review, steward, storage_insights, health_review, workspace_snapshot) contain no developer projects or hosts. The only item is the `~/dev/canon` example in workspace_snapshot.yaml:35, a placeholder (suppressed by the one policy exception).
F3 [clean] Dashboard templates (routes/dashboard.rs:555-600) use generic card types.
F4 [major] default_enabled worker extensions: platform_extensions/mod.rs:149-398 list many `default_enabled: true`. The Finance extension description (mod.rs:766) mentions the Picker and Polybot with "the user's Polybot". A fresh install shows a Finance/Picker/Polybot surface for services a new user does not have. Fix: gate on "integration configured" (picker_url/picker_root, polybot config), not on the extension flag.

---

## G. Agent roster (product roles: fine)
Librarian, Picker, Strix, Forecaster, Steward, Watcher, Polybot, Reader, Financier and Council are fixed product roles in world/agents/roster.ts, forum/agentProfiles.ts and the World HUDs. No developer names, project names or user names are rendered in desk labels or nameplates. All nameplates come from the roster or `/api/agent/identity`, except A2.

---

## Appendix: test and fixture hits (allowed, listed separately)
- Paths: crates/goose/src/session/crash_capture.rs:515,546,557 (`/Users/jesse`); storage_health/safety.rs:117-154; storage_health/classify.rs:450-610 (`/Users/j/...`, `GetLadle`, `spectral`); storage_health/inuse.rs:549; security/scanner.rs:647-694; goose-server/src/strix.rs:1024,1027,2090-2163 (`jessesharratt@m1` and `/Users/jessesharratt`; `remote_home` is `#[cfg(test)]`); goose-cli/src/session/composer.rs:2979 (`atlasatlantic`), :3257; verification_approval/tests.rs:17; config/base.rs:3313,3328 (`me@m1`); mesh/pool.rs:1347 (`mini.local`).
- Ports and models: crates/goose/src/decision/tests.rs:210-258 (:8000, :8080); mesh/pool.rs:1320-1695 (qwen2.5:7b, llama3:8b); orchestrator.rs:40762-41183.
- Eval: crates/permagent-eval/tasks/, README.md and evidence/*.md (dev history).
- *.test.* hits for "Hank": NavAgentStatus.test.tsx (8), ForumView.conversation.test.tsx (5), HankConversation.test.tsx (4), agentFocus.test.tsx (2), passportHeader.test.tsx, extremeRuns.test.ts.
- crates/goose-cli/src/scenario_tests/recordings/*.json embed upstream goose/Block prompts (unrelated to developer data).

---

## Ranked lane plan (12 lanes)
1. L1 [A1+A2+E6] Identity-source lane: make chitin/chain opt-in config (`chitin_slug`, contracts), empty DEFAULTS, no network when unset, drop chitin.id/base/arweave from CSP unless enabled. Remove the 'Hank' literals (ForumView:527,568, HabitatAgentDock:50), use the persona API with neutral "Your orchestrator", rename HankConversation.
2. L2 [B1] Remove assets/ontology.toml from git, replace with a synthetic fixture, repoint 7 test includes.
3. L3 [E1] Provider-aware defaults: choose chat, voice and harness defaults from configured providers; make the TrustedDefault set config.
4. L4 [E2] `local_default_model` config key; replace the 9 `qwen2.5:7b` constants and `model_loaded:qwen2.5` check.
5. L5 [D1] Locale-neutral finance categories plus a user rules file; fix substring over-match.
6. L6 [C1+F4] Gate Picker/Polybot/Finance-scanner surfaces on configured integration; neutral error strings; drop the dated dev narrative (picker.rs:202,212).
7. L7 [C2] Remove qwen38_split provider/fallback rung, or make it explicit-config only.
8. L8 [B5+B6+B7] Release config: signing identity from env, authors field, updater override docs, spectral dependency made buildable (publish or vendor).
9. L9 [E5] PostHog key/host from config; no embedded upstream key.
10. L10 [C4] One Ollama base resolver honoring `OLLAMA_HOST` everywhere.
11. L11 [B2+B3+B4+C5+A4+C6] Cosmetic sweep: session_name.md example, kinrows TTS entry, tailnet host in qrDecode.verify.md, CyborgTest dev-gate, mockDecisions paths, comment names.
12. L12 [guard] Extend the guard (section L) and wire it to CI. Do this alongside L1, since it pins the fixes.

## L. How to add the repo-wide guard
Extend the existing policy-driven guard instead of writing a third scanner, because it already has exceptions with reasons and `reviewBy` dates.
1. Add rules to scripts/policies/shipped-content.json and widen `scope.roots` to `crates/goose/assets`, `crates/*/src`, `ui/command-center/src`, `ui/desktop/src-tauri` and `ui/command-center/public`. Add `.toml`, `.json` and `.html` to `extensions`, excluding `providers/canonical/data` and the whisper tokens via `skipDirectories` or `include` negation.
2. Rules (`operationalLiteral`, with `testFixture` exceptions only):
   - `legacy-agent-alias`: `\bHank\b`, `HankConversation`, `henry-malcolm`.
   - `developer-wallet-or-did`: `0x[0-9a-fA-F]{40}` and `did:chitin:` outside config/chain.ts defaults.
   - `developer-places-projects`: `(?i)\b(Halifax|Nova Scotia|Kinrows|Evntally|GetLadle|Atlas ?Atlantic|CityCircle|Pibye|Plekk|Lauft|Sharratt|Schembri)\b`.
   - `personal-hosts`: `jessesharratt`, `\.ts\.net`, `"m1"`, `mini\.local`, `Z58XSBM78S`.
   - `hardcoded-local-model`: `qwen2\.5:|qwen3\.8` outside model_roles.rs, `cost_router/knowledge.rs` and the allow-listed default-resolver module.
   - `third-party-telemetry-key`: `phc_[A-Za-z0-9]{20,}`.
   - `regional-merchant-rules`: `nslc|nspower|eastlink` in finance*.rs.
3. Keep the persona literal check in identity_name_guard.rs, extending `LEGACY_AGENT_NAME` to a list (`Henry`, `Hank`). Treat `Aria` as allowed only in `agent_identity.rs` and the wizard.
4. Run it in CI through the existing shipped-content check, and add a Rust test that shells out to `node scripts/shipped-content-check.mjs`, as other guards do, so `cargo test` fails locally too.
5. Record every exemption as an exception with `reasonCode` and `reviewBy`. The fixtures in the appendix are the first `testFixture` exceptions.
