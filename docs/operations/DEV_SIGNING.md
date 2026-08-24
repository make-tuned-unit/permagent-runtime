# Signing locally built binaries

**Problem:** macOS asks for the login-keychain password every time you use a
locally built `permagent` or `permagentd`, even after clicking *Always Allow*.

**Cause:** on Apple silicon the linker signs every Mach-O ad hoc and cargo
leaves it that way. Measured on this repo's own release CLI (2026-08-19):

```
$ codesign -dvvv target/release/permagent
Identifier=permagent-6455f961797787c0
CodeDirectory v=20400 size=1861272 flags=0x20002(adhoc,linker-signed) …
Signature=adhoc
TeamIdentifier=not set

$ codesign -d -r- target/release/permagent
designated => cdhash H"722bcfac4c429c73c6f594d5feb8eb3d7ee0c1de"
```

A keychain ACL grant is matched against the **designated requirement**. For an
ad-hoc signature that requirement is a hash of the binary's own bytes, so it
changes on every rebuild that changes anything at all — and the grant made for
yesterday's binary does not apply to today's. (The identifier's
`-6455f961797787c0` suffix drifts for the same reason.) This is what ad-hoc
signing *is*; there is no setting that makes it stable.

Both binaries that read the keychain are affected. They share
`KEYRING_SERVICE = "permagent"` — see `crates/goose/src/config/base.rs`.

**Fix:** re-sign the built binaries with the machine's Developer ID under a
pinned identifier. The requirement then contains no content hash:

```
designated => identifier permagent and anchor apple generic
              and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */
              and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */
              and certificate leaf[subject.OU] = <team id>
```

Identical for every build, so one *Always Allow* holds. The identifiers are the
same ones `tauri build` stamps on the shipped bundle's copies, so a grant made
against `/Applications/Permagent.app` covers the dev build too.

## What runs it

| Invocation | Signed? |
| --- | --- |
| `npm run build:cli` (in `ui/desktop`) | yes — signs `permagent` |
| `npm run build:daemon` | yes — signs `permagentd` |
| `npm run build:all` / `build:all:local` | yes — both, via the two above |
| `scripts/sign-dev-binaries.sh` | yes — run it yourself, any time |
| **`cargo build --release` directly** | **no** |
| **`cargo run`, `cargo test`, `tauri dev`** | **no** |
| **`scripts/test-daemon.sh`** | **no** — it ad-hoc signs the dylibs its test binary needs, which is a different problem; test binaries hold no keychain grant |

Cargo has no post-build hook, so there is no way to cover a bare `cargo build`.
If that is how you build, run the script afterwards:

```bash
cargo build --release -p permagent-cli --bin permagent
scripts/sign-dev-binaries.sh permagent
```

The script is idempotent and cheap when nothing changed: it skips a binary that
already carries the pinned identifier with a non-ad-hoc signature, which is the
normal case when cargo did not relink.

Debug builds (`cargo build`, `tauri dev`) are not signed by default. Pass
`PERMAGENT_SIGN_PROFILE=debug` if you want them to be.

## Without a certificate

Contributors and CI runners have no Developer ID. The script checks with
`security find-identity -v -p codesigning` **before** invoking `codesign`, and
exits 0 with an explanation if there is none. It also exits 0 off macOS, and
when a binary simply has not been built yet. It never fails a build for a
missing certificate.

The one case that does fail (exit 1) is a machine that *has* a certificate where
signing or verification nonetheless goes wrong — that is a real fault worth
seeing.

## Configuration

| Variable | Effect |
| --- | --- |
| `PERMAGENT_SIGN_IDENTITY` | codesign identity to use — SHA-1 hash or full common name. Default: the first `Developer ID Application` identity found at runtime. Nothing is hardcoded; this repo is public. |
| `PERMAGENT_SIGN_HARDENED` | `1` adds `--options runtime` plus `ui/desktop/src-tauri/Entitlements.plist`. Off by default. |
| `PERMAGENT_SIGN_PROFILE` | cargo profile directory. Default `release`. |
| `CARGO_TARGET_DIR` | honoured, so lane-namespaced target dirs work. |

