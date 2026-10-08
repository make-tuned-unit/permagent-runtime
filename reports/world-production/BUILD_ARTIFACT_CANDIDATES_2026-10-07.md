# Build artifact audit and cleanup status

**Current status:** this document's original size table below is historical. The coordinator subsequently removed the audited 6,304-file `candidates` array in [ARTIFACT_REACHABILITY](ARTIFACT_REACHABILITY_2026-10-07.json): 6,330,597,068 bytes; reported free space 25.383→31.294 GiB. [Cleanup ledger](ARTIFACT_CLEANUP_2026-10-07.json). No worker deletion was performed. Current roots, failed-test 9b37 family, dylibs, review apps and runtime data were preserved. Rebuild activity invalidates treating the older table as a fresh deletion list.

The audit recursively resolves each Cargo JSON dependency tuple's fourth integer against sibling fingerprint files decoded as little-endian 64-bit hashes. Five current roots reach 1,826 fingerprints with zero unresolved edges. Every candidate filename hash maps to a known fingerprint outside that closure. All 6,304 candidates were unchanged on recheck and older than the latest 23:11:48 build start; the newest dated 20:27:17. Current native app 66fac and debug/permagent-app SHA matched the recorded isolated Record-r1 raw build hash, confirming the native root selection.

`cargo clean -p PACKAGE --target-dir /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006 --dry-run --verbose` can preview a package-scoped cleanup while retaining other packages' outputs. It does **not** select only an obsolete hash variant within that package, so it is inappropriate when a current variant must remain. Use the reviewed exact-file manifest instead. No clean command was executed. [Official Cargo clean documentation](https://doc.rust-lang.org/cargo/commands/cargo-clean.html).

## Historical inventory before the dependency-closure audit


Root: `/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006`.

No deletion was performed by this worker. The coordinator subsequently removed only the two old test executables listed below and reported 26.987 GiB free, then started a new guarded Cargo test build. **Do not delete anything while that build is active.** Reassess file timestamps and current build ownership afterwards.

| Exact family under `debug/deps` | Observed size | Status |
| --- | ---: | --- |
| `permagent-3f80f92087463ee9` executable |545,607,448 bytes|Old 20:24 core test binary; coordinator removed it. Matching fingerprint is `test-lib-permagent`.|
| `permagent_daemon-63a 696f4161b18b5` executable |723,807,328 bytes|Old 20:34 daemon test binary; coordinator removed it. Matching fingerprint is `test-lib-permagent_daemon`.|
| `libpermagent-a 7df22b63c43d173.rlib` |594,647,672 bytes|Old 20:31 core library variant. Rebuildable, but deleting sacrifices reuse of that variant; preserve current 248dc4f388df31e9.|
| `libpermagent-a 7df22b63c43d173.rmeta` |132,737,166 bytes|Metadata paired with the old library.|
| `permagent-a 7df22b63c43d173.*.rcgu.o` |16 files,402,214,248 bytes|Old variant codegen objects; evaluate only after active build finishes.|
| `permagent-3f80f92087463ee9.*.rcgu.o` |16 files,554,263,944 bytes|Old core test codegen objects; retaining them can aid same-variant relink.|
| `permagent_daemon-63a 696f4161b18b5.*.rcgu.o` |16 files,232,909,896 bytes|Old daemon test objects.|
| `permagent-9b37c58c9d9d9d6c.*.rcgu.o` |12 files,214,336,648 bytes|Incomplete 22:31 variant; fingerprint held only invoked.timestamp. Current resumed test may own this family: not yet a safe deletion candidate.|
| `staged/review-backups/6ed4d3b0fa 85` and `staged/review-backups/f268b01a 9bec` |~220 MiB combined|Older staged executable backups; not Cargo inputs. Retention is a coordinator decision.|

The remaining listed families total only about 2.2 GiB beyond the two removed executables, and some may now be active-build inputs. They do not establish enough safely reclaimable space for a 30 GiB free target. APFS shared extents/clones also make apparent size an unreliable free-space promise.

Preserve both running staged Review apps, `record-qa-runtime` data/models, all benchmark targets, current `debug/permagentd` and `debug/deps/permagentd-16367fb0e75db594`, current `libpermagent-248dc4f388df31e9.{rlib,rmeta}`, native app outputs, dependency libraries, build-script outputs, frameworks and model bundles. No broad `cargo clean`, suffix-wide library deletion or process cleanup was attempted.
