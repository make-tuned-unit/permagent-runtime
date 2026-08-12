# Proposal: `permagent device export` / `import`

**Status:** draft for ruling
**Author:** derived from the 2026-08-11 brain migration (mini → mini)
**Related:** `docs/operations/BACKUPS.md`, `crates/goose-server/src/device_registry.rs` (#628)

## Context

On 2026-08-11 a full Permagent brain was moved between two Macs by hand. It
succeeded, but only because every failure mode was found and worked around
live. None of them announced themselves — each one presented as silence or as
an unrelated symptom. That migration is the evidence base for this proposal.

Every trap below is a thing that actually happened, not a hypothetical:

| What happened | Why it mattered |
|---|---|
| `BACKUPS.md` excludes `secrets/` "by design" | A restore-from-backup would have silently dropped **every paired device**. Backup is not migration. |
| 25 API keys lived in one macOS Keychain item (`permagent`/`secrets`), outside `~/.permagent` | Copying the data directory looked complete and was not. Providers failed to initialize on the new machine. |
| `brain.id` / `brain.key` / `brain.pub` live in `brain/`, not `secrets/` | A database-only copy produces **a different brain wearing your memories** — a silent identity swap. |
| 15 of 19 `projects.root_path` values pointed at a home that did not exist | Layouts differed (`~/dev` vs `~/Documents/dev`). A prefix rewrite would have been wrong too. |
| Voice models live in `~/Library/Application Support/permagent/models`, not the `~/.permagent/models` that was copied | Two model directories. The migrated one was not the one the code reads. |

There were **four** distinct state locations. A user following the existing
docs would have found one.

## Principles

1. **Backup ≠ migration.** Backups optimise for restoring *this* machine and
   deliberately exclude credentials. Migration must move identity and secrets
   or it produces a plausible-looking impostor.
2. **Identity is state.** Brain keys, the master token, and device pairings are
   as much "the brain" as the databases are.
3. **Never migrate absolute paths.** Re-resolve against a stable key. Matching
   projects by **git remote** worked perfectly where name matching would have
   mismatched `plek`/`plekk` and missed `HarbourviewRA` → `harbourview-ra`.
4. **Fail loud.** Every failure in the real migration was silent. An import
   that cannot resolve something must say so, by name.
5. **Report what you did not bring.** "12 remapped, 3 unresolved, 1 needs the
   source machine" is trustworthy. "Done" is not.
6. **Leave the source intact.** Reversibility is what makes the operation
   comfortable to perform.

## Proposed commands

```
permagent device export [--out FILE] [--include-secrets] [--passphrase-stdin]
permagent device import  FILE [--dry-run] [--remap-projects] [--passphrase-stdin]
```

### Export

Collects **one** bundle spanning all four state locations, driven by a single
`StateInventory` defined in code (so a new state directory cannot silently fall
out of migration — the inventory is the only list, and it is what both export
and the backup job read):

- `~/.permagent/` — databases, config, schedules, recipes, skills, project docs
- `~/.permagent/brain/` — including `brain.id` / `brain.key` / `brain.pub`
- `~/.permagent/secrets/` — `daemon_token.json`, `device_tokens.json`
- `~/Library/Application Support/permagent/models/` — voice/STT/TTS assets
- The Keychain blob (`service=permagent`, `account=secrets`)

Excluded by default, with a printed note: `logs/`, `backups/`, `crashes/`.
Model weights are excluded but **listed**, with the commands to re-fetch them —
they are large, re-downloadable, and were the slowest part of the manual run.

Databases are captured with `VACUUM INTO` (already used by the backup job) so
the source daemon need not be stopped and no torn WAL can result.

### Import

Runs a **preflight** before touching anything, printing a plan:

```
Bundle: permagent-export-20260811T154712.pmg  (from Jesse's Mac mini, v1.31.0)
  brain          209 MB   memory.db, recognition.db, graph.kz   OK
  identity       brain.key present                              OK
  schema         v40 → will migrate to v41 on first boot        OK
  secrets        25 keys, 2 paired devices                      NEEDS PASSPHRASE
  projects       19 total
                   12 matched by git remote
                    3 no local checkout   → GetLadle, Teenity, E2E Harness Probe
                    4 no root_path (unchanged)
  models         2 assets not in bundle   → re-fetch: permagent voice provision
Existing ~/.permagent will be preserved at ~/.permagent.pre-import-<ts>
```

`--dry-run` stops here. Otherwise it moves the existing state aside (never
deletes), unpacks, remaps, and prints the same table as a **postflight** with
actual outcomes.

**Project remapping** resolves each `root_path` by git remote against the
target machine's checkouts, falling back to basename match, then to leaving the
path null with the project preserved. It never invents a path.

## Secrets: the decision to make

Recommended: **secrets included by default, bundle encrypted with a
user-supplied passphrase** (age or `libsodium` secretbox), and `import` refuses
to proceed without it.

Rationale: the alternative — omit secrets for safety — is precisely the
behaviour that would have silently unpaired every device in the real migration.
Users do not know that pairings live in a credential file. Default-on with
mandatory encryption is smooth *and* safe; the passphrase makes the sensitivity
legible at exactly the moment it matters.

Apple's own posture supports making this explicit rather than implicit:
keychain items are deliberately non-portable, and the recommended pattern for
apps that hold many credentials is a dedicated encrypted export/import pair.

Alternative if rejected: `--include-secrets` opt-in, with a loud preflight
warning enumerating exactly what will break without it.

## Non-goals

- Continuous multi-device sync. This is a **move**, not replication. Two
  daemons sharing one master token is the failure this must not create.
- Cross-platform migration. macOS → macOS only in v1.
- Migrating model weights. Listed and re-fetched, not shipped.

## Follow-ups this surfaced

- `BACKUPS.md` must state plainly that it is not a migration path.
- The two model directories (`~/.permagent/models` vs Application Support)
  should converge, or the split should be documented.
- After a move, the source machine must be demoted (daemon stopped, or
  re-paired as a device) so the master token is never live in two places.
