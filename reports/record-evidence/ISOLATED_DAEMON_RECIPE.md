# Isolated Record daemon recipe

Prepared for parent execution after the owned debug daemon build completes. The worker has not signed libraries, prepared config, launched a daemon, or called provisioning. The Record review native app and its stage/launch are parent-owned.

From the integration worktree, inspect the freshly built `../.shared-target/world-production-20261006/debug/permagentd` and record its SHA-256. The CLI requires that exact path and hash; it will not run an installed/baseline daemon. Resolve the bundled Whisper model to an absolute canonical path (for example, the review app's `Contents/Resources/whisper/whisper-base-q8_0.gguf`). Its required SHA-256 is hardcoded from the model catalogue, `7073e51db7ab02b38cc4fceeac39adc2d7a19beb98badf66aa708f4f0ac71aa9`.

Run these explicit steps with actual paths/hash substituted:

```sh
python3 reports/record-evidence/isolated_daemon.py dry-run
python3 reports/record-evidence/isolated_daemon.py sign-libs
python3 reports/record-evidence/isolated_daemon.py prepare \
  --binary /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/debug/permagentd \
  --sha256 <fresh-daemon-sha256> \
  --whisper <canonical-bundled-whisper-file>
python3 reports/record-evidence/isolated_daemon.py run --max-seconds 7200
```

`run` stays in the foreground; use an owned command session. Keep that session available. In a separate command session:

```sh
python3 reports/record-evidence/isolated_daemon.py probe
python3 reports/record-evidence/isolated_daemon.py stop
```

Wait for the foreground supervisor's completion and final evidence before considering stop complete. Stop writes a nonce/owner-matched request consumed by the owning supervisor; it never kills a listener or acts on a stored PID. The supervisor creates a new process group and uses the existing guarded process-group shutdown, including remaining descendants. SIGINT/SIGTERM also stop its owned group. Never replace this with a global `pkill`.

## Preconditions and data boundaries

- The canonical root is `../.shared-target/world-production-20261006/record-qa-runtime`, port is exactly loopback3002. `prepare` rejects existing config/database/unknown files. It permits only the reviewed native artifact `logs/ui-heartbeat` (regular file <=32 bytes), preserving it. An owned preparation manifest enables subsequent `run` without repeating preparation.
- `sign-libs` signs and verifies only canonical sherpa/onnx debug dylibs under this target. `prepare` records their post-signature hashes; `run` rechecks all artifact hashes. Any rebuild/re-sign invalidates prior evidence; do not bypass the mismatch. The child receives only that debug directory as `DYLD_LIBRARY_PATH`. This addresses the macOS missing-rpath/unsigned-library failures documented in `scripts/test-daemon.sh` without changing installed libraries or global environment.
- `run` requires25GiB startup and20GiB continuous free disk, refuses an occupied3002, and requires `qwen2.5:7b` already listed by local Ollama. It never starts Ollama or downloads a model. Port3001 and its bearer/config are not consulted.
- Child environment is an explicit allowlist. HOME retains its existing value; `PERMAGENT_PATH_ROOT` isolates state, `PERMAGENT_DISABLE_KEYRING=1` avoids inherited credential stores. No proxy/cloud key/provider environment is imported. Ollama/Chat/Voice/model packs are configured to local qwen2.5:7b; meeting writeup is local-only and mesh trust/engine are off.
- The five starter cron recipes are tombstoned before first boot. Librarian schedule, Council, Concierge, Strix, Steward scanning, Polybot, tunnel autostart and telemetry are disabled. No gateway credentials/config are imported. Core always-running loops still exist; this is state isolation, not an OS/network sandbox. Synthetic project text can be consumed by ordinary local insights loops; do not claim all background execution is disabled.
- Root owner/config/evidence and logs are private. Generated daemon bearer is read only in memory, sent only as an Authorization header to3002, never printed or copied to argv/env/reports. HTTP failure bodies are not printed.

## Provisioning and wizard

After authenticated readiness, the supervisor calls `/api/dictation/provision` with the validated bundled path, verifies the copied model SHA-256, and checks the resulting `LOCAL_WHISPER_MODEL=base` and local-only meeting config. The existing API does not itself validate the source file hash; validation here is necessary, and the destination is checked because the API treats an existing file as installed.

`wizard_complete` remains false, with no env override. Complete the actual startup wizard through the isolated native app using existing local Ollama/qwen2.5:7b and leave optional background services off. This preserves wizard test coverage. No external/cloud setup or credentials are needed. UI completion can persist the flag normally.

The first ready JSON names the private `record-review-run-*.json` evidence and log under the isolated root. That metadata records exact binary and signed library hashes, command, PID, readiness/provisioning and final exit state. Probe is a narrow authenticated-read/model-integrity check, not proof of live recording.

## Live acceptance still required

Use the distinct Record review app (compiled3002 target) and source picker `ai.permagent.record-practice-20261007`. Verify actual native app audio chunks, stop-tail delivery, local Whisper transcripts and temporal/speaker order. Save the meeting in a synthetic project; inspect the durable triage state, Orchestrator result, relevant board cards and evidence links. Retry/reopen must not duplicate cards. Test permission denial, silent input, empty/short recordings and provider failure; do not infer these results from unit tests or a save toast.

## Narrow daemon path fixes

Budget state (Echo/Concierge), disabled-starter tombstones, project document storage, uploaded attachments, integration config and scheduler findings now use shared `Paths` state/data/config roots. The budget guards assert explicit temporary-root filenames under the existing shared test-root/env-lock convention. HOME reads for host inventory, CLI lookup and user-chosen tools remain outside this Record flow; these changes do not make arbitrary tools an OS sandbox.

Validation: six stdlib launcher tests, Python syntax/help/dry-run, and source diff check passed. Runtime launch, dylib signing, model provision and live meeting acceptance remain parent-executed checks.

## Additional backend acceptance (parent execution only)

After readiness and before/after the separate native CUA practice test:

```sh
python3 reports/record-evidence/test_isolated_whisper_http.py --execute
python3 reports/record-evidence/test_isolated_meeting_http.py --execute --timeout 240
```

Omit `--execute` for a no-network/no-mutation description. The meeting script uses two new named synthetic projects, a shared recording ID and a UI-shaped notes/transcript body. It requires exactly two commitments, retains the cancelled purchase/no-email statements without turning them into tasks, checks project-bound identities, identical-save deduplication, changed-body409, completed retry stability, note/source links and local qwen provenance. It does not invoke any goal, send, purchase or execution endpoint. It checks standard/unassigned cards with only source metadata and no active goals for its project IDs.

The session API proves Orchestrator/project/note context; the read-only isolated SQLite connection supplies hidden-session type, final transcript presence, absence of tool messages and durable triage/provenance rows. Exact-key Brain API lookup proves delivered catalogue content; undelivered outbox metadata is retained as diagnostic evidence, not counted as successful catalogue delivery. Polling has a240-second shared deadline (optionally up to600), with HTTP timeouts. Failed/skipped/timed-out states are reported as failures; no fallback synthetic success is invented.

The Whisper script verifies three original fixture hashes, resamples to16kHz mono PCM16 in a new evidence directory, verifies WAV headers, and uploads directly to the local transcription endpoint. Essential checks include launch negation, no email, cancelled purchase, no purchase task, and the final venue step-free-route commitment. It records expected versus observed synthetic text and source/upload hashes. It neither plays audio nor captures audio; it does not prove native stop-tail or speaker attribution.

Both scripts store only their synthetic project/fixture evidence and public metadata, never bearer values or unrelated project/session text. Six additional pure contract tests passed (`test_isolated_http_contracts.py`), including dropped-negation detection, tail checks, external-URL rejection and SQLite write rejection. No live acceptance calls have been run by this worker.

## Rebuilt artifacts without resetting test data

After the owning supervisor has fully stopped (no active/stop marker, port3002 free), sign the rebuilt owned debug dylibs as before. Review the new daemon SHA-256 and the current private ownership-manifest SHA-256. Then explicitly refresh the artifact contract:

```sh
shasum -a 256 /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/debug/permagentd
shasum -a 256 /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/record-qa-runtime/record-review-owner.json
python3 reports/record-evidence/isolated_daemon.py refresh-artifacts \
  --binary /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/debug/permagentd \
  --sha256 <reviewed-new-daemon-sha256> \
  --previous-manifest-sha256 <reviewed-current-manifest-sha256>
```

This command does not sign, start or stop anything. It accepts only the existing daemon path and exact same canonical dylib identity set, requires their signatures to verify, and checks the unchanged bundled Whisper identity/hash plus installed model hash when present. It requires the reviewed prior manifest hash twice before writing, preventing accidental acceptance of another refresh. Root/URL/owner identity checks reject unknown, malformed, nonprivate or symlinked ownership manifests. The owner UUID remains unchanged. Config, database, bearer and existing test artifacts are not read/copied/reset by refresh (the installed model is read only for its digest). A private `record-review-artifact-refresh-*.json` records the prior and adopted artifact contracts; only that evidence and the ownership manifest are written.

If any artifact identity/path changed, refresh refuses it. Do not work around refusal by deleting the root or editing its owner marker; inspect the build output and preserve the existing test evidence.

## Empty isolated Brain ontology

New `prepare` calls now create private `<root>/brain/ontology.toml` with exactly:

```toml
version = 1
```

For the already-owned runtime prepared before this addition, first refresh any rebuilt artifacts as above, then run while the daemon is stopped:

```sh
python3 reports/record-evidence/isolated_daemon.py setup-brain
```

`setup-brain` validates the full current ownership/artifact contract, requires no runner marker and a free3002, then creates only the Brain directory and this exact file. It imports no host ontology/data/keys. It leaves an identical file unchanged and refuses any differing existing ontology or symlink. No overwrite/delete option exists. `run` now checks for the exact empty ontology before any daemon launch or Ollama request, so a missing setup step fails clearly.

Validation after these changes:13 launcher tests and6 acceptance-contract tests passed. Coverage includes preserving existing config/database bytes and owner through a real temp-directory artifact refresh, retaining prior artifact evidence, stale manifest hashes, changed library identities, wrong daemon/model hashes, active-runner refusal, private/idempotent ontology creation and preservation of a differing ontology. Signature calls and port checks are mocked only in these pure tests; no actual signing, live runtime mutation, daemon start, inference, Cargo, native build or UI operation was performed by this worker.

## Paraphrase and unavailable-service coverage

The meeting harness now supports `--scenario original` (default), `--scenario paraphrase-a`, and `--scenario paraphrase-b`. Run the two paraphrases separately after the fixed daemon is ready:

```sh
python3 reports/record-evidence/test_isolated_meeting_http.py --execute --scenario paraphrase-a --timeout 240
python3 reports/record-evidence/test_isolated_meeting_http.py --execute --scenario paraphrase-b --timeout 240
```

Both independently worded fixtures retain exactly two positive internal commitments: preparing the launch checklist and confirming the venue step-free route. They also contain a cancelled equipment purchase and an unrelated restriction on external contact/messages. Each scenario still creates two isolated synthetic projects sharing one recording ID to exercise project-scoped identity. The two required card titles must represent distinct commitments; a combined card plus an unrelated task, missing commitment, third task or cancelled/external action fails acceptance. The helper checks each scenario's own complete transcript, typed notes, restriction sentences and final-tail wording.

The harness never selects/changes the backend prompt or model. It reads the configured local Ollama model from the isolated config and checks stored provenance against that value. Local-only privacy remains mandatory.

HTTP failures now retain structured method/path/status/category metadata;503 and transport failures are retryable within polling's deadline. Error response bodies, headers and transport-reason strings are omitted. An inspection failure for one project records its prior checks and targeted read-only SQLite fallback, then continues inspecting the other project. Every report uses scenario/time/random identity and an exclusive private file create; previous failed evidence is never overwritten. Evidence and SQLite paths reject symlinks/noncanonical paths.

Validation:12 pure HTTP contract tests pass, including all scenario fixtures, missing/combined/extra commitment detection,503/transport redaction, mocked503 inspection preserving both projects, prior-report preservation, and canonical-file guards. No live API request, daemon/inference, UI or build was performed for this expansion. Earlier failed acceptance artifacts remain unchanged.
