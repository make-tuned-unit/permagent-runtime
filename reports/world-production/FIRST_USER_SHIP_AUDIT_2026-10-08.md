# First-user shipping audit — October 8, 2026

Base: implementation checkpoint `7f4046c3`. Scope: native bootstrap, packaging inputs, config paths, setup wizard, resource discovery. No UI control, API calls, install, runtime launch, model calls, or native build performed. Record and World implementations remain owned by other workers. No applicable ancestor or subtree AGENTS instructions were found for these frontend/native files; the read-only backend config audit read `crates/goose/AGENTS.md` (context only).

## Ship-blocker matrix

| Gate | Evidence and current result | Next acceptance |
| --- | --- | --- |
| Frontend/native backend target agreement | **Fixed in source.** `src/lib/api.ts` accepts both VITE daemon URL aliases, whereas native production auth/supervision uses 3001. A stale review URL could previously be embedded by an ordinary native build. New `nativeBuildInputs.ts` validates both aliases after Vite loads its mode-specific env. Production permits only unset/empty or canonical local 3001; explicit review builds must match their port. Errors omit URL values. | Rebuild the eventual shipping artifact through the normal Vite config. `build.rs` still handles debug-only review root restrictions. Skipping frontend rebuild with a previously contaminated dist remains an artifact-review gate. |
| Fresh home, no credentials/provider/model | **Source reviewed; genuine fresh runtime not yet tested.** `Paths::base_dir` uses current-user `.permagent`, or an explicit test override. `Config::default` uses that config and an optional executable/workspace `defaults.yaml`; no tracked defaults.yaml found. GOOSE_PROVIDER/MODEL have no personal values assigned here. Existing 3002 runtime deliberately seeds Ollama and is not evidence for this gate. | Use distinct 3003 plan below. Confirm config starts without provider/model/wizard flag, setup opens, Decide later completes, chat accurately explains missing model. |
| Daemon on a machine without a separate service | `ui/desktop/src-tauri/src/daemon.rs: start_daemon/spawn_sidecar` uses bundled daemon, loopback3001, current user's data paths, and bounded startup reporting; it preserves an existing launchd owner. | Install signed release on a fresh macOS user without launchd service and prove sidecar starts and stops with the app. Review builds deliberately do not spawn their own daemon. |
| Bundled dictation model recovery | **Fixed in source; backend build/runtime pending.** The former App once-ref suppressed provisioning failures. App now starts a shared readiness check after boot, shows preparing/failure/retry, and rechecks after bundled provisioning. Record worker consumes the same state before capture. Missing resource, disk failure, and transient backend errors now surface with retry guidance. | Deploy and exercise the new GET /api/dictation/status route. Browser only reads its paired backend; desktop can provision bundled base for unconfigured/missing-base setup. Supported model file availability is explicitly not proof of inference. Older backend404 blocks capture with update/retry guidance. No fresh-install Record pass claimed. |
| Resource/dylib delivery | Tauri resources use relative bundled paths (MCP runtime, Whisper, audio helper); App resolves Whisper through Tauri. copy-whisper-model pins SHA256; sidecar script rewrites ONNX references relative to bundle Frameworks. CLI staging uses selected CARGO_TARGET_DIR and isolated smoke root. | Verify actual final bundle has correct-architecture executables/libs/models and no dependency on checkout/Homebrew. Run without developer PATH. Normal build is not proof of a signed/notarized downloadable artifact. |
| Missing Ollama or missing local model | StepModel offers Start/Get Ollama only after local selection; no auto start. Running-empty runtime displays no-model guidance and disables Use this model. Decide later explicitly says the agent cannot reply until connected. Provider discovery loading is visible. | Fresh runtime CUA: absent service, running-empty runtime, interrupted download, retry, and no-network behavior. No downloads performed in this audit. |
| Permission messaging | Info.plist includes microphone, camera, Automation, and Calendar descriptions. Folder setup is explicit. Presence of descriptions does not prove prompts or denials work. | Fresh-user native prompt/deny/retry acceptance, with each action explicitly initiated. Existing macOS grants on the development machine cannot satisfy this gate. |
| Signing, distribution, OS support | Tauri declares app+dmg, updater artifacts, macOS11 minimum; signing identity is supplied externally rather than embedded. These are configuration facts, not release verification. | Verify actual signing identity, notarization/stapling, updater signature, supported OS/architecture, relocation to Applications and launch from downloaded/quarantined DMG. |

## Personal-data scan boundary

