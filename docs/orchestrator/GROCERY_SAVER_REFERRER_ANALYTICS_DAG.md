# GrocerySaver referrer analytics DAG

Status: audit complete; implementation blocked by the workspace boundary. The
GrocerySaver checkout at `/Users/j/Documents/dev/grocery-saver` is clean but
not writable from this session (`test -w` is false for the frontend, backend,
and migration paths). No live traffic, database, deployment, or sibling-repo
files were changed.

This is a surgical extension of the existing first-party Permagent relay. It
must not add a second analytics system or a third-party provider.

## Audit baseline

- `frontend/index.html` already reads `document.referrer`.
- Its first pageview sends the value in the compact `r` field. SPA route
  changes deliberately send the top-level referrer only once, while the
  `referrer` event dimension is sent on each pageview.
- `backend/src/routes/analytics.ts` already accepts `r`, clamps it to 512
  characters, stores it in `permagent_analytics_events.referrer`, and returns
  it from `/drain`.
- `backend/migrations/008_permagent_analytics.js` already creates the
  nullable `referrer` column; no duplicate referrer migration should be made.
- `sessionAttribution.ts` already derives source/medium from Reddit and other
  hosts, but that attribution is not a substitute for retaining the full
  source URL.
- The Permagent daemon already preserves the drained full referrer and the
  first-party stats route groups referrers by normalized host. The
  `observe_app(surface="analytics")` response currently exposes UTM sources,
  mediums, campaigns, and daily totals, but not the referrer-domain or
  queryable full-referrer lists.

## Sequential DAG and gates

### R0 — Freeze the current contract (read-only)

Owner: GrocerySaver collector maintainer. Inputs: the existing inline beacon,
collector route, migrations, drain response, and Permagent first-party stats.

Gate: preserve the current `r` contract for old deployed snippets; preserve
`referrer` in the drain response; do not create a second table or collector.

### R1 — Make the public payload explicit and backward compatible

Files:

- `frontend/index.html` (inline beacon)
- preferably a new pure helper under `frontend/src/lib/analyticsPayload.ts`
  plus `frontend/src/lib/analyticsPayload.test.ts`, so payload behavior is
  testable without executing inline HTML

Change:

- Send `referrer: document.referrer || null` in the canonical collect payload.
- Keep sending `r` for one compatibility window, or have the backend accept
  both while the deployed snippets roll forward. Do not send the referrer on
  every SPA route as the top-level field; that would multiply one visit across
  internal navigation.
- Keep the existing first-touch/session attribution event, but do not use its
  truncated `referrer_raw` property as the canonical full referrer.

Gate: a deterministic payload test with
`https://www.reddit.com/r/halifax/comments/abc123/grocery_thread/?context=3`
asserts the canonical `referrer` value, page path, event kind, and session id.
An empty document referrer must serialize as `null`, not the string
`"undefined"` or an unrelated stored attribution source.

### R2 — Validate and safely retain the full referrer at the site boundary

Files:

- `backend/src/routes/analytics.ts`
- a pure test module, e.g. `backend/src/routes/analytics.test.ts`

Change:

- Read `referrer` first and fall back to legacy `r` only when `referrer` is
  absent. Reject conflicting values deterministically by preferring the
  canonical field and record no warning containing the URL.
- Normalize only URLs that are useful as traffic sources: `http:` and `https:`
  URLs, plus the existing Reddit Android package URI. Reject `javascript:`,
  `data:`, credentials-bearing URLs, and malformed values.
- Store a safe full URL capped at 2,048 Unicode characters. Remove fragments
  (they are never sent in an HTTP Referer header), usernames/passwords, and
  sensitive query keys (`token`, `access_token`, `auth`, `code`, `state`,
  `session`, `password`, `secret`, `key`, `api_key`, `signature`, `email`).
  Preserve non-sensitive path and query components, including Reddit thread
  and comment context parameters, so the source can be opened later.
- Derive a normalized `referrer_domain` from the URL host (lowercase,
  remove a leading `www.`, preserve subdomains such as `old.reddit.com`).
  Keep the full safe URL in `referrer`; never replace it with the domain.
