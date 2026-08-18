# Shipping updates without disrupting keys or data

Status: findings + plan. Written 2026-08-17, ahead of a September ship.

The requirement: a user updates from inside the app and loses nothing — no keys,
no data, no re-setup. Today one of those is at real risk, and it is the one that
looks most like data loss when it happens.

---

## The blocker: the app is ad-hoc signed

`ui/desktop/src-tauri/tauri.conf.json:51` — `"signingIdentity": null`.

An ad-hoc signature is derived from the binary's own contents, so **every build
produces a different code-signing identity**. macOS binds keychain item ACLs to
that identity. A new version is therefore a different application as far as the
keychain is concerned, and it is refused access to the items the previous
version stored.

This is not speculation. It is documented in this codebase from when it
happened, at `crates/goose/src/config/base.rs:1398-1406`:

> rebuilding the desktop app changes its ad-hoc code signature, macOS binds
> keychain ACLs to that signature, and the new binary was refused access to the
> existing item. The old string match treated that denial as "keyring
> unavailable", silently swapped in an EMPTY secrets file, and every cloud
> provider then read as unconfigured. Requests went out with no auth header and
> the user saw `401 Missing bearer` — concluding their API keys had been
> deleted. **All 25 were sitting in the keychain the whole time.**

And it reproduced today, unprompted: a direct `security find-generic-password`
against the real item **hung for 15 seconds and timed out**, because a refusal
surfaces as an authorisation prompt that a headless caller cannot answer. The
same block is what wedged the daemon's provider init earlier — inside a
`tokio::time::timeout` that structurally cannot fire against a blocking sync
call.

**So on current configuration, shipping in September means every in-app update
locks users out of their own API keys, and the symptom reads as deletion.**

## The fix, and it is already half-built

`RELEASE.md:66-75` documents Apple Developer signing, and
`.github/workflows/release.yml` "signs/notarizes **if the Apple secrets
exist**". The pipeline supports it. The secrets are not set.

Setting `APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY` and the notarization
credentials gives every release the *same* Developer ID identity. The keychain
ACL then holds across versions and the problem disappears — not mitigated,
removed.

That is a purchase and a CI secret, not an engineering project. It is the single
highest-value thing to do before shipping, and nothing else on this page
substitutes for it.

## What is already safe

**User data survives updates by construction.** `Paths::base_dir()`
(`crates/goose/src/config/paths.rs:6-12`) resolves to `~/.permagent`, outside the
app bundle. An update replaces the bundle and never touches it. Sessions, the
Brain, projects, notes, recipes, schedules and analytics all persist.

**The updater cannot be hijacked by an unsigned manifest.** The Tauri updater
requires a signature, and `release.yml:100-104` **fails the release** if
`plugins.updater.pubkey` is still the literal placeholder — which it currently
is (`tauri.conf.json`). The guard is real, so this cannot ship broken by
accident. It does mean the key must be generated before the first release.

**Tag ↔ version parity is enforced** in CI (`release.yml:93-97`), so a release
cannot claim a version the config disagrees with.

## What still needs doing

### 1. Signing (blocker, must precede first ship)

Buy the Developer ID, set the secrets, set `signingIdentity`. Verify by
installing release N, storing a key, updating to N+1, and confirming the key
still reads. **That round trip is the acceptance test and it should be run
before launch, not after.**

### 2. Never report a permission failure as absence (defence in depth)

The rule is already written down at `base.rs:1408-1412`:

> a PERMISSION error must never be reported as ABSENCE, and must never cause an
> empty config to be synthesised on top of real saved data.

It should be *tested*, not just documented — a test that simulates a refused
keychain read and asserts the result is an error naming the refusal, never an
empty secret set. Without that, the next refactor can reintroduce the exact
"user thinks their keys were deleted" failure.

### 3. No keychain read may block the daemon

Provider init is fixed (the work now runs in a spawned task so the deadline can
fire), but the underlying `Config::get_secret` is synchronous and reachable from
several async paths. Every call site reachable from the daemon should be audited
for the same wedge. A blocking FFI call inside `tokio::time::timeout` is a
deadline that does not exist.

### 4. A recovery path a user can follow

Even with correct signing, a keychain item can become unreadable — restore from
backup, migration to a new Mac, ACL reset. When that happens the app should say
so precisely and offer re-entry, rather than presenting as "no keys configured".
The distinction is the whole lesson of this page: *unreadable* and *absent* are
different states and must never render the same.

### 5. Schema migrations are warn-only

`spectral_schema.rs:84` — `verify_schema_version` only **warns** on a mismatch.
That is tolerable while every change is additive (`CREATE TABLE IF NOT EXISTS`),
which it currently is. Before a release that changes a table's shape, this needs
to become a real migration path, because a warning is not a plan for a user's
existing Brain.

## Acceptance test for "clean and organized"

Before shipping, run this end to end on a real machine:

1. Install release N. Configure a provider key, create a project, record a note.
2. Update to N+1 **from inside the app**.
3. Assert, without re-entering anything: the key still reads, the project is
   there, the note is there, scheduled jobs still run, the Brain still answers.

If step 3 needs any manual repair, it is not ready to ship. Everything above is
in service of that test passing.
