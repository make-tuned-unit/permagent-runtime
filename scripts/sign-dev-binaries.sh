#!/usr/bin/env bash
#
# sign-dev-binaries.sh — give locally built binaries a STABLE code-signing
# identity so a macOS keychain "Always Allow" survives a rebuild.
#
# THE PROBLEM (measured on this repo's own artefacts, 2026-08-19)
# ---------------------------------------------------------------
# On Apple silicon the linker signs every Mach-O ad hoc, and cargo does nothing
# to change that. A release CLI built here reported:
#
#     Identifier=permagent-6455f961797787c0
#     CodeDirectory ... flags=0x20002(adhoc,linker-signed)
#     Signature=adhoc            TeamIdentifier=not set
#
# and its designated requirement — the thing macOS actually matches a keychain
# ACL against — was:
#
#     designated => cdhash H"722bcfac4c429c73c6f594d5feb8eb3d7ee0c1de"
#
# That requirement is a hash OF THE BINARY'S CONTENTS. Change one line of Rust,
# rebuild, and it is a different string, so every keychain item whose ACL
# trusts the old one now refuses the new one and the user is asked for their
# login password again. It is not a misconfiguration; it is what ad-hoc
# signing means. (The identifier's `-6455f961797787c0` suffix drifts for the
# same reason, but the cdhash alone is enough to break the grant.)
#
# Both binaries that read the keychain are affected: the CLI and the daemon
# share KEYRING_SERVICE = "permagent" (crates/goose/src/config/base.rs).
#
# THE FIX
# -------
# Re-sign the freshly built binaries with the machine's Developer ID and a
# PINNED identifier. A real certificate makes the designated requirement
#
#     designated => identifier permagent and anchor apple generic
#                   and certificate 1[...] and certificate leaf[...]
#                   and certificate leaf[subject.OU] = <team id>
#
# which mentions no content hash at all. Two builds of different source then
# carry byte-identical requirements, one "Always Allow" covers both, and the
# prompt stops. Pinning `--identifier` is what makes that true — without it
# codesign would default to something derived from the file, and the whole
# property would evaporate.
#
# The identifiers pinned here are deliberately the SAME ones Tauri stamps on
# the shipped bundle's copies (`permagent`, `permagentd`), so a grant made
# against /Applications/Permagent.app also covers the dev build and vice
# versa. Verify with:  codesign -d -r- <binary>
#
# WHAT THIS IS NOT
# ----------------
# It is not notarization and not part of the release path. `tauri build` still
# signs and hardens everything it bundles, from its own config; this script
# only touches the loose binaries under the cargo target directory. See
# docs/operations/DEV_SIGNING.md for the ordering argument and for exactly
# which build invocations are and are not covered.
#
# USAGE
#   scripts/sign-dev-binaries.sh                    # every known dev binary
#   scripts/sign-dev-binaries.sh permagent          # just one, by name
#   scripts/sign-dev-binaries.sh --file P --identifier I   # arbitrary Mach-O
#   scripts/sign-dev-binaries.sh --check            # report, sign nothing
#
# ENVIRONMENT
#   PERMAGENT_SIGN_IDENTITY   codesign identity to use (SHA-1 hash or full
#                             common name). Default: the first "Developer ID
#                             Application" identity in the login keychain,
#                             discovered at runtime — this repo is public and
#                             hardcodes nobody's name.
#   PERMAGENT_SIGN_HARDENED   1 to add --options runtime plus the bundle's
#                             Entitlements.plist. Off by default; see below.
#   PERMAGENT_SIGN_PROFILE    cargo profile directory. Default: release.
#   CARGO_TARGET_DIR          honoured, so lane-namespaced target dirs work.
#
# EXIT STATUS
#   0  signed, already signed, or a deliberate no-op (not macOS / no
#      certificate / nothing built yet)
#   1  a certificate WAS available and signing or verification failed — that
#      is a real fault on a machine that can sign, and stays loud
#   2  bad arguments
#
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

info()  { printf '[sign-dev-binaries] %s\n' "$*"; }
warn()  { printf '[sign-dev-binaries] %s\n' "$*" >&2; }
fatal() { printf '[sign-dev-binaries] ERROR: %s\n' "$*" >&2; exit 1; }

