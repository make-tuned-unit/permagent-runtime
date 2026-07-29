# Local-first web analytics: relay-and-drain

**Status:** design, not built (2026-07-29)
**Goal:** real visitor data from public websites lands in Permagent's Analytics
page, with no third-party analytics vendor, no tunnel proxy, and no inbound
exposure of the home machine.

## The problem with the current model

`first_party_analytics.rs` ships a browser snippet that beacons directly to the
daemon's `/collect/{site_key}`. That works only when the visitor's browser can
reach the daemon. On a public site it cannot, for three independent reasons:

1. **Mixed content.** The site is HTTPS; the daemon is HTTP. Browsers block
   insecure requests from secure pages outright.
2. **`127.0.0.1` is the visitor's machine**, not yours. The generated snippet
   embeds whatever `ingest_base` was configured, defaulting to localhost.
3. **NAT.** The daemon sits behind a residential router with no inbound route.

Fixing only #1 and #2 (a tunnel, a port-forward) leaves a fourth problem that
is worse because it is silent: **availability**. The daemon is on a laptop/mini
that sleeps, reboots, and gets rebuilt. The beacon has no queue and no retry —
every event during downtime is lost, and nothing in the UI distinguishes "no
traffic" from "collector was asleep."

## The design: invert the direction

Do not make visitors reach the daemon. Make the daemon reach out.

```
visitor browser
    │  POST /collect        (same origin, HTTPS, no CORS, no mixed content)
    ▼
your own site's backend  ──────────► your own database (buffer)
                                            │
                                            │  GET /api/analytics/drain?since=<cursor>
                                            │  (outbound from home, shared secret)
                                            ▼
                                    Permagent daemon ──► analytics_events ──► Analytics page
```

Three properties fall out of the inversion:

- **No inbound exposure.** The home machine opens no ports and needs no tunnel;
  it only makes outbound HTTPS calls, which every NAT allows.
- **Downtime is survivable.** Events accumulate in the site's own database. The
  daemon drains whatever accrued since its cursor — asleep for a week, it
  catches up on wake with zero loss.
- **Nothing leaves your infrastructure.** Visitor data goes to your server, your
  database, then your Permagent. No analytics vendor sees it.

### On "no third-party dependencies"

Stated precisely, because absolute zero is not achievable: the site is hosted
somewhere (Railway), the domain is registered somewhere, and public HTTPS
requires a certificate authority. Those are unavoidable for *any* public
website and are already load-bearing today.

What this design eliminates is the category that actually matters: **no
third-party service receives visitor data, and no external proxy sits in the
data path.** Compare the built-in `/tunnel/start`, which relays through
`cloudflare-tunnel-proxy.michael-neale.workers.dev` — a personal Cloudflare
Worker inherited from upstream Goose. That is a third party in the data path
and is not suitable for production traffic.

### The unlock: same-origin beacons dodge ad blockers

A beacon to `https://grocerysaver.ca/collect` is same-origin. Ad blockers and
tracking-protection lists target third-party analytics *domains*; a first-party
path on your own domain is effectively unblockable. Expect materially higher
capture than any vendor script — a data-quality win, not just a privacy one.

## Implementation

### Site side (grocery-saver: Express + Knex + Postgres on Railway)

Concrete because the stack is known: single Railway service, Express serves the
built SPA, Knex migrations run automatically at boot (`db.migrate.latest()` in
`initializeServices()`).

1. **Migration** `backend/migrations/008_analytics_events.js` — follow the
   dual-dialect pattern in `006_deal_price_history.js` (Postgres in prod,
   SQLite in dev):

   | column | notes |
   |---|---|
   | `id` | bigIncrements — **the drain cursor**, must be monotonic |
   | `kind` | `'pageview'` \| `'event'` |
   | `path`, `referrer`, `name` | clamp lengths on write |
   | `visitor_hash` | computed server-side (below) |
   | `created_at` | **event time**, not ingest time |
   | `drained_at` | nullable; observability into drain lag |

2. **Collect route**, mounted **before** `app.get('*')` (the SPA fallback would
   otherwise swallow it) and **outside** `/api` (which 503s until the DB is
   ready — a pageview should never 503). Copy the `Router` + `asyncHandler` +
   `db('table').insert()` idiom from `routes/cityRequests.ts`.

3. **Drain route** `GET /api/analytics/drain?since=<id>&limit=1000`, guarded by
   the existing shared-secret idiom (`x-admin-key` / `ADMIN_API_KEY`,
   fail-closed when unset) — mirror `cityRequests.ts:120`. Returns rows with
   `id > since` ordered ascending, so the cursor is exact.

#### Gotchas the research surfaced — each one silently breaks collection

- **`sendBeacon` posts `text/plain`.** `express.json()` will not parse it. Mount
  `express.text({ type: '*/*' })` (or `json({ type: '*/*' })`) on the collect
  route only, and `JSON.parse` defensively.
- **The global rate limiter is 100 req / 15 min per IP** and applies to every
  path. A normal browsing session would be throttled mid-visit, silently losing
  pageviews. The collect route must be exempted or given its own far higher
  bucket.
