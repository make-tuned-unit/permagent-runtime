# Releasing Permagent

The whole point: **you tag a version, and everyone's app updates itself.** No
manual builds, no re-uploading, no telling users to re-download.

## How the app reaches people

`ui/desktop` (the `permagent-app` Tauri bundle) is the product. It embeds the
daemon (`permagentd`) as a sidecar and the web-search MCP runtime, and ships as
a signed `.dmg`. It carries the Tauri **updater**: on every launch it checks a
hosted manifest and, if a newer signed build exists, downloads, verifies, and
installs it, then relaunches. So the *first* install is a website `.dmg`
download; every update after that is automatic.

## Cutting a release (what you do)

```sh
# 1. Bump the version (tauri.conf.json is the source of truth)
#    edit ui/desktop/src-tauri/tauri.conf.json  "version": "1.32.0"
git commit -am "chore(release): 1.32.0"
git push

# 2. Tag it — this is the trigger
git tag v1.32.0
git push origin v1.32.0
```

That's it. `.github/workflows/release.yml` builds the universal bundle on a
macOS runner, signs it for the updater (and, once Apple secrets are set, code-
signs + notarizes it for Gatekeeper), and publishes the `.dmg` **and**
`latest.json` to a GitHub Release. Installed apps point at
`releases/latest/download/latest.json`, so they find the new version on their
next launch. `workflow_dispatch` with `dry_run` builds without publishing.

## One-time setup (do this once, before the first real release)

### 1. Updater signing keypair (required for self-update)

The updater refuses any bundle whose signature doesn't match the public key
baked into the app. Generate the pair once:

```sh
cd ui/desktop && npx @tauri-apps/cli signer generate -w ~/.permagent-updater.key
```

- It prints a **public key** → paste it into
  `ui/desktop/src-tauri/tauri.conf.json` at `plugins.updater.pubkey`
  (replacing `REPLACE_WITH_TAURI_SIGNER_PUBLIC_KEY`) and commit.
- The **private key** file contents → repo secret `TAURI_SIGNING_PRIVATE_KEY`;
  its password → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Keep the private key safe: losing it means you can't push updates to existing
installs (they'd have to re-download once from a new keypair).

### 2. Apple Developer signing + notarization (for a clean download)

Without this, the `.dmg` still works but macOS Gatekeeper makes users
right-click → Open the first time. To ship a double-click-clean download, set
these repo secrets (from your Apple Developer account):

| Secret | What |
|---|---|
| `APPLE_CERTIFICATE` | base64 of your **Developer ID Application** `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | an app-specific password (appleid.apple.com) |
| `APPLE_TEAM_ID` | your 10-char team id |

`base64 -i cert.p12 | pbcopy` gets the certificate value.

## The website download

Link the website's "Download for macOS" button at the release's `.dmg`:

```
https://github.com/make-tuned-unit/permagent-runtime/releases/latest/download/Permagent_universal.dmg
```

`releases/latest/...` always resolves to the newest release, so the button
never needs updating.

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