- Keep the existing 8 KB body ceiling and flat-property limits. A malformed or
  over-limit beacon remains a no-op/204 or the established non-blocking
  response; it must not take down a page load.

Migration:

- `backend/migrations/010_permagent_analytics_referrer_domain.js` (next
  migration number in the repository): add nullable `referrer_domain` and an
  index suitable for `(referrer_domain, created_at)`. The existing `referrer`
  column is already present and must not be recreated.
- Backfill only safe domains from existing `referrer` values, with malformed
  rows left nullable. Do not rewrite or expose old URLs during migration.

Gate: unit tests cover valid Reddit thread/comment URLs, `www.reddit.com`,
`old.reddit.com`, Android Reddit package URIs, empty/direct traffic, malformed
schemes, credentials, fragments, sensitive query redaction, Unicode, and the
2,048-character boundary. A route test proves both `referrer` and legacy `r`
insert the expected safe URL/domain and a conflicting payload uses canonical
`referrer`.

### R3 — Keep relay/drain lossless and idempotent

Files:

- `backend/src/routes/analytics.ts` drain response (only if a new domain field
  is exposed)
- `crates/goose-server/src/analytics_drain.rs` only if the daemon needs to
  receive a new `referrerDomain` field; deriving it from the preserved full URL
  is preferred to avoid two competing values

Change: retain the existing source row id/cursor behavior and full safe
`referrer`. A retry must not duplicate traffic. The daemon may derive the
domain from the full URL using the same normalization rules, but must not drop
the path/query needed to identify a Reddit thread or comment.

Gate: replay the same drain page twice and assert one row by
`source_event_id`; assert the stored full Reddit URL survives JSON round-trip.

### R4 — Add bounded observe_app referrer reporting

Files in the Permagent runtime (owned by the runtime analytics/UI agent):

- `crates/goose/src/app_views.rs`
- `crates/goose/src/agents/platform_extensions/app_perception.rs`
- the existing first-party Grow stats route/UI only where needed for the same
  response contract

Response shape should be bounded and explicit:

```json
"referrers": {
  "domains": {"items":[{"domain":"reddit.com","events":12}],"total":1},
  "urls": {"items":[{"url":"https://www.reddit.com/r/.../comments/...","events":4}],"total":2}
}
```

Rules: count pageviews only; exclude bots by default; exclude self-referrals;
sort count descending then name ascending; cap output to the existing
`LIST_LIMIT`; keep totals/truncation metadata; return safe full URLs only.
The Build/browser surface may make `urls.items[].url` clickable, but that is a
separate UI change and must not make `observe_app` return raw event rows.

Gate: seeded SQLite analytics rows for two Reddit threads, one blog, a
self-referral, a bot, and a direct hit prove domain grouping, URL retention,
bot filtering, deterministic ordering, and bounded output. `observe_app` must
show both `reddit.com` and the exact retained thread URL.

### R5 — End-to-end known-referrer validation (no deployment mutation)

Run against an isolated test database/server only:

1. POST a pageview with the known Reddit referrer using the canonical
   `referrer` field.
2. Assert HTTP success and inspect the isolated `permagent_analytics_events`
   row for the safe full URL and normalized domain.
3. Call `/drain` with the test key; assert the same full URL is present.
4. Ingest into a disposable Spectral/Permagent database; call the first-party
   stats endpoint and `observe_app(surface="analytics")`.
5. Assert `reddit.com` is grouped and the exact thread URL remains queryable.

This is a deterministic synthetic referer test. A real Reddit click-through
is an optional post-deploy smoke check and must be explicitly approved because
it mutates live analytics data. No deployment or live database write is part
of this DAG.

### R6 — Final verification and handoff

Gates, in order:

- frontend TypeScript/build and focused payload tests
- backend TypeScript build, focused analytics tests, and migration dry-run
- runtime focused analytics/app-perception tests (avoid broad daemon builds)
- `git diff --check`, clean ownership review, and a receipt listing exact
  payload key compatibility, limits, migration, tests, and any rows created

The task is complete only when all gates pass or a named external blocker is
recorded. Do not claim that a Reddit thread was observed until the isolated
end-to-end test or a user-approved live smoke test proves it.