- **Helmet CSP `connectSrc` is `'self'` + Mapbox.** Same-origin collection
  satisfies it as-is; any other host must be added or the browser blocks the
  beacon. (This is a third reason the current localhost snippet fails.)
- **The snippet hooks `pushState` and `popstate` but not `replaceState`.**
  React Router uses `replaceState` for some navigations, so those pageviews are
  missed today. Hook both.
- **Compute `visitor_hash` server-side**, never trust it from the client. Keep
  Permagent's privacy property: `sha256(site_salt, UA, Accept-Language, UTC
  day)` — no IP stored, rotates daily.

### Permagent side

1. **Schema.** `analytics_events` today is `(id, project_id, kind, path,
   referrer, name, visitor_hash, created_at)` with **no idempotency key**, and
   `created_at` defaults to `now()`. Both are wrong for pull ingest:
   - Add `source_event_id TEXT` + `UNIQUE(project_id, source_event_id)`, and
     insert with `INSERT OR IGNORE` — a retried or overlapping drain becomes a
     no-op instead of duplicating traffic.
   - **Always set `created_at` explicitly** from the source event. A naive
     insert stamps every pulled row with the fetch time, collapsing days of
     history into one bar.

   Ship it as `migrate_v38_to_v39` on the existing ladder in
   `session_manager.rs:990` (additive; `SPECTRAL_SCHEMA_VERSION` stays pinned,
   per the documented precedent in that file).

2. **Config** splits exactly the way `grow_analytics` already splits it:
   non-secret connection config in the project metadata bag next to the
   `first_party_analytics` key (`{ drainUrl, cursor, lastDrainAt, lastError }`,
   camelCase on disk, read-modify-write to preserve sibling keys), and the
   shared secret in the keyring via
   `Config::global().set_secret(&format!("analytics_drain_secret_{project_id}"))`
   — mirroring `analytics.rs:39`. Never put the secret in `metadata_json`.

3. **Poller** — `pub fn spawn(state: Arc<AppState>)` registered in
   `commands/agent.rs:168` beside `watcher_insights::spawn` /
   `concierge::spawn`, which is the closest existing analogue (iterates active
   projects, writes to the DB, holds `AppState`). Sleep past boot before the
   first pass, then tick every ~2 minutes. Per project: GET the drain URL with
   the secret, `INSERT OR IGNORE` the batch in one transaction, advance the
   cursor only after commit, and keep paging while a full batch returns so a
   long outage catches up promptly. Record `lastError` rather than logging into
   the void.

   **Two non-obvious requirements:**
   - **Sovereignty gate.** Outbound HTTP is not on the audited egress path;
     the only enforcement is a fail-closed check the caller must make itself.
     Replicate `analytics.rs:271`: `if sovereignty::global_sovereign_mode() {
     return Err(SovereignBlocked) }`. Omitting it means sovereign mode silently
     leaks outbound traffic.
   - **Failures are a 200 with `error` set**, not a 5xx — the established
     convention (`grow_analytics.rs:99`), because the UI must render "drain
     failing" honestly rather than as an opaque error.

4. **UI: no change required for the numbers.** `first_party_stats` aggregates
   `analytics_events` purely by `project_id` with no source discriminator, so
   drained rows light up the existing panel — chart, top pages, referrers, live
   badge — for free. The one thing worth adding is a freshness/last-drain
   indicator, because the current "receiving" badge derives from row counts and
   would read a dead poller as a quiet traffic day.

### Static sites (no backend)

The per-site collector needs a server. For static projects, deploy **one** small
collector service you own (same Express + Postgres shape, one Railway service)
and point every static site's snippet at `https://collect.<yourdomain>/…`. You
lose the same-origin ad-blocker immunity for those sites and gain a CORS
requirement, but the data path is still entirely yours. Sites that *do* have a
backend should always prefer the same-origin path.

## Alternatives considered

| Option | Why not |
|---|---|
| Permagent's built-in `/tunnel/start` | Relays through a third party's personal Cloudflare Worker. Unsuitable for production data. |
| Cloudflare Tunnel (own account) | Works, but Cloudflare is in the data path, and it does nothing about daemon downtime — events during sleep are still lost. |
| Port-forward + DDNS + Let's Encrypt | Most self-owned, but exposes the home IP, needs router control, many ISPs block :443, and it still loses every event while the daemon is down. |
| Tailscale / WireGuard | Visitors are the public internet; they cannot be on your private network. Non-starter for public-site analytics. |
| Keep PostHog | It is already wired in the frontend but inactive (no production token). It is exactly the third-party dependency to avoid. |

The drain model is the only option that removes both the inbound-exposure
problem and the availability problem at once, which is why it is recommended
over every tunnel variant.

## Build order

1. Site: migration + collect route + drain route; update the snippet to the
   same-origin path; deploy. **Verify with real traffic before touching
   Permagent** — a row count climbing in Postgres is the proof.
2. Permagent: schema (`source_event_id`, explicit `created_at`), config, poller,
   freshness indicator.
3. Remove the dead `127.0.0.1` snippet from `frontend/index.html` (currently
   live and inert on production).
