# Analytics / Build Browser Closure Receipt — 2026-09-05

## Scope

This closure pass audited the existing referrer analytics and Build-browser branch only. It did not change production source, call providers, deploy, or run Rust builds. Cost-meter, persisted-store, liveness, and unrelated harness work remain outside this branch.

## Local verification evidence

All commands were run from `ui/command-center` unless noted.

| Gate | Result |
| --- | --- |
| `npm test -- --run src/components/browser/historyLogic.test.ts src/components/browser/browserRegressions.test.ts src/lib/browserState.test.ts src/lib/browserLinks.test.ts` | PASS — 4 files, 91 tests |
| `npm test -- --run src/components/grow/analyticsFormat.test.ts src/components/grow/DailyPageviewsChart.test.tsx src/components/grow/GrowView.calendar.test.tsx` | PASS — 3 files, 17 tests |
| `npm run typecheck` | PASS |
| `git diff --check` on the analytics/browser/frontend paths | PASS |

The browser test run emits jsdom's known “navigation (except hash changes)” stderr for a navigation assertion; it does not fail the suite. The Grow test run emits the existing ReactDOMTestUtils `act` deprecation warning only.

## Wiring audited

- `Browser.tsx` records successful HTTP(S) navigation in the existing browser-history API, provides frequency-first suggestions, disables spellcheck/autocorrect/autocapitalization for the address field, and derives Back/Forward disabled state from native navigation state.
- `GrowView.tsx` exposes bounded referred-chatter links and routes clicks through the existing `openInBrowser` path. No parallel analytics or memory store was introduced.
- `DailyPageviewsChart.tsx` and `analyticsFormat.ts` provide zero-filled daily pageviews and a visible linear trendline.
- The analytics beacon/backend path accepts legacy `r`, descriptive `referrer`, and both fields with deterministic precedence; stored referrers are bounded and limited to HTTP(S). Raw valid URLs remain available for clickable referred-chatter links while normalized hosts drive grouped summaries.
- Native browser code uses an HTTP(S)-only URL guard and real webview history state. Close removes the session registry entry only after a successful native close; close failure is logged and retains the registry entry for retry. The orphan reaper remains a fallback.

## Native/source regression coverage inspected (not executed here)

Rust tests present for the branch include:

- `browser_navigation_accepts_only_http_schemes`
- `close_path_stops_media_and_keeps_registry_on_failure`
- `beacon_referrer_wire_variants_have_deterministic_precedence`
- `referrer_storage_is_bounded_and_http_only`
- browser-history persistence/rejection tests in `browser_state.rs`

The parent orchestrator reserved the Rust build slot for `close_runtime_gate`; therefore this receipt does not claim those tests ran in this pass.

## Remaining live gates

These require a running daemon/app and an actual known referral; they cannot be proven by the local frontend suite:

1. Visit a known Reddit link to the site, collect a pageview, refresh Grow, and verify the normalized Reddit domain plus the bounded clickable URL appear.
2. Click a referred-chatter link and verify it opens in Build; type an HTTP(S) URL, reload/reopen, and verify the suggestion is persisted without spellcheck/autocorrect.
3. Navigate a page through at least two destinations and verify native Back/Forward enablement and behavior.
4. Close a media-playing YouTube tab and verify audio stops before/with teardown. Current native code requests `window.stop()` and pauses/unloads media before close, but this is a best-effort evaluation and is not an observed completion acknowledgement. A failed close retains the registry entry and the orphan reaper is only a fallback.

No Reddit thread or comment is asserted to exist until the browser-provided referrer is observed in a live event.

## Actionable audit findings

1. **Live validation remains open.** The branch is locally wired and tested, but the four live gates above need one controlled runtime pass.
2. **Self-referral host source needs confirmation.** The first-party analytics query derives the self-referral host from `config.drain_url`. If that URL is the analytics relay rather than the project site URL, internal traffic may be counted as external. Confirm the authoritative project/site-host field before changing filtering; do not infer it from a referral event.
3. **Raw clickable URLs can retain query/fragment data.** The current bounded HTTP(S) policy is appropriate for opening real referred chatter, but query strings/fragments may contain sensitive campaign or visitor data. If privacy requirements demand redaction, add an explicit display/storage policy while preserving normalized-domain counts.

## Follow-up source repairs completed in this closure

- Self-referral filtering now derives the public host from `Project.site_url`, not `FirstPartyConfig.drain_url`. The drain poller receives the same authoritative site URL, so a relay host cannot cause false internal/external classification.
- Referrer persistence is now shared through `analytics_classify::sanitize_referrer`: only HTTP(S) URLs with a host survive; credentials, query strings, and fragments are removed while the origin/path remains clickable. The path is bounded and malformed/custom-scheme inputs are discarded.
- Generated direct/relay beacon snippets now normalize `document.referrer` in the browser before transmission, and the install-brief attribution example follows the same rule. Read-path display links also re-sanitize legacy rows that were stored before this change.
- Added focused regressions for project-vs-relay host selection, query/fragment/credential removal, bounded URLs, and drained referred-chatter links. Rust tests were added but intentionally not executed because the parent orchestrator reserved the Rust build slot.
- Re-ran the frontend branch gate after the source repair: 7 files and 108 tests passed; `npm run typecheck` passed.

## Closure decision

The analytics/Build-browser branch is **locally verified and source-repaired, ready for the reserved runtime gate**, not fully live-validated. No source widening is justified until the live gates establish whether the remaining issues are runtime defects or environment/setup issues.

## Native transport audit

`app_conductor` can navigate app surfaces and show/hide Build panes, but it has no URL/history/media-close actions. The existing `ui/goose2` `app-test-driver` is a localhost test plugin for that app's main webview; it does not drive the current desktop app's child Build webviews and is not an authenticated native browser seam. The current desktop crate exposes `create_browser_webview`, `navigate_browser`, `browser_nav_state`, `browser_go`, and `close_browser` only as Tauri commands. No safe authenticated CLI/integration hook was found, so the native runtime gates remain open.

Queue exact Rust filters for the runtime owner:

- `cargo test --manifest-path ui/desktop/src-tauri/Cargo.toml browser_navigation_accepts_only_http_schemes`
- `cargo test --manifest-path ui/desktop/src-tauri/Cargo.toml close_path_stops_media_and_keeps_registry_on_failure`
- `cargo test --manifest-path ui/desktop/src-tauri/Cargo.toml close_transition_requests_media_teardown_before_native_close`
- `cargo test -p permagent-daemon sanitize_referrer_keeps_clickable_path_but_drops_private_suffixes`
- `cargo test -p permagent-daemon legacy_referrer_links_are_sanitized_before_grow_display`
- `cargo test -p permagent-daemon drain_uses_project_site_url_not_relay_host_and_sanitizes_links`