# ---- binaries we know about -------------------------------------------------
# name -> pinned identifier. Same string on both sides today; kept as a mapping
# because the identifier is a signing-level fact, not a filename.
#
# `permagent-app` is deliberately ABSENT. It is the Tauri shell (see
# ui/desktop/src-tauri/Cargo.toml): it does not depend on `keyring`, does not
# depend on the `permagent` crate, and never issues a SecItem call — it talks
# to the daemon over HTTP. It has no keychain ACL to preserve, and the only
# copy of it that matters is the one `tauri build` produces and signs itself
# under the bundle identifier. Signing a loose target/release/permagent-app
# would buy nothing and put us in the bundler's way.
KNOWN_NAMES=(permagent permagentd)
identifier_for() {
  case "$1" in
    permagent)  printf 'permagent' ;;
    permagentd) printf 'permagentd' ;;
    *)          return 1 ;;
  esac
}

# ---- args -------------------------------------------------------------------
CHECK_ONLY=0
EXPLICIT_FILE=""
EXPLICIT_IDENT=""
SELECTED=()
while [ $# -gt 0 ]; do
  case "$1" in
    --check)      CHECK_ONLY=1; shift ;;
    --file)       EXPLICIT_FILE="${2:-}"; shift 2 ;;
    --identifier) EXPLICIT_IDENT="${2:-}"; shift 2 ;;
    -h|--help)    sed -n '/^# USAGE/,/^# EXIT STATUS/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*)           warn "unknown option: $1"; exit 2 ;;
    *)            SELECTED+=("$1"); shift ;;
  esac
done
if { [ -n "$EXPLICIT_FILE" ] && [ -z "$EXPLICIT_IDENT" ]; } ||
   { [ -z "$EXPLICIT_FILE" ] && [ -n "$EXPLICIT_IDENT" ]; }; then
  warn "--file and --identifier must be given together"
  exit 2
