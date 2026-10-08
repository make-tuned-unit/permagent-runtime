# Fresh-user Brain bootstrap — source checkpoint

Production trace: native `daemon.rs` launches `permagentd agent`; `commands/agent.rs::run` initializes logging/config, then calls `AppState::new`. Logging creates the data root through `Paths::in_state_dir("logs")`. Config initializes config.yaml but no ontology. AppState previously skipped Brain whenever brain/ or ontology.toml was absent. Searches of desktop startup, daemon commands, config and CLI startup found no earlier Brain bootstrap. Existing unit tests deliberately create brainless AppState instances.

## Change

Added `brain_bootstrap.rs` and product-owned `brain_default_ontology.toml`, invoked from the production serving command before AppState. Baseline is only version1 plus explanatory comment: no entities, predicates, names, projects, URLs, credentials, fixtures, or model selection. The helper accepts the existing configured data root; it does not discover a personal path itself.

Directory creation is exclusive and private (0700 on Unix). Only a newly created Brain directory gets a baseline. Complete bytes are written and synced to a private temporary file (0600), then published with a hard link that cannot replace a concurrent destination. Owned temporary file is removed. On error, an empty owned directory can be removed; other contents are preserved.

Existing ontology/data, existing empty or partial directories, ordinary files, and symlinks are never repaired or replaced automatically. An interrupted setup remains explicit: production startup warns that long-term memory needs repair. A newly prepared ontology is logged as prepared for mounting, not as a completed Brain; AppState's actual success/failure logging still determines whether Brain mounted.

Keeping bootstrap in the serving command avoids changing the many deliberately brainless AppState unit fixtures. The library exports the helper for a dedicated integration gate. No Record files changed.

## Validation and limits

- Standalone rustc filesystem suite:6/6 passed. Tests cover fresh/private/idempotent setup, preservation of invalid user ontology and database, empty/interrupted directories, ordinary-file conflicts, symlink protection, missing parent error.
- A first parallel test run exposed temporary test-name collisions; unique per-process atomic suffix added and final6/6 passed. Production filenames were not affected.
- `git diff --check`: exit0; Rust files formatted.
- Pinned dependency source reviewed at Spectral7025328: ontology.rs fields entities/predicates have serde defaults; from_toml validates version1, and all entity/predicate validation iterates the empty arrays. This is compatible with the existing successful synthetic runtime's version1 empty ontology, but the shipping fix now supplies the product baseline in actual startup rather than manually seeding a QA runtime.
- Added `tests/fresh_brain_bootstrap.rs`: parses the product baseline with pinned Spectral, runs the production bootstrap helper, mounts actual AppState Brain, checks an absent memory and second-run preservation. **Not compiled/run yet**: no Cargo/native build or runtime launch was performed.

Guarded integration command after adequate headroom:

```text
cargo test -p permagent-daemon --test fresh_brain_bootstrap -- --nocapture
```

Use the coordinator's owned target/build runner and sanitized environment; do not run while disk is below its floor. This integration test is a dedicated process, pins PERMAGENT_PATH_ROOT to a temporary directory before AppState and disables Keychain. It does not install the daemon's goal-dispatch hook or perform inference.

Remaining gates: compile the serving binary with the hook, run that integration test, then launch genuinely empty3003 state through the normal daemon command without manual ontology seeding. Verify Brain mounted and a new note/catalogue association works through real API/UI. Preserve first-run no-provider behavior and existing-user data. A header/version-valid baseline alone is not proof that every runtime dependency or database mount succeeds.
