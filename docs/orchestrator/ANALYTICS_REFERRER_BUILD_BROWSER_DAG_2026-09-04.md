# Analytics referrer → Build browser DAG

This branch extends the existing first-party `analytics_events` collector and
the existing Build browser state. It does not add a second analytics or memory
store.

## Sequential gates

1. **Audit gate — passed**
   - Confirmed the browser analytics client and install snippet already capture
     `document.referrer` in the compact `r` beacon field.
   - Confirmed the daemon stores the bounded value in `analytics_events.referrer`.
   - Confirmed Grow already normalizes host counts and Build already has the
     native navigation-state seam, but raw referral links, `observe_app`
     referral domains, address suggestions, and explicit URL safety were absent
     or incomplete.

2. **Collector/reporting gate**
   - Preserve full external HTTP(S) referrer URLs for the bounded Grow
     drilldown while retaining normalized domain aggregates.
   - Add `top_referrers` to the shared `app_views::analytics_summary`, which is
     the source for both Grow and `observe_app`.
   - Accept `referrer` alongside the deployed compact `r` wire contract without
     a serde alias collision. Legacy-only, descriptive-only, identical-dual,
     and conflicting-dual payloads are covered; when both differ, the explicit
     `referrer` field wins deterministically.
   - Validate before persistence: only bounded absolute HTTP(S) URLs with a
     host are stored or exposed as referred-chatter links. `javascript:`,
     `file:`, malformed, and other custom schemes are discarded.

3. **Grow handoff gate**
   - Render exact referred-page URLs as bounded “Referred chatter” links.
   - Clicking a link calls the existing `openInBrowser` store seam and opens it
     in the Build browser; links are not sent to an external browser.

4. **Browser persistence gate**
   - Persist bounded, privacy-bounded recent/frequent HTTP(S) URLs in the
     existing daemon browser-state area (`browser_history.json`).
   - Address suggestions are frequency-first, then recency, and use a native
     datalist so they remain available after restart without a parallel store.

5. **Browser safety/controls gate**
   - Disable spellcheck, autocapitalization, and autocorrect on the address
     field.
   - Enforce HTTP(S)-only navigation at the native Tauri command boundary,
     protecting deep links and persisted state from `javascript:`, `file:`,
     `data:`, and custom schemes.
   - Back/forward remains sourced from WKWebView's actual
     `canGoBack`/`canGoForward` state and uses `goBack`/`goForward`, not a
     renderer-invented URL stack.

## Verification gates

- TypeScript focused tests: analytics-client referrer beacon and browser
  history ranking/scheme filtering.
- Rust focused tests: browser state history validation, app-view aggregation
  contract, native browser URL scheme guard, and four beacon-wire variants plus
  the actual `/collect` normalization path.
- `git diff --check`.
- Re-open verification: analytics-client/browser focused suite passed (8 tests),
  command-center typecheck passed, and `cargo check -p permagent-daemon --lib`
  passed after the dual-field and validation changes. The daemon test binary
  linked but was SIGKILLed by the environment before executing its selected
  test, so that runtime gate remains explicitly unconfirmed rather than being
  reported as green.
- Live validation still required in a running daemon/app: send a pageview from
  a known Reddit thread, refresh Grow, click the resulting URL, and exercise
  address suggestions plus back/forward after two real navigations.
