# Build header alignment — October 18, 2026

Removed the entire Build subtitle as requested, so the Projects picker sits beside the one-line Build title. The shared header, project launch callbacks and active-run controls remain unchanged. The existing rail tests now check the title-only block in idle and active states.

Verification: Build and ViewHeader suites passed 10 files / 53 tests; TypeScript and diff checks passed. Logs: `build-header-tests-20261018.log`, `build-header-typecheck-20261018.log`. Packaged installation and native visual check passed on October 19.


## Build evidence

App/UI source `ed03f5d97fb968c9f04a3c23086cf84d609867b5`, unchanged verified backend `227484e71`. Artifact: `/Volumes/PermagentBuilds/permagent/targets/world-production-20261006/artifacts/Permagent-ed03f5d97-20261018-202318.app`. Main executable SHA-256: `b1ff6b70d2400ae00bd7f646a01bdd8383915330ac2a7ba4e6340b26d63fed12`. Normal verification passed Developer ID/deep strict signatures, source and bundled-code provenance, resources and 332 UI hashes.

The first invocation was refused by the unchanged 12 GiB internal disk floor (11.9 GiB available); free space recovered naturally. A mistaken duplicate invocation was rejected by the existing build lock without changing the successful build. The original retry then completed normally. Successful guard logs/receipts: `20261018-201319-11216`, `20261018-202317-26608`, `20261018-202319-26638`. The log preserves the refused attempts; no guards were bypassed.

Installation resumed October 19 using the already verified artifact. The installed baseline still matched the preceding Home/Calendar release.


## Installed verification — October 19

Guarded staging, deep/strict signatures and exact installed executable/resource hashes passed. Both isolated staged tools started successfully: daemon version 1.31.0 in 2.91s and CLI version 1.31.0 in 0.04s. Installed backend health returned HTTP 200 with the expected 227484e71 backend. The existing LaunchAgent configuration is unchanged. Previous app retained at `/Applications/.Permagent-rollback-20261019-073447.app`, with baseline executable hashes verified.

The installed native Build header showed only Build, with Projects directly beside it and vertically aligned. The dropdown opened its project list and closed using the same trigger; no project/model request was launched. Escape did not dismiss the existing picker in this check; this subtitle-only change does not claim to fix that separate keyboard behavior. App remains open on Build with the dropdown closed.

Receipts: [installation](build-header-install-receipt.json), [isolated startup](build-header-version-smoke.json), [build provenance](desktop-rebuild-manifest.json). D32 acceptance is complete.