Targeted scans of native launcher/scripts, command-center production source, backend config and startup paths found `/Users/j` references in test fixtures and explanatory comments, not in the audited shipping bootstrap. Reports, QA fixture scripts, review Vite config, benchmark results, and debug-only review targets intentionally contain task-specific locations; they are not user defaults. `DEFAULT_LOCAL_MODEL = qwen2.5:7b` is a product fallback, not evidence that onboarding has installed or connected it. No credentials were read or printed. This is a scoped audit, not a claim that every repository byte or bundled artifact has been audited.

## Verification

- New native URL validation: 8 tests passed.
- Existing boot/setup checks: 33 tests passed across BootScreen, wizardLogic, WizardShell.flow, StepReview.
- `npm run typecheck`: exit0. `git diff --check`: exit0.
- Actual `vite.resolveConfig({}, 'build')` with TAURI_ENV_PLATFORM=darwin and stale VITE_DAEMON_URL pointing to3002 rejected before any bundle output. This exercises plugin wiring, not only the pure validator.
- Additional coordinated setup edits: App.tsx, dictationReadiness.ts +13 tests; dictation.rs GET status route and dictation_status.rs file probe. Backend probe standalone rustc test passed, covering missing/empty/invalid/regular GGUF fixture without mutation. This is not a Cargo/route compilation pass.
- Latest helper checks:21/21 (13 readiness +8 build validation); tsc exit0. Record worker separately reports121 integration checks passed. Counts overlap other suites and are not a full-suite replacement.
- Source frozen after checks. New backend endpoint still needs daemon build and authenticated HTTP acceptance; no model is loaded, downloaded, or inferred by status.

## Fresh-user daemon3003 plan — not executed

This must be a new root, not a reuse/copy of3002. Keep the existing owned-process-group runner's25GiB start floor and20GiB stop floor, bounded30-minute lifetime, and secret-redacted logs. Current free space reported by root (~22GiB) is below the launch floor.

Prepare a new canonical directory named `first-user-qa-runtime` under the owned world-production target, refusing any pre-existing/nonempty directory or symlink. Inside it create a fresh `home/`, `state/`, and private `logs/`; do not copy config, secrets, identity, projects, models, or ontology from a real user. Use an empty state directory, not the existing Record prepare command (which seeds provider/model/role packs). Check port3003 is free by binding it temporarily; never stop an existing listener. Verify selected daemon/dylib hashes and signatures first.

The child command is:

```text
<verified-world-target>/debug/permagentd agent --host 127.0.0.1 --port 3003
```

Construct its environment from an empty dictionary, not os.environ, with these exact bindings:

```text
PATH=/usr/bin:/bin:/usr/sbin:/sbin
HOME=<fresh-root>/home
LANG=en_US.UTF-8
PERMAGENT_PATH_ROOT=<fresh-root>/state
PERMAGENT_DISABLE_KEYRING=1
PERMAGENT_CONFIG=<fresh-root>/state/config.yaml
PERMAGENT_SPECTRAL_DB=<fresh-root>/state/spectral/permagent.db
DYLD_LIBRARY_PATH=<verified-world-target>/debug
GOOSE_TELEMETRY_ENABLED=false
COUNCIL_ENABLED=false
CONCIERGE_ENABLED=false
STRIX_ENABLED=false
STEWARD_SCAN_ENABLED=false
POLYBOT_ENABLED=false
TUNNEL_AUTO_START=false
PERMAGENT_MESH_TRUSTED=0
PERMAGENT_MESH_ENGINE=0
OLLAMA_HOST=http://127.0.0.1:9
PERMAGENT_OLLAMA_HOST=http://127.0.0.1:9
```

Confirm port9 has no listener before using it as the intentionally unavailable Ollama endpoint; otherwise select another verified closed loopback port. These service restrictions keep the test away from the host's installed Ollama and optional background integrations. They do not seed an inference provider. Do not set GOOSE_PROVIDER, GOOSE_MODEL, role packs, API keys, HTTP proxies, browser credentials, or inherited secret-manager environment. Run with cwd=<fresh-root> and start_new_session=True. Since HOME alone does not isolate macOS Keychain, PERMAGENT_DISABLE_KEYRING is essential. Inspect the daemon executable's ancestor chain for defaults.yaml and init-config.yaml before launch (Config discovers it by executable location, not cwd).

First read-only assertions after startup: local status ready; config has no provider/model; provider list has no configured hosted key; wizard flag absent/false; Ollama unreachable; project list has no personal rows. Do not export token/config secret values into evidence. Parent can then pair a separately scoped browser preview to3003 using the daemon's own pairing flow and drive setup through Decide later. Do not redirect the production native app: it owns3001. A native fresh-user acceptance still requires a separate macOS user or a deliberately built isolated artifact later.