fi
if [ -n "$EXPLICIT_FILE" ] && [ ${#SELECTED[@]} -gt 0 ]; then
  warn "--file cannot be combined with binary names"
  exit 2
fi

# ---- platform ---------------------------------------------------------------
# Not a failure. Linux and Windows contributors have no code signatures to
# stabilise and no keychain to placate.
if [ "$(uname -s)" != "Darwin" ]; then
  info "not macOS — nothing to sign."
  exit 0
fi
command -v codesign >/dev/null 2>&1 || { info "codesign not on PATH — skipping."; exit 0; }
command -v security >/dev/null 2>&1 || { info "security(1) not on PATH — skipping."; exit 0; }

# ---- identity ---------------------------------------------------------------
# Discovered, never hardcoded: this repository is public and a certificate
# common name is a personal identity string. An operator who has several
# Developer ID certificates, or who wants a specific one, sets
# PERMAGENT_SIGN_IDENTITY (a SHA-1 hash is the unambiguous form).
discover_identity() {
  if [ -n "${PERMAGENT_SIGN_IDENTITY:-}" ]; then
    printf '%s' "$PERMAGENT_SIGN_IDENTITY"
    return 0
  fi
  # Lines look like:  1) <40-hex sha1> "Developer ID Application: ... (TEAM)"
  # Match on the certificate TYPE only and hand back the hash, so no personal
  # name is ever carried in this script's variables, logs or output.
  security find-identity -v -p codesigning 2>/dev/null \
    | awk '/"Developer ID Application:/ { print $2; exit }'
}

IDENTITY="$(discover_identity)"
if [ -z "$IDENTITY" ]; then
  info "no Developer ID Application certificate in the keychain — skipping."
  info "  Locally built binaries keep cargo's ad-hoc signature. Everything"
  info "  still builds and runs; macOS will simply re-ask for keychain"
  info "  permission after each rebuild. Set PERMAGENT_SIGN_IDENTITY to"
  info "  choose a certificate explicitly."
  exit 0
fi
info "signing identity: ${IDENTITY:0:8}… (resolved at runtime)"

# ---- hardened runtime -------------------------------------------------------
# OFF by default, on purpose.
#
# The hardened runtime is a notarization requirement, and notarized builds come
# out of `tauri build`, which applies it from tauri.conf.json with the bundle's
# Entitlements.plist. Nothing here is ever notarized or shipped.
#
# What it would cost locally is concrete. The hardened runtime forbids a
# debugger attaching unless the binary also carries get-task-allow, so
# `rust-lldb ./target/release/permagentd` stops working. It ignores DYLD_*, so
# the dylib-injection tricks used to test the sherpa/onnxruntime pairing stop
# working. And permagentd embeds V8 (pctx_code_mode -> deno_core), which the
# hardened runtime SIGKILLs on its first JIT page unless allow-jit and
# allow-unsigned-executable-memory ride along — one forgotten flag and the
# local daemon dies at launch with no useful diagnostic.
#
# What it would buy is nothing at all for the problem at hand: the hardened
# flag is not part of the designated requirement, so the keychain grant is
# exactly as stable without it. Cost with no benefit, so: off.
#
# PERMAGENT_SIGN_HARDENED=1 turns it on together with the bundle entitlements,
# for the rare case of reproducing shipping behaviour locally.
SIGN_OPTS=()
if [ "${PERMAGENT_SIGN_HARDENED:-0}" = "1" ]; then
  ENTITLEMENTS="$ROOT/ui/desktop/src-tauri/Entitlements.plist"
  [ -f "$ENTITLEMENTS" ] || fatal "PERMAGENT_SIGN_HARDENED=1 but $ENTITLEMENTS is missing"
  SIGN_OPTS+=(--options runtime --entitlements "$ENTITLEMENTS")
  info "hardened runtime ON (entitlements: $ENTITLEMENTS)"
fi
# --timestamp=none: a secure timestamp means a network round trip to Apple on
# every local build, and timestamps only matter for artefacts that outlive the
# certificate — i.e. released ones. Dev binaries are rebuilt hourly.
SIGN_OPTS+=(--timestamp=none)

# ---- one binary -------------------------------------------------------------
# Returns 0 if the file ends up correctly signed (or was already), 1 otherwise.
sign_one() {
  local path="$1" ident="$2" current

  # Already carrying our pinned identifier and a real (non-adhoc) signature?
  # cargo does not relink when nothing changed, so this is the common case on
  # a repeat build and it saves re-hashing a 300 MB binary.
  current="$(codesign -dvv "$path" 2>&1)"
  if printf '%s' "$current" | grep -qx "Identifier=$ident" &&
     ! printf '%s' "$current" | grep -q 'Signature=adhoc'; then
    info "already signed: $path (identifier $ident)"
    return 0
  fi

  if [ "$CHECK_ONLY" = "1" ]; then
    info "would sign: $path (identifier $ident)"
    return 0
  fi

  if ! codesign --force --sign "$IDENTITY" --identifier "$ident" \
        "${SIGN_OPTS[@]}" "$path" 2>&1 | sed 's/^/[sign-dev-binaries]   /'; then
    warn "codesign failed for $path"
    return 1
  fi

  # Prove the identifier actually landed. This is the entire point of the
  # exercise; a silent default would restore the drift we came to remove.
  local after
  after="$(codesign -dvv "$path" 2>&1)"
  if ! printf '%s' "$after" | grep -qx "Identifier=$ident"; then
    warn "identifier did not stick on $path — got: $(printf '%s' "$after" | grep '^Identifier=' || echo '<none>')"
    return 1
  fi
  if printf '%s' "$after" | grep -q 'Signature=adhoc'; then
    warn "$path is still ad-hoc signed after signing"
    return 1
  fi
  info "signed: $path"
  info "  $(codesign -d -r- "$path" 2>&1 | grep '^designated' || true)"
  return 0
}

# ---- explicit single file mode ---------------------------------------------
if [ -n "$EXPLICIT_FILE" ]; then
  [ -f "$EXPLICIT_FILE" ] || fatal "no such file: $EXPLICIT_FILE"
  sign_one "$EXPLICIT_FILE" "$EXPLICIT_IDENT" || exit 1
  exit 0
fi

# ---- the built dev binaries -------------------------------------------------
PROFILE="${PERMAGENT_SIGN_PROFILE:-release}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN_DIR="$TARGET_DIR/$PROFILE"

if [ ${#SELECTED[@]} -eq 0 ]; then
  SELECTED=("${KNOWN_NAMES[@]}")
fi

rc=0
found_any=0
for name in "${SELECTED[@]}"; do
  ident="$(identifier_for "$name")" || fatal "unknown binary '$name' (known: ${KNOWN_NAMES[*]})"
  path="$BIN_DIR/$name"
  if [ ! -f "$path" ]; then
    # Not an error: `npm run build:cli` builds only the CLI, and a contributor
    # may never build the daemon at all.
    info "not built yet, skipping: $path"
    continue
  fi
  found_any=1
  sign_one "$path" "$ident" || rc=1
done

if [ "$found_any" = "0" ]; then
  info "nothing to sign under $BIN_DIR"
fi
exit "$rc"
