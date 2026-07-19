# Releasing Permagent

The whole point: **you tag a version, and everyone's app updates itself.** No
manual builds, no re-uploading, no telling users to re-download.

## How the app reaches people

`ui/desktop` (the `permagent-app` Tauri bundle) is the product. It embeds the
daemon (`permagentd`) as a sidecar binary, the sherpa-onnx/onnxruntime dylibs,
the web-search MCP runtime (bundled Node), and the local Whisper dictation
model, and ships as a `.dmg`. It carries the Tauri **updater**: on every launch
it checks a hosted manifest and, if a newer signed build exists, downloads,
verifies, and installs it, then relaunches. So the *first* install is a website
`.dmg` download; every update after that is automatic.

**Apple Silicon only** (deliberate): the bundled sherpa/onnxruntime dylibs are
arm64 prebuilts and `ort` has no darwin-x64 prebuilt, so there is no honest
universal build today. The workflow builds `aarch64-apple-darwin`.

## First-run daemon behavior

On a fresh Mac (no daemon on `127.0.0.1:3001` and no
`~/Library/LaunchAgents/ai.permagent.daemon.plist`), the app **spawns the
bundled `permagentd` sidecar itself** (`src-tauri/src/daemon.rs`) with the
same args/env the launchd plist uses, and waits for it to become healthy.
Output goes to `~/.permagent/logs/daemon-sidecar.log`; failures surface in
the window title. The child dies with the app (SIGTERM on exit) — meaning
**daemon-side scheduled jobs stop when the app closes**. Machines that ran
`permagent setup` keep their launchd-managed daemon: if the plist exists or
the port answers, the app never spawns (launchd stays the single spawner —
double-spawn caused the historical KeepAlive crash loop). A persistent
LaunchAgent installed by the app (survives app quit, starts at login) is the
long-term consumer behavior, deliberately deferred: it needs consent +
uninstall UX.

## One-time setup (in this order)

### 1. Updater signing keypair — required before ANY run, even a dry run

`bundle.createUpdaterArtifacts` makes `tauri build` sign the update bundle;
the build **hard-fails without the private key**. Generate the pair once:

```sh
cd ui/desktop && npx @tauri-apps/cli signer generate -w ~/.permagent-updater.key
```

- [ ] The printed **public key** → `ui/desktop/src-tauri/tauri.conf.json` at
      `plugins.updater.pubkey` (replacing
      `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY`) — commit it. The workflow
      refuses to publish while the placeholder is in place.
- [ ] The **private key** file contents → repo secret
      `TAURI_SIGNING_PRIVATE_KEY`.
- [ ] Its password → repo secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Keep the private key safe: losing it means existing installs can't receive
updates (they'd have to re-download once from a new keypair).

### 2. Dry run — prove the pipeline before touching Apple

- [ ] Actions → Release → Run workflow (from `main`) with **dry_run** checked.
- [ ] It builds everything and publishes nothing; the `.dmg` + updater bundle
      are attached as a workflow artifact.
- [ ] Download the artifact, install the `.dmg` on a Mac, and smoke-test
      (right-click → Open; unsigned at this stage).

### 3. Apple Developer signing + notarization (for a clean download)

Without this the `.dmg` works but Gatekeeper makes users right-click → Open
the first time. From your Apple Developer account, set these repo secrets:

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | base64 of your **Developer ID Application** `.p12` (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | an app-specific password (appleid.apple.com) |
| `APPLE_TEAM_ID` | your 10-char team id |

Notes: signing enables the hardened runtime — the entitlements the daemon
needs (V8 JIT) and the mic needs are in `src-tauri/Entitlements.plist`. The
**first signed build should be smoke-tested end-to-end** (launch, dictation,
an agent turn): hardened-runtime issues only show up signed.

### 4. Make CI actually required (repo admin)

`main` currently has **no branch protection** — nothing blocks a red merge.

- [ ] Settings → Branches → protect `main`, require status checks:
      `test (ubuntu-latest)`, `test (macos-15)`, `lint`, `build`,
      `frontend`, `tauri-shell`.

## Cutting a release (what you do every time)

```sh
# 1. Bump the version — tauri.conf.json is the source of truth
#    edit ui/desktop/src-tauri/tauri.conf.json   "version": "1.32.0"
#    mirror it in ui/desktop/package.json and ui/desktop/src-tauri/Cargo.toml,
#    then sync the shell lockfile (CI checks it with --locked):
cargo update -p permagent-app --manifest-path ui/desktop/src-tauri/Cargo.toml
git commit -am "chore(release): 1.32.0"
git push

# 2. Tag it — this is the trigger
git tag v1.32.0
git push origin v1.32.0
```

The workflow preflights (tag matches the conf version, pubkey not a
placeholder, updater key present), builds the daemon + bundle on macos-15,
signs/notarizes if the Apple secrets exist, and publishes to a GitHub Release:
the versioned `.dmg`, the updater `.tar.gz` + `.sig`, `latest.json`, and a
stable-named `Permagent.dmg`. Installed apps find the new version on their
next launch.

After the run, verify the release page has all of: `Permagent.dmg`,
`Permagent_<version>_aarch64.dmg`, `Permagent.app.tar.gz`, its `.sig`, and
`latest.json`.

**If the run fails after the tag exists:** fix, then re-run via Actions → Release
→ Run workflow **on the tag** with dry_run unchecked — it re-publishes that
tag's release.

## The website download

Evergreen URL for the "Download for macOS" button (the workflow re-uploads a
stable-named copy on every release so this never changes):

```
https://github.com/make-tuned-unit/permagent-runtime/releases/latest/download/Permagent.dmg
```

## Stability / security notes

- **The updater verifies signatures.** A tampered or unsigned bundle at the
  endpoint is rejected — the app keeps running the current version. The update
  check is failure-tolerant (offline / no manifest = silent no-op).
- **Rollback:** delete or mark the bad GitHub Release as a draft; `latest`
  falls back to the prior one. (For a forced downgrade, cut a higher version
  tag with the older code.)
- **Before recommending tailnet exposure** (`HOST=0.0.0.0`, multi-device): the
  public-route auth hardening (#630) must land — a downloaded app on loopback
  is safe; a network-bound one is not until that ships.
- **Crash reporting** (#327) is a separate ship item — wire it before wide
  distribution so field crashes reach you.