## Why the hardened runtime is off by default

The hardened runtime is a *notarization* requirement, and notarized artefacts
come out of `tauri build`, which applies it from `tauri.conf.json` with the
bundle entitlements. Nothing this script touches is ever notarized.

Locally it costs real things:

- A debugger cannot attach to a hardened binary without `get-task-allow`, so
  `rust-lldb ./target/release/permagentd` stops working.
- `DYLD_*` is ignored under the hardened runtime, and this repo depends on it:
  `scripts/test-daemon.sh` exports `DYLD_LIBRARY_PATH` to stand in for the
  rpath cargo does not add, because without it the daemon test binary is
  SIGKILLed with no output. Hardening dev binaries would put that whole class
  of local workaround out of reach.
- `permagentd` embeds V8 (`pctx_code_mode` → `deno_core`), which the hardened
  runtime kills on its first JIT page unless `allow-jit` and
  `allow-unsigned-executable-memory` ride along — one forgotten flag and the
  local daemon dies at launch with no useful diagnostic.

And it buys nothing here: the hardened flag is not part of the designated
requirement, so the keychain grant is exactly as stable without it. Cost with no
benefit — hence off, with `PERMAGENT_SIGN_HARDENED=1` for anyone reproducing
shipping behaviour.

`--timestamp=none` is used for the same reason: a secure timestamp is a network
round trip to Apple on every local build, and it only matters for artefacts that
must outlive the certificate.

## Ordering, and why the app bundle is unaffected

`npm run build:all` runs `build:daemon` → `build:sidecar` → `build:cli` →
`build:cli-sidecar` → … → `tauri build`. Signing sits at the end of
`build:daemon` and `build:cli`, i.e. **after cargo, before the copy scripts**.

- `copy-cli.sh` copies `target/release/permagent` verbatim; it rewrites nothing,
  so the Developer ID signature travels intact instead of the ad-hoc one.
- `copy-sidecar.sh` runs `install_name_tool` on its copy of the daemon, which
  invalidates whatever signature it had, and re-applies an ad-hoc one — exactly
  as before. It never touches the original under `target/release`.
- `tauri build` then force-signs every `externalBin` with the bundle identity,
  the bundle entitlements and the hardened runtime. That is what the shipped
  `Contents/MacOS/permagent` and `Contents/MacOS/permagentd` carry, and it is
  unchanged by any of this.

So the bundle is byte-for-byte the same product; only the loose binaries under
`target/` gained a stable identity.

`permagent-app` is deliberately not signed here. It is the Tauri shell: it
depends on neither `keyring` nor the `permagent` crate (see
`ui/desktop/src-tauri/Cargo.toml`) and never makes a keychain call — it talks to
the daemon over HTTP. It has no ACL to preserve, and the only copy that matters
is the one `tauri build` produces and signs under the bundle identifier.

## Verifying it

The property is tested automatically in
`crates/goose/src/config/dev_signing_guard.rs`, run by
`cargo test -p permagent --lib`. One test reads the script and the npm wiring as
text and runs everywhere; the other builds two *different* tiny Mach-O binaries,
signs both through the script, and asserts their designated requirements are
byte-identical while their ad-hoc ones differ. The second test can only run on a
macOS machine holding a Developer ID certificate, and skips with a printed
reason anywhere else — there is no honest way to fake a certificate chain.

To check the real binaries by hand, build twice with a source change in between:

```bash
cd ui/desktop
npm run build:cli
codesign -d -r- ../../target/release/permagent > /tmp/dr1.txt

# change something in crates/goose-cli/src
npm run build:cli
codesign -d -r- ../../target/release/permagent > /tmp/dr2.txt

diff /tmp/dr1.txt /tmp/dr2.txt && echo "requirement is stable"
```

Both files should read `designated => identifier permagent and anchor apple
generic …` with no `cdhash`. If you see `designated => cdhash H"…"`, the binary
is still ad-hoc: the signing step did not run, or found no certificate.
