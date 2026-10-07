# Hardcode Rust round 2 report (2026-10-07)

## Hits by rule (node scripts/shipped-content-check.mjs)
Before (after merge 2d5b4e65): 274 = hardcoded-local-model 140, developer-places-projects 95, personal-hosts-ids 28, legacy-agent-alias 5, developer-did 5, regional-merchant-rules 1.
After comment/test-tail scoping: 28. After fixes and written exceptions: 0 (exit 0).

## Scanner changes
- Policy root `crates/goose/assets` was still present on the tip (not repointed); now `crates/goose/tests/fixtures`.
- Rules take `scope: "code" | "all"`. `code` skips comments, `#[cfg(test)] mod {}` blocks (brace-matched), `*.test.*`/`*.spec.*`, `tests/`, `__tests__/`, `tests.rs`, `*_test(s).rs`. `all`: developer-did, developer-wallet, personal-account-names.
- personal-hosts-ids split: tailnet/team-id/mini.local hosts (code) and new `personal-account-names` (all).
- New top-level `fileExceptions` (path globs, reasonCode, reason, reviewBy; stale or unused ones fail). New reason codes: catalogData, resolverModule, guardList.
- Removed the `terminal-reattach-label-example` line exception (it was a doc comment, now out of scope).
- Stripper unit tests added to ui/command-center/src/shippedContentCheck.test.ts (12/12 pass).

## File exceptions (all reviewBy 2027-01-31)
- config/local_model.rs: resolverModule, the one place the default id lives.
- permagent-harvester/src/config.rs: resolverModule, standalone crate; now env LOCAL_DEFAULT_MODEL first, literal fallback.
- goose-cli/src/commands/setup.rs: catalogData, picker suggestions only.
- model_review/known_models.json + inventory.rs: catalogData, report-only rows.
- providers/declarative/qwen38_split.json: catalogData, optional provider, never auto-selected.
- config/identity_name_guard.rs: guardList.

## Real leaks fixed
- strix.rs, projects.rs comment: personal account name replaced with example-user / someone-else.
- prompts/session_name.md: example project replaced with acme-shop.
- DID literals in identity/canonical.rs (doc + test), session_wing.rs, activity/ingestion.rs tests: did:example:ada-lovelace.
- librarian.rs lines 2073/2111/2173: `default_model` unbound (round-1 error) now `crate::config::local_default_model()`.
- Most other listed files (proper_noun_corrector, analytics_drain, routes, finance_statements, etc.) no longer produce hits in non-test, non-comment code; their remaining mentions are in comments/test modules (out of scope for code-scope rules). I did not audit them beyond the scanner.

## Checks
- cargo check -p permagent, -p permagent-daemon, -p permagent-cli, -p permagent-harvester: Finished (success, warnings only, pre-existing).
- cargo test -p permagent --lib -- identity_name_guard shipped_content canonical session_wing::tests ingestion wing_slug librarian: 242 passed, 0 failed, 3 ignored, exit 0.
- node scripts/shipped-content-check.mjs: exit 0.
- vitest shippedContentCheck.test.ts: 12 passed.

## Unverified
- cargo test -p permagent-daemon --lib -- strix: test binary SIGKILLed (known), so the strix.rs fixture rename is compile-checked only.
- Scanner now ignores code-scope hits in comments/tests; rust brace matching is naive (braces inside string literals in test mods could mis-size a block).