This plan requires a new runner/config prepared by the coordinator before execution; it is not a command to run the existing seeded3002 helper. Stop only the process group created by that runner; retain resulting fresh config as evidence. Report safety env overrides separately from genuine shipping-default behavior.

## Readiness contract and remaining limits

GET `/api/dictation/status` returns status available/unavailable, a bounded reason code, a registry model ID or null, and `inference_verified:false`. It exposes no model path or arbitrary config value. The file probe checks regular/readable file and GGUF header only; full parser/model/inference health is intentionally not claimed. Missing/disabled/unsupported/unreadable/invalid/config-unavailable have distinct reasons. Native retry preserves a user-selected model: it provisions bundled base only when no model is configured or configured base is missing. Browser performs no provisioning writes.

Fresh backend route compilation, feature-disabled compilation, real HTTP status, native missing-bundle retry, and actual first-user capture/transcription remain unverified. The App error UI deliberately does not echo filesystem errors or credentials. Existing provision route's installed-file behavior is unchanged; invalid existing model files remain blocked with repair/reinstall guidance rather than silently overwritten.

## Independent readiness review after full frontend verification

The coordinator's serial isolated full frontend suite passed3923 tests across486 files (`SHIPPING_FRONTEND_ISOLATED_2026-10-08.json`). This supersedes the earlier shared-worker test result for frontend acceptance, not the native/backend compile gates.

The selected debug daemon is still the03:10:40UTC artifact. A read-only binary scan found `/api/dictation/provision` but not `/api/dictation/status` or `inference_verified`. It cannot serve this newly added route. There is no authentic current-route HTTP shortcut without compiling the backend: a mock endpoint, a shimmed Config/AppState harness, or the standalone file probe would not test production routing/auth/config semantics. Existing-binary requests can only establish the expected old-version404 path. Do not launch below the25GiB guard floor.

Status is merged into the ordinary protected router and inherits bearer, peer-policy, and origin middleware. It exposes registry model IDs, not arbitrary configured strings or host paths. Its shared Config reader can initialize/recover/migrate configuration; the endpoint performs no explicit configuration writes, downloads, model load, or inference. This distinction corrects its previous overbroad comment.

Remaining technical limits: readiness is cached for the app session until explicit Retry, so later model removal/backend replacement can invalidate a previously available result; runtime ASR errors must remain visible. The20second abort bounds network fetches but does not independently reject a hung native resolveResource IPC promise. These are follow-up risks, not claims that either occurred. The status result only verifies a readable GGUF header, not the full file or inference.

Do not assume `--no-default-features` provides a cheap route build: existing provision_handler references the feature-gated Whisper module unconditionally. Feature-disabled compilation remains unverified, independently of the new status branch.

Fresh-user priorities, in order: (1) exact clean root/env and empty configuration/provider/credential/project inventory; (2) no-provider wizard → Decide later → honest inability to chat; (3) no-Ollama and no-installed-model guidance; (4) real new status endpoint unauthorized/origin checks, unset/unsupported/missing/invalid/available states and unchanged model choice; (5) desktop bundled-resource preparation, error/retry, and allowed user permission prompts; (6) native no-host-PATH/relocated signed app bootstrap; (7) Brain initialization on a true empty home. `state.rs` currently leaves Brain disabled when its directory or ontology is absent; do not silently seed the test ontology as the earlier synthetic runtime did. Determine the intended first-user Brain setup flow before claiming notes are catalogued on a fresh install.

The post-cleanup owned target inventory is only3.9GiB on disk: debug2.7GiB, staged985MiB, runtime103MiB, plus small support directories. No safe3–5GiB reclaim set remains under this scope while preserving binaries/libraries/models/staged artifacts. `SHIPPING_COMPILED_CLEANUP_CANDIDATES_2026-10-08.json` inventories230 regenerable build-script executables totaling326,827,184bytes (0.304GiB), with hashes/mtime/inode metadata. No deletions occurred. Rebuilding those scripts has a cost; this small gain does not satisfy the25GiB startup floor from22GiB free. No benchmark, other lane, running fixture, staged artifact, or runtime data was inspected for deletion.

The subsequently authorized fresh-Brain fix now adds production startup bootstrap before AppState; see `FRESH_BRAIN_BOOTSTRAP_2026-10-08.md`. Six standalone filesystem tests pass. Existing directories remain untouched with explicit warning if uninitialized. Real AppState integration test and serving-binary compile/runtime remain pending; the earlier “fresh Brain missing” finding is fixed in source, not yet accepted in a new runtime.
