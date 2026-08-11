//! First-party web analytics (#23) — the daemon IS the collector; no
//! third-party analytics dependency. Jesse's ruling (2026-07-28) supersedes
//! the 2026-07-20 connector-only decision: the connector lens
//! (`grow_analytics.rs`) stays for people who already have a provider, and
//! this module adds the self-hosted path:
//!
//!   POST /api/projects/{id}/analytics/first_party/enable — mint the site key
//!        (idempotent), optionally set the ingest base URL; returns the full
//!        setup payload (snippet + coding-agent prompt).
//!   GET  /api/projects/{id}/analytics/first_party        — status + setup payload
//!   GET  /api/projects/{id}/analytics/first_party/stats  — ?days=30 aggregates
//!   POST /collect/{site_key}                             — the beacon endpoint
//!
//! The collect endpoint is deliberately **outside** both the bearer middleware
//! and the origin guard: the user's own website posts beacons cross-origin
//! from visitors' browsers, which can never hold a daemon token. Its exposure
//! is bounded: 128-bit random site key in the path, body parsed from raw
//! bytes (≤2 KiB), fixed field whitelist, per-key rate limit, and it can only
//! ever INSERT rows into `analytics_events`.
//!
//! Visitor uniques are privacy-preserving: sha256(site_key, UA,
//! Accept-Language, UTC day) — no IP stored, rotates daily.

use crate::routes::analytics_attribution as attribution;
use crate::routes::analytics_classify as classify;
use crate::routes::analytics_funnel as funnel;
use crate::routes::analytics_verify as verify;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use permagent::app_views::{self, AnalyticsWindow};
use permagent::projects::{self, Project, UpdateProject};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::sync::{Arc, Mutex, OnceLock};

/// Metadata bag key on the project (`metadata_json.first_party_analytics`).
pub const METADATA_KEY: &str = "first_party_analytics";

/// Per-site-key rate limit: events accepted per rolling minute.
const EVENTS_PER_MINUTE: u32 = 300;
/// Beacon body cap — a legitimate beacon is < 300 bytes.
const MAX_BODY_BYTES: usize = 2048;

// ── Config stored in the project metadata bag ────────────────────────────────

/// Alias for the poller: the drain-relevant view of this config.
pub type DrainState = FirstPartyConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstPartyConfig {
    pub site_key: String,
    /// Base URL visitors' browsers can reach the daemon at (LAN URL, tailnet
    /// or tunnel hostname). The snippet embeds `<ingest_base>/collect/<key>`.
    ///
    /// DIRECT MODE ONLY — unusable for a public site, where the browser can
    /// never reach a home daemon (mixed content + NAT). Public sites use drain
    /// mode below.
    #[serde(default)]
    pub ingest_base: Option<String>,

    // ── Drain mode (relay-and-drain, docs/architecture/LOCAL_FIRST_WEB_ANALYTICS.md) ──
    // The site collects same-origin into its own database and the daemon pulls
    // outbound on a timer. No inbound exposure, and events survive daemon
    // downtime because the site buffers them.
    /// Absolute URL of the site's drain endpoint, e.g.
    /// `https://example.com/api/permagent-analytics/drain`.
    #[serde(default)]
    pub drain_url: Option<String>,
    /// Last successfully ingested source event id — the pull watermark.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub last_drain_at: Option<String>,
    /// Last drain failure, surfaced in the UI so a dead poller never reads as
    /// a quiet traffic day.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Newest event id the relay reported holding at the last drain (optional
    /// `latest_id` in the drain response, spec v41). Against `cursor` this
    /// makes drain lag VISIBLE — "37 events behind" — instead of a healthy
    /// cursor and a backlog looking identical. Absent from pre-v41 relays.
    #[serde(default)]
    pub relay_latest_id: Option<String>,
}

/// Keyring key for a project's visitor-hash salt.
///
/// Permagent mints this rather than telling the install brief to "use a fixed
/// server-side salt", which left the coding agent to invent one — unrecorded,
/// unrotatable, and often just a literal it made up on the spot. The salt is
/// what stops the visitor hash being reversible by anyone who can guess a
/// user-agent string, so it is a real credential and belongs here with the
/// drain secret.
pub fn visitor_salt_key(project_id: &str) -> String {
    format!("analytics_visitor_salt_{project_id}")
}

/// Keyring key for a project's drain shared secret. Mirrors
/// `analytics::api_key_secret_key` — secrets never live in `metadata_json`.
pub fn drain_secret_key(project_id: &str) -> String {
    format!("analytics_drain_secret_{project_id}")
}

fn config_from_project(project: &Project) -> Option<FirstPartyConfig> {
    project
        .metadata_json
        .get(METADATA_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

// ── Poller API (crate::analytics_drain) ──────────────────────────────────────

/// This project's drain config, if analytics is enabled for it.
pub fn drain_config(project: &Project) -> Option<DrainState> {
    config_from_project(project)
}

/// The drain secret WITHOUT minting one — the poller must never create
/// credentials as a side effect of a background pass.
pub fn stored_drain_secret_readonly(project_id: &str) -> Option<String> {
    permagent::config::Config::global()
        .get_secret::<String>(&drain_secret_key(project_id))
        .ok()
        .filter(|k| !k.is_empty())
}

/// Persist cursor/status after a drain pass.
pub async fn persist_drain_state(
    pool: &Pool<Sqlite>,
    project: &Project,
    config: &DrainState,
) -> Result<(), String> {
    write_config(pool, project.clone(), config)
        .await
        .map(|_| ())
        .map_err(|c| format!("metadata write failed: {c}"))
}

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnableRequest {
    #[serde(default)]
    ingest_base: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupResponse {
    enabled: bool,
    site_key: Option<String>,
    ingest_base: Option<String>,
    ingest_url: Option<String>,
    snippet: Option<String>,
    agent_prompt: Option<String>,
    /// Drain mode: where the daemon pulls from, and how ingestion is going.
    drain_url: Option<String>,
    /// The shared secret, echoed so the UI can put it in the copyable brief.
    /// Local-only surface behind the bearer choke point; it is a credential the
    /// user is about to paste into their own deployment anyway.
    drain_secret: Option<String>,
    /// The visitor-hash salt, minted here rather than invented by the coding
    /// agent. Shown alongside the key so both env vars can be set in one pass.
    visitor_salt: Option<String>,
    cursor: Option<String>,
    last_drain_at: Option<String>,
    last_error: Option<String>,
    /// True once at least one event has ever been ingested for this project.
    receiving: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDrainRequest {
    /// Absolute URL of the site's drain endpoint. Empty/absent clears it.
    #[serde(default)]
    drain_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DayCount {
    day: String,
    pageviews: i64,
    visitors: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NamedCount {
    name: String,
    count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FirstPartyStats {
    enabled: bool,
    receiving: bool,
    period_days: u32,
    pageviews: i64,
    /// Distinct DEVICE SIGNATURES, not people. The hash collapses everyone
    /// sharing a browser build, OS version and language into one value, which
    /// on mobile merges many real people — so this systematically UNDERCOUNTS.
    /// The field name is deliberately not `visitors`: the UI must not present
    /// it as a precise headcount.
    device_signatures: i64,
    /// Events in the last 5 minutes — the "live" indicator.
    events_last_5m: i64,
    /// How many rows the bot filter removed from every figure above. Surfaced
    /// so a filtered number is never mistaken for a quiet day.
    bots_excluded: i64,
    /// True when bot traffic is included (the `includeBots` toggle).
    including_bots: bool,
    by_day: Vec<DayCount>,
    top_pages: Vec<NamedCount>,
    top_referrers: Vec<NamedCount>,
    top_events: Vec<NamedCount>,
    // ── v40 / traffic attribution ──
    /// First-touch `"source / medium"` ranks. Prefers `session_attribution`
    /// events; falls back to pageview referrer via the shared hostname map.
    /// Does **not** require `utm_*`.
    top_sources: Vec<NamedCount>,
    top_campaigns: Vec<NamedCount>,
    /// Answer-engine visits: `answer_engine_visit` and/or first-touch
    /// medium=`aeo` (ChatGPT, etc.) — countable without campaign params.
    aeo_visits: i64,
    /// Sessions and what they unlock. Zero/None until a relay sends session ids.
    sessions: i64,
    /// Share of sessions with exactly one pageview, 0..1. None when there are
    /// no sessions to divide by — an honest gap, not a misleading 0%.
    bounce_rate: Option<f64>,
    pages_per_session: Option<f64>,
    top_entry_pages: Vec<NamedCount>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn project_and_pool(
    state: &AppState,
    project_id: &str,
) -> Result<(Pool<Sqlite>, Project), StatusCode> {
    let pool = state
        .session_manager()
        .pool_clone()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let project = projects::get_project_by_id_or_slug(&pool, project_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((pool, project))
}

fn mint_site_key() -> String {
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

// Currently unused: the install snippet is served through the install-brief
// payload rather than generated here. Kept rather than deleted because it is
// the canonical sendBeacon/keepalive snippet and the brief may want to share
// it — delete it if the brief is now the single source of truth.
#[allow(dead_code)]
fn snippet_for(ingest_url: &str) -> String {
    // sendBeacon with a stringified body avoids a CORS preflight entirely
    // (text/plain simple request); fetch keepalive is the fallback.
    format!(
        r#"<script>
(function () {{
  var E = "{ingest_url}";
  function send(kind, name) {{
    var body = JSON.stringify({{ k: kind, p: location.pathname, r: document.referrer || null, n: name || null }});
    if (!(navigator.sendBeacon && navigator.sendBeacon(E, body))) {{
      fetch(E, {{ method: "POST", body: body, keepalive: true }}).catch(function () {{}});
    }}
  }}
  send("pv");
  var push = history.pushState;
  history.pushState = function () {{ push.apply(this, arguments); send("pv"); }};
  addEventListener("popstate", function () {{ send("pv"); }});
  window.permagent = window.permagent || {{}};
  window.permagent.event = function (name) {{ send("ev", name); }};
}})();
</script>"#
    )
}

/// The relay snippet: identical beacon, but posting to a SAME-ORIGIN path on
/// the site itself. Same-origin means no CORS, no mixed content, and — because
/// blockers target third-party analytics domains, not first-party paths —
/// materially higher capture than any vendor script.
fn relay_snippet(collect_path: &str) -> String {
    format!(
        r#"<script>
(function () {{
  var E = "{collect_path}";
  // First-party session id: sessionStorage, no cookie, never cross-site, gone
  // when the tab closes. visitor_hash rotates daily, so without this there is
  // no bounce rate, pages per session or entry page.
  var S = null;
  try {{
    S = sessionStorage.getItem("_pa_sid");
    if (!S) {{
      S = Math.random().toString(36).slice(2) + Date.now().toString(36);
      sessionStorage.setItem("_pa_sid", S);
    }}
  }} catch (e) {{ /* private mode: sessions degrade, pageviews still count */ }}
  function send(kind, name, props, ref) {{
    var body = JSON.stringify({{
      k: kind,
      // Full path+query so the server can pull utm_* from its allowlist; it
      // stores the path WITHOUT the query, so pages still aggregate.
      p: location.pathname + location.search,
      r: ref || null,
      n: name || null,
      d: props || null,
      s: S
    }});
    if (!(navigator.sendBeacon && navigator.sendBeacon(E, body))) {{
      fetch(E, {{ method: "POST", body: body, keepalive: true }}).catch(function () {{}});
    }}
  }}
  // Dedupe on pathname. React Router calls replaceState during mount, so
  // hooking both without this fires the initial pageview AND a second from the
  // wrapper — every figure permanently ~2x high, and plausible-looking.
  // Apps that sync filters into the URL via replaceState inflate much further.
  // Hooking only pushState is not the fix either: that misses Router
  // navigations and undercounts.
  var lastPath = null;
  // document.referrer keeps pointing at the ORIGINAL external referrer after a
  // client-side route change, so reporting it on every SPA navigation inflates
  // referrer counts. Send it once, on the first pageview of this page load.
  var sentRef = false;
  function pageview() {{
    if (location.pathname === lastPath) return;
    lastPath = location.pathname;
    var ref = sentRef ? null : (document.referrer || null);
    sentRef = true;
    send("pv", null, null, ref);
  }}
  pageview();
  ["pushState", "replaceState"].forEach(function (m) {{
    var orig = history[m];
    history[m] = function () {{ orig.apply(this, arguments); pageview(); }};
  }});
  addEventListener("popstate", pageview);
  window.permagent = window.permagent || {{}};
  // Second argument is a FLAT object of string/number/boolean properties, e.g.
  // permagent.event("list_item_added", {{ source: "deals", store: "sobeys" }}).
  // Nested values are dropped server-side because they cannot be grouped by.
  window.permagent.event = function (name, props) {{ send("ev", name, props, null); }};
  // Opt-in, conservative autocapture: outbound link clicks only. Call
  // permagent.autocapture() to enable. Deliberately not on by default — it
  // manufactures events the site never asked for.
  window.permagent.autocapture = function () {{
    addEventListener("click", function (e) {{
      var a = e.target && e.target.closest && e.target.closest("a[href]");
      if (!a) return;
      var url;
      try {{ url = new URL(a.href, location.href); }} catch (err) {{ return; }}
      if (url.host === location.host) return;
      send("ev", "outbound_click", {{ host: url.host, href: url.href.slice(0, 256) }}, null);
    }}, true);
  }};
}})();
</script>"#
    )
}

/// The portable install brief: paste into a coding agent inside the project's
/// own repo and it builds the relay end to end.
///
/// This replaces the old "here's a script tag" prompt, which only worked when
/// the browser could reach the daemon — never true for a public site. What the
/// agent builds instead is the site half of relay-and-drain: collect
/// same-origin into the site's own database, expose an authenticated drain
/// endpoint, and let this daemon pull outbound on a timer.
fn agent_prompt_for(
    project_name: &str,
    collect_path: &str,
    drain_path: &str,
    secret: &str,
    visitor_salt: &str,
) -> String {
    let snippet = relay_snippet(collect_path);
    format!(
        "Install self-hosted Permagent analytics for \"{project_name}\". Everything stays on \
         our own infrastructure — do NOT add Plausible, PostHog, GA, or any third-party \
         analytics service, and remove any existing ones you find.\n\n\
         ARCHITECTURE. Visitors beacon same-origin to this app; this app stores the events in \
         its OWN database; a Permagent daemon on a home machine drains them outbound on a \
         timer. The daemon is NOT reachable from the internet — never point the browser at it. \
         Buffering in our database is deliberate: the daemon is often asleep, and events must \
         survive until it wakes.\n\n\
         BEFORE YOU START, two checks that cause TOTAL SILENT FAILURE when skipped.\n\n\
         (a) CONTENT SECURITY POLICY. The snippet below is INLINE. If this app sets a CSP with \
         script-src (helmet, a _headers or netlify.toml file, an nginx config, a \
         <meta http-equiv=\"Content-Security-Policy\"> tag, or a framework default such as \
         Next.js middleware or SvelteKit's csp config), the browser will REFUSE to run it and \
         you will get zero events in every environment with no error anywhere. grep for a CSP \
         first, then deliberately pick ONE of:\n\
         - a sha256 hash of the inline script, DERIVED FROM THE BUILT HTML AT RUNTIME (preferred \
         — it survives later snippet edits);\n\
         - a nonce, if the app already has nonce plumbing;\n\
         - serve the snippet as a first-party .js file and drop the inline approach.\n\
         Do NOT hardcode a literal hash string: it works once, then breaks silently the next \
         time the snippet changes.\n\n\
         (b) IDEMPOTENCY. This brief may be run twice. Detect and SKIP anything already \
         present — a duplicated snippet double-counts, and a duplicated migration fails the \
         deploy.\n\n\
         BUILD FOUR THINGS.\n\n\
         1) A table `permagent_analytics_events` via this project's normal migration mechanism \
         (Knex/Prisma/Drizzle/SQL — match what the repo already uses; on Railway or Neon this \
         is Postgres):\n\
         - id: auto-increment integer PRIMARY KEY — this is the drain cursor and MUST be \
         monotonic\n\
         - kind: text, 'pageview' or 'event'\n\
         - path: text; referrer: text null; name: text null\n\
         - visitor_hash: text null\n\
         - created_at: timestamptz NOT NULL default now()  <- the EVENT time\n\
         - properties: jsonb null      <- event payload; add a GIN index on it\n\
         - is_bot: boolean NOT NULL default false\n\
         - session_id: text null\n\
         - utm_source / utm_medium / utm_campaign: text null\n\
         - country: text null\n\
         Index (id) and (created_at), plus (is_bot, created_at).\n\n\
         2) POST {collect_path} — the collector. Requirements that are easy to get wrong:\n\
         - navigator.sendBeacon sends Content-Type text/plain, so a JSON body parser will NOT \
         parse it. Accept text/* (or any type) on THIS ROUTE and JSON.parse defensively; \
         malformed bodies return 204, never a 500.\n\
         - Body shape: {{ k: 'pv'|'ev', p: string, r: string|null, n: string|null, \
         d: object|null, s: string|null }}. Map k='pv' to kind='pageview', k='ev' to 'event'. \
         Clamp p/r to 512 chars, n to 128, s to 64.\n\
         - `d` is the event's PROPERTIES. Keep flat scalars only (string/number/bool); drop \
         nested objects and arrays, cap at 32 keys, 256 chars per value and 4 kb total, and \
         TRUNCATE rather than reject — a rejected beacon is a silently lost event. Store as \
         jsonb in `properties`.\n\
         - `s` is a first-party session id from sessionStorage. Store verbatim in session_id.\n\
         - `p` arrives as path+query. Store `path` WITHOUT the query (or every ?utm_source= \
         variant becomes a separate page), and extract utm_source/utm_medium/utm_campaign from \
         an ALLOWLIST — utm_*, ref, gclid, fbclid — never the whole query string, which routinely \
         carries emails and tokens.\n\
         - Set is_bot from the User-Agent server-side (bot/crawler/spider/headless/curl/ \
         python-requests/facebookexternalhit/…; a MISSING user agent counts as a bot). Store the \
         flag, never the agent string.\n\
         - If you can resolve country cheaply from an edge header (Cloudflare CF-IPCountry, \
         Vercel x-vercel-ip-country, Fly Fly-Client-IP region), store the two-letter code and \
         DISCARD the IP. Never persist the IP itself.\n\
         - EXEMPT this route from global rate limiting, or a normal browsing session gets \
         throttled and silently loses pageviews.\n\
         - Register it BEFORE any SPA catch-all route, or the catch-all swallows it.\n\
         - Do NOT gate it behind auth, and do not let it 503 while the DB warms up — a \
         pageview must never block or error a visitor.\n\
         - Compute visitor_hash SERVER-side, never from the client: \
         sha256(PERMAGENT_ANALYTICS_SALT + user-agent + accept-language + UTC date), store the \
         first 32 hex chars. Read the salt from the env var — do NOT invent one, and do NOT \
         commit it. Do NOT store IP addresses. The daily date component makes the hash rotate \
         every 24h, which is what keeps this privacy-preserving; the salt is what stops it \
         being reversible by anyone who can guess a user-agent string.\n\
         - Respond 202 with an empty body.\n\n\
         3) GET {drain_path} — the authenticated drain, for the daemon only.\n\
         - Auth: header `x-permagent-key` must equal env var PERMAGENT_ANALYTICS_KEY. Fail \
         CLOSED with 401 when the env var is unset or the header does not match. Do not accept \
         the key in a query param.\n\
         - Query: ?since=<id>&limit=<n> (default limit 500, cap 1000).\n\
         - Return rows with id > since, ORDERED BY id ASC, as JSON:\n\
         {{ \"events\": [ {{ \"id\": 123, \"kind\": \"pageview\", \"path\": \"/deals\", \
         \"referrer\": null, \"name\": null, \"visitorHash\": \"ab12…\", \
         \"at\": \"2026-07-29T15:04:05.000Z\", \"properties\": null, \"isBot\": false, \
         \"sessionId\": \"s1\", \"utmSource\": null, \"utmMedium\": null, \
         \"utmCampaign\": null, \"country\": null }} ] }}\n\
         - The fields after `at` are optional: an older relay that omits them still drains \
         correctly, it simply carries no dimensions. Send them when you have them.\n\
         - `at` MUST be the row's created_at in ISO-8601 UTC. The daemon stores it verbatim; \
         if you send the current time instead, every chart collapses into one day.\n\
         - Ordering and the id > since filter are the entire correctness story — the daemon \
         re-requests from its last cursor after any failure.\n\n\
         4) The browser snippet, injected on every page just before </head> (root layout for \
         Next.js/Astro/Remix, index.html for a Vite SPA):\n\n{snippet}\n\n\
         Paste it VERBATIM. In particular the endpoint must remain the relative path \
         \"{collect_path}\" exactly as written — echo that string back in your report. NEVER \
         substitute an absolute host: http://127.0.0.1:… beacons to each VISITOR'S own machine, \
         and any http:// URL is blocked as mixed content from an HTTPS page. The lastPath guard \
         is load-bearing; without it React Router's mount-time replaceState double-counts every \
         pageview.\n\n\
         If this app PRERENDERS or statically generates routes (headless Chrome at build time, \
         next export, astro build): (i) confirm the snippet survives into the built HTML for \
         every route — prerender post-processing sometimes strips <script> tags by regex, \
         written for some third-party provider but happy to eat this one too; and (ii) suppress \
         beacons during build-time renders, or each build records a phantom pageview per route.\n\n\
         CONFIG. Set BOTH of these on THE SERVICE THAT RUNS THIS APP'S SERVER PROCESS — not \
         the database service. On Railway/Render/Fly a project has several services and putting \
         them on Postgres looks completely correct while producing a 401 that is \
         indistinguishable from a wrong key. Redeploy/restart the service after adding them. \
         Both values are generated by Permagent; use them exactly as given:\n\n\
             PERMAGENT_ANALYTICS_KEY={secret}\n\
             PERMAGENT_ANALYTICS_SALT={visitor_salt}\n\n\
         On Vercel that is exactly these two commands, run once per environment \
         (repeat with `preview` and `development` if you deploy those):\n\n\
             vercel env add PERMAGENT_ANALYTICS_KEY production\n\
             vercel env add PERMAGENT_ANALYTICS_SALT production\n\n\
         Each prompts for the value; paste the matching one above. Then redeploy — \
         env changes do not apply to an existing deployment.\n\n\
         The KEY authenticates the drain. The SALT is what makes the visitor hash \
         non-reversible. Add both to .env.example as empty placeholders. NEVER commit the real \
         values.\n\n\
         RETENTION. The events table grows without bound — the drain is a read-only cursor and \
         never deletes. Add a prune step (e.g. delete rows older than 90 days) or state the \
         retention window you chose in your report.\n\n\
         VERIFY BEFORE YOU REPORT DONE. This is not optional and it is not a prose \
         checklist — it is one call that decides whether the install works:\n\n\
             POST <your-hub>/api/projects/<this-project>/analytics/first_party/verify\n\
             {{\"origin\": \"https://your-deployed-site\"}}\n\n\
         Run it against the DEPLOYED origin after the deploy finishes. It fetches the \
         served HTML, checks the Content-Security-Policy actually permits the snippet, \
         posts a real beacon, confirms one page load records exactly one pageview, and \
         exercises the drain with a correct AND a deliberately wrong key. It returns \
         pass/fail per assertion with what to change on each failure.\n\n\
         Do NOT report success because you got a drain URL, because the build passed, or \
         because you curled the collector yourself. Every failure mode here is silent — \
         fire-and-forget beacons, 202 responses, empty catch blocks, and a 401 that looks \
         identical whether the key is wrong, unset, or set on the wrong service. Two \
         previous installs were reported complete and were not; both would have been \
         caught by this call. If verification fails, fix and re-run until it passes, or \
         say plainly that it does not pass and which assertion failed.\n\n\
         REPORT PARTIAL WORK HONESTLY. If you finish only some of the four, say exactly which \
         — a committed snippet with no table, no endpoints and no migration looks like success \
         and produces silence.\n\n\
         MIGRATING OFF AN EXISTING PROVIDER. When you replace posthog.capture / mixpanel.track / \
         analytics.track calls, CARRY THE PROPERTIES ACROSS — they are the whole value of the \
         event. posthog.capture('list_item_added', {{ store: 'sobeys', price: 4.99 }}) becomes \
         permagent.event('list_item_added', {{ store: 'sobeys', price: 4.99 }}), NOT \
         permagent.event('list_item_added'). Dropping the second argument silently destroys the \
         data that answers every product question, and nothing will ever surface the loss. If \
         any call site's properties cannot be carried over, list it explicitly in your report.\n\n\
         SESSION TRAFFIC ATTRIBUTION (optional but recommended for every project). \
         Emit once per browser session, ideally BEFORE the first pageview, so Traffic \
         sources and funnel slices work without relying on utm_* (organic search, \
         social, and answer engines usually have empty UTMs):\n\
         \n\
         permagent.event('session_attribution', {{\n\
           source: 'google',\n\
           medium: 'organic',\n\
           referrer_raw: document.referrer || '',\n\
           landing_path: location.pathname,\n\
           timestamp: new Date().toISOString()\n\
         }});\n\
         \n\
         When the referrer is an answer engine (chatgpt.com, …), also emit\n\
         permagent.event('answer_engine_visit', {{ source: 'chatgpt' }});\n\
         \n\
         Permagent falls back to a shared referrer map when these events are absent:\n\
         google→google/organic, bing→bing/organic, duckduckgo→duckduckgo/organic,\n\
         chatgpt→chatgpt/aeo, reddit→reddit/social, yahoo→yahoo/organic,\n\
         youtube→youtube/social, other host→host/referral, empty→direct/none.\n\
         Bind session via beacon `s` / sessionId — do not require a project-specific \
         cookie name.\n\
         \n\
         WHERE THINGS GO, by stack (match the repo; these are the usual answers):\n\
         - Next.js app router: snippet in app/layout.tsx (next/script, strategy=afterInteractive \
         or a raw <script> in <head>); routes as app/api/permagent-analytics/{{collect,drain}}/route.ts; \
         CSP in next.config.js headers() or middleware.ts.\n\
         - Vite SPA + Express: snippet in index.html; routes on the Express app BEFORE the SPA \
         catch-all; CSP wherever helmet is configured.\n\
         - Astro: snippet in the base layout; routes as src/pages/api/…; CSP in the adapter or \
         host headers.\n\
         - Remix / React Router: snippet in root.tsx; routes as resource routes; CSP in \
         entry.server.tsx.\n\
         - SvelteKit: snippet in src/app.html; routes as +server.ts; CSP in svelte.config.js \
         (kit.csp).\n\
         - Rails: snippet in app/views/layouts/application.html.erb; routes + controller; CSP in \
         config/initializers/content_security_policy.rb.\n\
         - Django: snippet in the base template; a view + urls.py entry; CSP via django-csp.\n\
         - Laravel: snippet in resources/views/layouts/app.blade.php; routes/web.php or api.php; \
         CSP in middleware.\n\n\
         If this app has no server-side runtime at all (a purely static site), STOP and report \
         that — do not invent a third-party service or a serverless vendor to work around it.\n\n\
         VERIFY BEFORE REPORTING DONE — against the DEPLOYED origin, not localhost. Every \
         failure mode of this install is silent (fire-and-forget beacons, 202 responses, \
         empty catch blocks, a 401 that looks like a wrong key), so skipping this step is how \
         an install ships broken and looks finished.\n\
         - Fetch the deployed HTML for TWO different routes and confirm the snippet is in both, \
         with the relative endpoint intact.\n\
         - Confirm no CSP blocks it: either there is no script-src, or its hash/nonce covers \
         the snippet. Check the browser console for a Content Security Policy violation.\n\
         - Load a page and confirm the beacon POST returns 202 in the browser network tab, and \
         that a single load produces EXACTLY ONE pageview row (not two).\n\
         - Confirm a row actually landed: select the newest few rows from \
         permagent_analytics_events.\n\
         - Curl the drain with the key and confirm JSON comes back:\n\
           curl -s -H \"x-permagent-key: $PERMAGENT_ANALYTICS_KEY\" \
         '<deployed-origin>{drain_path}?since=0&limit=5'\n\
         - Confirm the drain returns 401 with a wrong key.\n\
         Paste the output of those checks into your report, then report the deployed origin and \
         the full drain URL, which get pasted back into Permagent.\n\n\
         Permagent then runs the SAME assertions itself against the deployed origin before it \
         starts ingesting — fetching the HTML on two routes, reading the CSP, posting a real \
         beacon and exercising the drain with a correct and a wrong key. It reports pass/fail \
         per check. If your report and that run disagree, the run is right."
    )
}

/// Same-origin paths the generated relay uses. Fixed (not configurable) so the
/// prompt, the daemon's expectations, and the site's implementation can never
/// drift apart.
pub const COLLECT_PATH: &str = "/api/permagent-analytics/collect";
pub const DRAIN_PATH: &str = "/api/permagent-analytics/drain";

fn setup_response(
    config: Option<&FirstPartyConfig>,
    project: &Project,
    drain_secret: Option<&str>,
    receiving: bool,
) -> SetupResponse {
    match config {
        None => SetupResponse {
            enabled: false,
            site_key: None,
            ingest_base: None,
            ingest_url: None,
            snippet: None,
            agent_prompt: None,
            drain_url: None,
            drain_secret: None,
            visitor_salt: None,
            cursor: None,
            last_drain_at: None,
            last_error: None,
            receiving,
        },
        Some(c) => {
            // The install brief IS the deliverable: paste it into a coding
            // agent in the project's repo and it builds the site half.
            // Both credentials are minted here, so the brief the user pastes
            // is complete: nothing is left for the coding agent to invent.
            let visitor_salt = stored_visitor_salt(&project.id);
            let agent_prompt = drain_secret.map(|secret| {
                agent_prompt_for(
                    &project.name,
                    COLLECT_PATH,
                    DRAIN_PATH,
                    secret,
                    visitor_salt.as_deref().unwrap_or(""),
                )
            });
            // Direct mode stays available for local dev (a browser on THIS
            // machine really can reach the daemon); it is simply never right
            // for a public site, which is what drain mode exists for.
            let ingest_url = c
                .ingest_base
                .as_deref()
                .map(|b| format!("{}/collect/{}", b.trim_end_matches('/'), c.site_key));
            SetupResponse {
                enabled: true,
                site_key: Some(c.site_key.clone()),
                ingest_base: c.ingest_base.clone(),
                ingest_url,
                snippet: Some(relay_snippet(COLLECT_PATH)),
                agent_prompt,
                drain_url: c.drain_url.clone(),
                drain_secret: drain_secret.map(str::to_owned),
                visitor_salt: visitor_salt.clone(),
                cursor: c.cursor.clone(),
                last_drain_at: c.last_drain_at.clone(),
                last_error: c.last_error.clone(),
                receiving,
            }
        }
    }
}

/// Base URL guess from the request's Host header — right for same-machine
/// dev, and the UI lets the user override it for anything public.
// Currently unused: callers derive the base URL from the stored project site
// URL instead of the request headers. Kept as the header-derivation fallback
// for a hosting setup where the site URL is not recorded.
#[allow(dead_code)]
fn base_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| format!("http://{h}"))
}

async fn has_any_events(pool: &Pool<Sqlite>, project_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM analytics_events WHERE project_id = ?1 LIMIT 1",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map(|n| n > 0)
    .unwrap_or(false)
}

// ── Protected handlers ───────────────────────────────────────────────────────

async fn get_setup(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    _headers: HeaderMap,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let config = config_from_project(&project);
    let receiving = has_any_events(&pool, &project.id).await;
    let secret = stored_drain_secret(&project.id);
    Ok(Json(setup_response(
        config.as_ref(),
        &project,
        secret.as_deref(),
        receiving,
    )))
}

/// The project's drain secret, minted on first read so the install brief always
/// carries one (the user should never have to invent a credential).
/// The project's visitor-hash salt, minting one on first read. Same
/// mint-on-demand shape as the drain secret so enabling analytics produces a
/// complete, pasteable configuration in one step.
fn stored_visitor_salt(project_id: &str) -> Option<String> {
    let key = visitor_salt_key(project_id);
    if let Ok(existing) = permagent::config::Config::global().get_secret::<String>(&key) {
        if !existing.is_empty() {
            return Some(existing);
        }
    }
    let bytes: [u8; 32] = rand::random();
    let minted = hex::encode(bytes);
    match permagent::config::Config::global().set_secret(&key, &minted) {
        Ok(_) => Some(minted),
        Err(e) => {
            tracing::warn!("could not persist analytics visitor salt: {e}");
            None
        }
    }
}

fn stored_drain_secret(project_id: &str) -> Option<String> {
    let key = drain_secret_key(project_id);
    if let Ok(existing) = permagent::config::Config::global().get_secret::<String>(&key) {
        if !existing.is_empty() {
            return Some(existing);
        }
    }
    let bytes: [u8; 32] = rand::random();
    let minted = hex::encode(bytes);
    match permagent::config::Config::global().set_secret(&key, &minted) {
        Ok(_) => Some(minted),
        Err(e) => {
            tracing::warn!("could not persist analytics drain secret: {e}");
            None
        }
    }
}

/// Mint a NEW drain secret, replacing the current one.
///
/// There was no rotation path at all, which matters more than usual here: the
/// secret is embedded in the install brief, so it lands in the coding agent's
/// transcript, its context window and often its tool logs. A credential that
/// has been through a third-party model's context should be rotatable without
/// tearing the whole install down.
///
/// Rotating INVALIDATES the deployed site until the new value is set on the
/// app service and it redeploys — draining will 401 in the meantime, which the
/// UI already surfaces. The response carries the fresh brief to paste.
async fn rotate_drain_secret(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let config = config_from_project(&project).ok_or(StatusCode::NOT_FOUND)?;
    let bytes: [u8; 32] = rand::random();
    let minted = hex::encode(bytes);
    permagent::config::Config::global()
        .set_secret(&drain_secret_key(&project.id), &minted)
        .map_err(|e| {
            tracing::warn!("could not rotate analytics drain secret: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    tracing::info!(
        target: "permagentd::analytics",
        project = %project.name,
        "analytics drain secret rotated — the deployed site will 401 until the new value is set"
    );
    let receiving = has_any_events(&pool, &project.id).await;
    Ok(Json(setup_response(
        Some(&config),
        &project,
        Some(minted.as_str()),
        receiving,
    )))
}

/// Point the daemon at the site's drain endpoint (the value the coding agent
/// reports back after installing the relay). Clearing it stops ingestion.
async fn set_drain(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    body: Option<Json<SetDrainRequest>>,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let requested = body
        .and_then(|Json(b)| b.drain_url)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(url) = requested.as_deref() {
        // Reject anything the poller could not use, loudly, at config time —
        // a bad URL discovered only by a silent background failure is the worst
        // version of this.
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let mut config = config_from_project(&project).ok_or(StatusCode::NOT_FOUND)?;
    // A changed target is a different data source; reset the watermark so the
    // new site is drained from its beginning rather than from a stale cursor.
    if config.drain_url.as_deref() != requested.as_deref() {
        config.cursor = None;
        config.last_error = None;
    }
    config.drain_url = requested;

    let updated = write_config(&pool, project, &config).await?;
    let receiving = has_any_events(&pool, &updated.id).await;
    let secret = stored_drain_secret(&updated.id);
    Ok(Json(setup_response(
        Some(&config),
        &updated,
        secret.as_deref(),
        receiving,
    )))
}

/// Read-modify-write the project's metadata bag, preserving sibling keys.
async fn write_config(
    pool: &Pool<Sqlite>,
    project: Project,
    config: &FirstPartyConfig,
) -> Result<Project, StatusCode> {
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    metadata.as_object_mut().expect("object ensured").insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(config).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    let id = project.id.clone();
    Ok(projects::update_project(
        pool,
        &id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .unwrap_or(project))
}

async fn enable_first_party(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    _headers: HeaderMap,
    body: Option<Json<EnableRequest>>,
) -> Result<Json<SetupResponse>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let requested_base = body
        .and_then(|Json(b)| b.ingest_base)
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());

    let mut config = config_from_project(&project).unwrap_or_else(|| FirstPartyConfig {
        site_key: mint_site_key(),
        ingest_base: None,
        drain_url: None,
        cursor: None,
        last_drain_at: None,
        last_error: None,
        relay_latest_id: None,
    });
    if requested_base.is_some() {
        config.ingest_base = requested_base;
    }

    // Read-modify-write the metadata bag, preserving unrelated keys.
    let mut metadata = if project.metadata_json.is_object() {
        project.metadata_json.clone()
    } else {
        serde_json::json!({})
    };
    metadata.as_object_mut().expect("object ensured").insert(
        METADATA_KEY.to_string(),
        serde_json::to_value(&config).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    let updated = projects::update_project(
        &pool,
        &project.id,
        UpdateProject {
            metadata_json: Some(metadata),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .unwrap_or(project);

    let receiving = has_any_events(&pool, &updated.id).await;
    let secret = stored_drain_secret(&updated.id);
    Ok(Json(setup_response(
        Some(&config),
        &updated,
        secret.as_deref(),
        receiving,
    )))
}

#[derive(Debug, Deserialize)]
struct StatsQuery {
    #[serde(default)]
    days: Option<u32>,
    /// Include traffic classified as automated. Off by default — crawler
    /// traffic on an SEO-heavy site otherwise dominates every figure.
    #[serde(default)]
    include_bots: Option<bool>,
}

/// Query for `/analytics/first_party/funnel`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FunnelQuery {
    /// Ordered, comma-separated steps: `/pricing,event:checkout_started,event:purchase`.
    steps: String,
    days: Option<i64>,
    include_bots: Option<bool>,
    /// Property holding a numeric amount on the final step, e.g. `value`.
    /// Named rather than assumed, because a site's field might be `amount`
    /// or `total` and silently reading the wrong one would misreport revenue.
    value_key: Option<String>,
    /// Optional first-touch traffic source filter (e.g. `google`, `chatgpt`).
    source: Option<String>,
    /// Optional first-touch medium filter (e.g. `organic`, `aeo`, `social`).
    medium: Option<String>,
}

/// Where visitors drop out of a funnel, and what completing it is worth.
///
/// The rows already carry everything needed — named events, first-party session
/// ids and properties — so this is pure analysis over what the collector
/// stores. Sequencing lives in `analytics_funnel`, where it is unit-tested
/// against the ways a funnel can lie (out-of-order hits, repeat visits,
/// later steps exceeding earlier ones).
async fn first_party_funnel(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(q): Query<FunnelQuery>,
) -> Result<Json<funnel::Funnel>, StatusCode> {
    let (pool, _project) = project_and_pool(&state, &project_id).await?;

    let steps: Vec<funnel::Step> = q.steps.split(',').filter_map(funnel::Step::parse).collect();
    if steps.is_empty() || steps.len() > 8 {
        // A funnel with no steps is meaningless; more than eight is almost
        // always a mistaken query string, and each step costs a scan.
        return Err(StatusCode::BAD_REQUEST);
    }

    let days = q.days.unwrap_or(30).clamp(1, 365);
    let since = format!("-{days} days");
    let bot_filter = if q.include_bots.unwrap_or(false) {
        ""
    } else {
        " AND is_bot = 0"
    };

    // One scan for all steps; the step each row satisfies is resolved in Rust.
    let sql = format!(
        "SELECT session_id, kind, path, name, created_at, properties
         FROM analytics_events
         WHERE project_id = ?1 AND created_at >= datetime('now', ?2){bot_filter}
         ORDER BY created_at"
    );
    let rows = sqlx::query_as::<
        _,
        (
            Option<String>,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
        ),
    >(&sql)
    .bind(&project_id)
    .bind(&since)
    .fetch_all(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // First-touch source/medium per session — same map as Traffic sources —
    // so funnels can slice by traffic source for every drained project.
    let traffic = attribution::rollup_traffic_sources(
        &pool,
        &project_id,
        &since,
        q.include_bots.unwrap_or(false),
    )
    .await;
    let allowed = attribution::sessions_matching(
        &traffic.by_session,
        q.source.as_deref(),
        q.medium.as_deref(),
    );

    let value_key = q.value_key.unwrap_or_else(|| "value".to_string());
    let mut touches = Vec::new();
    let mut excluded_sessionless: u64 = 0;

    for (session_id, kind, path, name, created_at, properties) in rows {
        let matched = steps.iter().position(|s| match s {
            funnel::Step::Path { value } => kind == "pageview" && &path == value,
            funnel::Step::Event { value } => {
                kind == "event" && name.as_deref() == Some(value.as_str())
            }
        });
        let Some(step_index) = matched else { continue };
        // Only count a sessionless row against the exclusion if it would
        // otherwise have mattered — counting every unrelated pageview would
        // make the figure meaningless as a confidence signal.
        let Some(session_id) = session_id else {
            excluded_sessionless += 1;
            continue;
        };
        if let Some(ref allow) = allowed {
            if !allow.contains(&session_id) {
                continue;
            }
        }
        let value = properties
            .as_deref()
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .and_then(|v| v.get(&value_key).and_then(|n| n.as_f64()));
        touches.push(funnel::Touch {
            session_id,
            step_index,
            at: created_at,
            value,
        });
    }

    Ok(Json(funnel::compute(
        &steps,
        &touches,
        excluded_sessionless,
    )))
}

async fn first_party_stats(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<FirstPartyStats>, StatusCode> {
    let (pool, project) = project_and_pool(&state, &project_id).await?;
    let enabled = config_from_project(&project).is_some();
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let since = format!("-{days} days");

    // Bots are excluded from EVERY figure by default. On a site with 50
    // prerendered SEO pages, crawler traffic is a large share of all requests
    // and counting it makes the numbers noise. `?includeBots=true` opts back in.
    let including_bots = q.include_bots.unwrap_or(false);
    // Interpolated, not bound: it is a fixed fragment chosen from two literals,
    // never user input.
    let bot_filter = if including_bots {
        ""
    } else {
        " AND is_bot = 0"
    };

    // Shared with `observe_app`: the Grow UI and Henry must use one definition
    // of visitors, pageviews, bot filtering, top paths, and campaigns.
    let shared = app_views::analytics_summary(
        &pool,
        &project.id,
        AnalyticsWindow::days(days),
        including_bots,
        10,
    )
    .await
    .map_err(|e| {
        tracing::warn!("first-party analytics aggregation failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let bots_excluded = shared.bot_events;
    let pageviews = shared.pageviews;
    let device_signatures = shared.unique_visitors;

    // ── Sessions (v40) ──
    // visitor_hash rotates daily, so none of this is computable without a
    // session id. Rows predating the relay change have NULL and are simply not
    // counted, rather than being invented as one-page sessions.
    let (sessions, session_pageviews, bounced): (i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT count(*), coalesce(sum(views), 0), coalesce(sum(views = 1), 0) FROM (
           SELECT session_id, count(*) AS views
           FROM analytics_events
           WHERE project_id = ?1 AND kind = 'pageview' AND session_id IS NOT NULL
             AND created_at >= datetime('now', ?2){bot_filter}
           GROUP BY session_id
         )"
    ))
    .bind(&project.id)
    .bind(&since)
    .fetch_one(&pool)
    .await
    .unwrap_or((0, 0, 0));
    let bounce_rate = (sessions > 0).then(|| bounced as f64 / sessions as f64);
    let pages_per_session = (sessions > 0).then(|| session_pageviews as f64 / sessions as f64);

    let events_last_5m: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM analytics_events
         WHERE project_id = ?1 AND created_at >= datetime('now', '-5 minutes')",
    )
    .bind(&project.id)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    let by_day: Vec<DayCount> = sqlx::query_as::<_, (String, i64, i64)>(&format!(
        "SELECT date(created_at), count(*), count(DISTINCT visitor_hash)
         FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview'
           AND created_at >= datetime('now', ?2){bot_filter}
         GROUP BY date(created_at) ORDER BY date(created_at)"
    ))
    .bind(&project.id)
    .bind(&since)
    .fetch_all(&pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(day, pageviews, visitors)| DayCount {
        day,
        pageviews,
        visitors,
    })
    .collect();

    let named = |rows: Vec<(String, i64)>| -> Vec<NamedCount> {
        rows.into_iter()
            .map(|(name, count)| NamedCount { name, count })
            .collect()
    };

    let top_pages = shared
        .top_paths
        .items
        .iter()
        .map(|item| NamedCount {
            name: item.name.clone(),
            count: item.count,
        })
        .collect();

    // Raw referrers, grouped by HOST rather than full URL — a hundred distinct
    // deep links from one site are one source, not a hundred.
    let referrer_rows = sqlx::query_as::<_, (String, i64)>(&format!(
        "SELECT referrer, count(*) FROM analytics_events
         WHERE project_id = ?1 AND kind = 'pageview' AND referrer IS NOT NULL
           AND referrer <> '' AND created_at >= datetime('now', ?2){bot_filter}
         GROUP BY referrer ORDER BY count(*) DESC LIMIT 200"
    ))
    .bind(&project.id)
    .bind(&since)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();

    // Self-referrals are internal navigation, not a traffic source. On an SPA
    // document.referrer still holds the ORIGINAL external referrer after a
    // client-side route change, so without this every internal navigation
    // re-reports it and inflates the list.
    let site_host = config_from_project(&project)
        .and_then(|c| c.drain_url)
        .and_then(|u| classify::referrer_host(&u));

    let mut by_host: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (referrer, count) in &referrer_rows {
        let class = classify::classify_referrer(Some(referrer), site_host.as_deref());
        if class == classify::ReferrerClass::Internal {
            continue;
        }
        if let Some(host) = classify::referrer_host(referrer) {
            *by_host.entry(host).or_default() += count;
        }
    }

    let sort_desc = |mut v: Vec<NamedCount>| -> Vec<NamedCount> {
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        v.truncate(10);
        v
    };
    let top_referrers = sort_desc(
        by_host
            .into_iter()
            .map(|(name, count)| NamedCount { name, count })
            .collect(),
    );

    // Traffic sources: first-touch source/medium per session. Prefer
    // session_attribution; fall back to pageview.referrer. Never require utm_*.
    let traffic =
        attribution::rollup_traffic_sources(&pool, &project.id, &since, including_bots).await;
    let top_sources: Vec<NamedCount> = traffic
        .top_sources
        .into_iter()
        .map(|(name, count)| NamedCount { name, count })
        .collect();
    let aeo_visits = traffic.aeo_visits;

    let top_campaigns = shared
        .top_utm_campaigns
        .items
        .iter()
        .map(|item| NamedCount {
            name: item.name.clone(),
            count: item.count,
        })
        .collect();

    // Entry pages: the first pageview of each session. Feeds the funnel view
    // and answers "where do people land?".
    let top_entry_pages = named(
        sqlx::query_as::<_, (String, i64)>(&format!(
            "SELECT path, count(*) FROM (
               SELECT session_id, path,
                      row_number() OVER (PARTITION BY session_id ORDER BY id) AS rn
               FROM analytics_events
               WHERE project_id = ?1 AND kind = 'pageview' AND session_id IS NOT NULL
                 AND created_at >= datetime('now', ?2){bot_filter}
             ) WHERE rn = 1
             GROUP BY path ORDER BY count(*) DESC LIMIT 10"
        ))
        .bind(&project.id)
        .bind(&since)
        .fetch_all(&pool)
        .await
        .unwrap_or_default(),
    );

    let top_events = named(
        sqlx::query_as::<_, (String, i64)>(&format!(
            "SELECT coalesce(name, '(unnamed)'), count(*) FROM analytics_events
             WHERE project_id = ?1 AND kind = 'event'
               AND created_at >= datetime('now', ?2){bot_filter}
             GROUP BY name ORDER BY count(*) DESC LIMIT 10"
        ))
        .bind(&project.id)
        .bind(&since)
        .fetch_all(&pool)
        .await
        .unwrap_or_default(),
    );

    let receiving = pageviews > 0 || has_any_events(&pool, &project.id).await;
    Ok(Json(FirstPartyStats {
        enabled,
        receiving,
        period_days: days,
        pageviews,
        device_signatures,
        events_last_5m,
        bots_excluded,
        including_bots,
        by_day,
        top_pages,
        top_referrers,
        top_events,
        top_sources,
        top_campaigns,
        aeo_visits,
        sessions,
        bounce_rate,
        pages_per_session,
        top_entry_pages,
    }))
}

// ── Install verification ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyRequest {
    /// Deployed origin, e.g. `https://www.grocerysaver.ca`.
    origin: String,
    /// Extra route to check besides `/`. The snippet being on the home page
    /// only is a common partial install.
    #[serde(default)]
    second_route: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResponse {
    verified: bool,
    checks: Vec<verify::Check>,
    summary: String,
}

/// Run the install checks against a DEPLOYED origin.
///
/// This exists because analytics has no natural error signal: a broken install
/// looks exactly like a quiet one. It does not trust the site's own report —
/// it fetches the HTML, reads the CSP, posts a real beacon, and exercises the
/// drain with a correct and an incorrect key.
async fn verify_install(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(req): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, StatusCode> {
    let (_pool, project) = project_and_pool(&state, &project_id).await?;
    let config = config_from_project(&project).ok_or(StatusCode::NOT_FOUND)?;
    let origin = req.origin.trim().trim_end_matches('/').to_string();
    if !origin.starts_with("http://") && !origin.starts_with("https://") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let secret = stored_drain_secret_readonly(&project.id);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        // A redirect to a login wall would otherwise be reported as "snippet
        // missing", which sends the user hunting in the wrong place.
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // ── HTML on two routes, and the CSP from the first ──
    let mut html_by_route: Vec<(String, Result<String, String>)> = Vec::new();
    let mut csp: Option<String> = None;
    let routes = ["/".to_string(), req.second_route.unwrap_or_default()];
    for route in routes.iter().filter(|r| !r.is_empty()) {
        let url = format!("{origin}{route}");
        match client.get(&url).send().await {
            Ok(resp) => {
                if csp.is_none() {
                    csp = resp
                        .headers()
                        .get("content-security-policy")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                }
                match resp.text().await {
                    Ok(body) => {
                        // A meta tag counts too — frameworks set it that way.
                        if csp.is_none() {
                            // Search AND slice the same lowercased string. The
                            // previous version found the index in
                            // `body.to_lowercase()` and then sliced `body` —
                            // lowercasing can change byte length for non-ASCII
                            // input, so that offset could be wrong, land
                            // mid-character, or sit past the end and panic.
                            // CSP directives are case-insensitive, so matching
                            // on the lowered copy loses nothing here.
                            let lowered = body.to_lowercase();
                            if let Some(idx) =
                                lowered.find("http-equiv=\"content-security-policy\"")
                            {
                                if let Some(content) = lowered
                                    .get(idx..)
                                    .and_then(|tail| tail.split("content=\"").nth(1))
                                {
                                    if let Some(end) = content.find('"') {
                                        if let Some(value) = content.get(..end) {
                                            csp = Some(value.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        html_by_route.push((route.clone(), Ok(body)));
                    }
                    Err(e) => html_by_route.push((route.clone(), Err(e.to_string()))),
                }
            }
            Err(e) => html_by_route.push((route.clone(), Err(e.to_string()))),
        }
    }

    // ── Drain before, beacon, drain after ──
    // The delta is how we assert "exactly one pageview per load" without any
    // access to the site's database.
    let drain_url = config
        .drain_url
        .clone()
        .unwrap_or_else(|| format!("{origin}{DRAIN_PATH}"));
    let drain_get = |since: String, key: Option<String>| {
        let client = client.clone();
        let url = format!(
            "{drain_url}{}since={since}&limit=1000",
            if drain_url.contains('?') { '&' } else { '?' }
        );
        async move {
            let mut req = client.get(&url);
            if let Some(k) = key {
                req = req.header("x-permagent-key", k);
            }
            req.send().await
        }
    };

    let mut before_max = "0".to_string();
    let mut drain_result: Option<(u16, bool)> = None;
    if let Some(key) = secret.clone() {
        if let Ok(resp) = drain_get("0".to_string(), Some(key)).await {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let parsed: Option<serde_json::Value> = serde_json::from_str(&body).ok();
            let is_json = parsed
                .as_ref()
                .map(|v| v.get("events").is_some())
                .unwrap_or(false);
            drain_result = Some((status, is_json));
            if let Some(events) = parsed.as_ref().and_then(|v| v["events"].as_array()) {
                for ev in events {
                    let id = ev["id"]
                        .as_i64()
                        .map(|n| n.to_string())
                        .or_else(|| ev["id"].as_str().map(str::to_string));
                    if let Some(id) = id {
                        if id.parse::<i64>().unwrap_or(0) >= before_max.parse::<i64>().unwrap_or(0)
                        {
                            before_max = id;
                        }
                    }
                }
            }
        }
    }

    // One real beacon, exactly as a browser would send it.
    let beacon_status = client
        .post(format!("{origin}{COLLECT_PATH}"))
        .header("content-type", "text/plain")
        .header(
            "user-agent",
            // Marked so the row it creates is classified as a bot and never
            // pollutes the user's real figures.
            "Mozilla/5.0 (compatible; PermagentVerify/1.0; +headless) HeadlessChrome/1.0",
        )
        .body(r#"{"k":"pv","p":"/__permagent_verify","r":null,"n":null}"#)
        .send()
        .await
        .ok()
        .map(|r| r.status().as_u16());

    // Give the site a moment to commit before re-reading.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let mut pageview_delta: Option<i64> = None;
    if let Some(key) = secret.clone() {
        if let Ok(resp) = drain_get(before_max.clone(), Some(key)).await {
            if let Ok(body) = resp.text().await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    pageview_delta = v["events"].as_array().map(|a| {
                        a.iter()
                            .filter(|e| e["path"].as_str() == Some("/__permagent_verify"))
                            .count() as i64
                    });
                }
            }
        }
    }

    // A deliberately wrong key must be refused.
    let wrong_key_status = drain_get("0".to_string(), Some("permagent-verify-wrong-key".into()))
        .await
        .ok()
        .map(|r| r.status().as_u16());

    let checks = verify::build_checks(
        &html_by_route,
        csp.as_deref(),
        beacon_status,
        pageview_delta,
        drain_result,
        wrong_key_status,
    );
    let verified = verify::verdict(&checks);
    let summary = verify::summarize(&checks);
    tracing::info!(
        target: "permagentd::analytics",
        project = %project.name, %verified, "install verification run"
    );
    Ok(Json(VerifyResponse {
        verified,
        checks,
        summary,
    }))
}

// ── Public collect endpoint ──────────────────────────────────────────────────

/// Rolling per-site-key rate limiter. Coarse by design: one bucket per key,
/// pruned lazily; protects the DB, not a billing meter.
fn rate_ok(site_key: &str) -> bool {
    static BUCKETS: OnceLock<Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>> =
        OnceLock::new();
    let buckets = BUCKETS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let Ok(mut map) = buckets.lock() else {
        return true;
    };
    let now = std::time::Instant::now();
    map.retain(|_, (start, _)| now.duration_since(*start).as_secs() < 120);
    let entry = map.entry(site_key.to_string()).or_insert((now, 0));
    if now.duration_since(entry.0).as_secs() >= 60 {
        *entry = (now, 0);
    }
    entry.1 += 1;
    entry.1 <= EVENTS_PER_MINUTE
}

#[derive(Debug, Deserialize)]
struct BeaconBody {
    /// "pv" (pageview) or "ev" (custom event).
    #[serde(default)]
    k: Option<String>,
    #[serde(default)]
    p: Option<String>,
    #[serde(default)]
    r: Option<String>,
    #[serde(default)]
    n: Option<String>,
    /// Event properties (v40). Flat scalars only; sanitized server-side.
    /// Without this an event carried a NAME alone, so migrating a site off a
    /// third-party analytics tool — which the install brief mandates —
    /// silently destroyed every property on every call site.
    #[serde(default)]
    d: Option<serde_json::Value>,
    /// First-party session id from sessionStorage (v40). No cookie, not
    /// cross-site. visitor_hash rotates daily, so this is what makes bounce
    /// rate, pages-per-session and entry/exit pages computable at all.
    #[serde(default)]
    s: Option<String>,
}

fn clamp(s: Option<String>, max: usize) -> Option<String> {
    s.map(|v| v.chars().take(max).collect::<String>())
        .filter(|v| !v.is_empty())
}

/// Resolve a site key to its project id by scanning project metadata. Personal
/// scale (a handful of projects); the rate limiter fronts this.
async fn project_for_site_key(pool: &Pool<Sqlite>, site_key: &str) -> Option<String> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, metadata_json FROM projects WHERE metadata_json LIKE ?1")
            .bind(format!("%{site_key}%"))
            .fetch_all(pool)
            .await
            .ok()?;
    for (id, metadata) in rows {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&metadata) {
            if value
                .get(METADATA_KEY)
                .and_then(|c| c.get("siteKey"))
                .and_then(|k| k.as_str())
                == Some(site_key)
            {
                return Some(id);
            }
        }
    }
    None
}

async fn collect(
    State(state): State<Arc<AppState>>,
    Path(site_key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    // Shape checks before any DB work.
    if site_key.len() != 32 || !site_key.chars().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND;
    }
    if body.len() > MAX_BODY_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE;
    }
    if !rate_ok(&site_key) {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    // sendBeacon posts text/plain — parse from raw bytes, never via the JSON
    // extractor (which would 415 on the missing content type).
    let beacon: BeaconBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let kind = match beacon.k.as_deref() {
        None | Some("pv") => "pageview",
        Some("ev") => "event",
        Some(_) => return StatusCode::BAD_REQUEST,
    };

    let Ok(pool) = state.session_manager().pool_clone().await else {
        return StatusCode::INTERNAL_SERVER_ERROR;
    };
    let Some(project_id) = project_for_site_key(&pool, &site_key).await else {
        return StatusCode::NOT_FOUND;
    };

    // Privacy-preserving daily visitor hash: UA + language + UTC day + key.
    // No IP, no cookie, rotates at midnight UTC.
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let lang = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut hasher = Sha256::new();
    hasher.update(site_key.as_bytes());
    hasher.update(ua.as_bytes());
    hasher.update(lang.as_bytes());
    hasher.update(day.as_bytes());
    let visitor_hash = hex::encode(&hasher.finalize()[..16]);

    // Classify server-side, at collect time, so the raw signal is never stored:
    // the user agent decides is_bot and is then discarded, and only allowlisted
    // campaign params survive out of the query string.
    let raw_path = beacon.p.clone().unwrap_or_else(|| "/".to_string());
    let utm = classify::extract_utm(&raw_path);
    // The stored path drops the query, or every ?utm_source= variant becomes a
    // distinct "page" and the top-pages list fragments into near-duplicates.
    let path = classify::normalize_path(&raw_path);
    let referrer = clamp(beacon.r, 512);
    let name = clamp(beacon.n, 128);
    let is_bot = classify::is_bot(Some(ua)) as i64;
    let properties = beacon.d.as_ref().and_then(classify::sanitize_properties);
    let session_id = clamp(beacon.s, 64);

    let inserted = sqlx::query(
        "INSERT INTO analytics_events
           (project_id, kind, path, referrer, name, visitor_hash,
            properties, is_bot, session_id, utm_source, utm_medium, utm_campaign)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(&project_id)
    .bind(kind)
    .bind(&path)
    .bind(&referrer)
    .bind(&name)
    .bind(&visitor_hash)
    .bind(&properties)
    .bind(is_bot)
    .bind(&session_id)
    .bind(&utm.source)
    .bind(&utm.medium)
    .bind(&utm.campaign)
    .execute(&pool)
    .await;

    match inserted {
        Ok(_) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!("analytics collect insert failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ── Routers ──────────────────────────────────────────────────────────────────

/// Bearer-protected management surface (merged into the protected group).
pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/projects/{project_id}/analytics/first_party",
            get(get_setup),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/enable",
            post(enable_first_party),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/stats",
            get(first_party_stats),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/funnel",
            get(first_party_funnel),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/drain",
            post(set_drain),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/verify",
            post(verify_install),
        )
        .route(
            "/api/projects/{project_id}/analytics/first_party/rotate",
            post(rotate_drain_secret),
        )
        .with_state(state)
}

/// The beacon endpoint — mounted OUTSIDE bearer auth and the origin guard
/// (see module docs for the exposure bounds).
pub fn collect_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/collect/{site_key}", post(collect))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use serial_test::serial;
    use tower::ServiceExt;

    async fn request(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().uri(uri).method(method);
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| serde_json::json!(String::from_utf8_lossy(&bytes)))
        };
        (status, json)
    }

    /// Beacon POST with NO content-type header — exactly what sendBeacon sends.
    async fn beacon(app: &Router, uri: &str, body: serde_json::Value) -> StatusCode {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .method("POST")
                    .header("user-agent", "test-browser/1.0")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status()
    }

    async fn test_app() -> (Router, Router, Pool<Sqlite>, Arc<AppState>) {
        let state = AppState::new(true).await.unwrap();
        let pool = state.session_manager().pool_clone().await.unwrap();
        (
            routes(state.clone()),
            collect_routes(state.clone()),
            pool,
            state,
        )
    }

    async fn seed_project(pool: &Pool<Sqlite>, name: &str) -> Project {
        projects::create_project(
            pool,
            projects::CreateProject {
                name: name.to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn enable_collect_stats_roundtrip() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let (app, collect_app, pool, _state) = test_app().await;
        let project = seed_project(&pool, "fp-analytics-roundtrip").await;

        // Enable mints a key and returns the setup payload.
        let (status, setup) = request(
            &app,
            "POST",
            &format!("/api/projects/{}/analytics/first_party/enable", project.id),
            Some(serde_json::json!({ "ingestBase": "https://example.tail1234.ts.net" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let site_key = setup["siteKey"].as_str().unwrap().to_string();
        assert_eq!(site_key.len(), 32);
        assert!(setup["ingestUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://example.tail1234.ts.net/collect/"));
        // The snippet is now SAME-ORIGIN (relay mode): it carries the fixed
        // collect path, not the site key, because a public site's visitors can
        // never reach this daemon directly.
        let snippet = setup["snippet"].as_str().unwrap();
        assert!(snippet.contains(COLLECT_PATH));
        assert!(
            snippet.contains("replaceState"),
            "SPA routes must be counted"
        );
        // Hooking both WITHOUT a pathname guard double-counts every load:
        // React Router calls replaceState during mount, so the initial
        // pageview fires again from the wrapper. Measured 2x on a real
        // install — plausible-looking and permanently wrong. Hooking only
        // pushState is not the fix either; that undercounts Router
        // navigations.
        assert!(
            snippet.contains("lastPath"),
            "snippet must dedupe pageviews on pathname or every figure is ~2x high"
        );
        assert!(
            !snippet.contains("http://") && !snippet.contains("https://"),
            "the collect endpoint must stay RELATIVE — an absolute host beacons to the \
             visitor's own machine and is blocked as mixed content"
        );
        let prompt = setup["agentPrompt"].as_str().unwrap();
        assert!(prompt.contains("window.permagent.event"));
        // The install's one total-silent-failure mode. Omitting it cost ~3
        // hours on the first real install: helmet's script-src 'self' refused
        // the inline snippet, producing zero events and no error anywhere.
        assert!(
            prompt.contains("Content Security Policy") || prompt.contains("CONTENT SECURITY"),
            "brief must make the agent check CSP before injecting an inline script"
        );
        // The env var went on the Postgres service on the first real install:
        // everything looked right and the only symptom was an ambiguous 401.
        assert!(
            prompt.contains("SERVICE THAT RUNS THIS APP'S SERVER PROCESS"),
            "brief must disambiguate WHICH service gets the key"
        );
        // The brief must carry the drain contract and a real secret, or the
        // agent on the other end cannot build a working relay.
        assert!(prompt.contains(DRAIN_PATH));
        assert!(prompt.contains("x-permagent-key"));
        let secret = setup["drainSecret"].as_str().unwrap();
        assert_eq!(secret.len(), 64, "256-bit hex drain secret");
        assert!(
            prompt.contains(secret),
            "brief must embed the actual secret"
        );
        // Permagent mints the visitor salt too. The brief used to say "use a
        // fixed server-side salt", leaving the coding agent to invent one:
        // unrecorded, unrotatable, and often a literal made up on the spot.
        let salt = setup["visitorSalt"].as_str().unwrap();
        assert_eq!(salt.len(), 64, "256-bit hex visitor salt");
        assert_ne!(salt, secret, "the two credentials must be distinct");
        assert!(
            prompt.contains(&format!("PERMAGENT_ANALYTICS_SALT={salt}")),
            "brief must embed the generated salt as an env var"
        );
        assert!(
            prompt.contains(&format!("PERMAGENT_ANALYTICS_KEY={secret}")),
            "brief must embed the key as an env var"
        );
        assert_eq!(setup["receiving"], serde_json::json!(false));

        // Re-enable is idempotent: same key.
        let (_, setup2) = request(
            &app,
            "POST",
            &format!("/api/projects/{}/analytics/first_party/enable", project.id),
            None,
        )
        .await;
        assert_eq!(setup2["siteKey"].as_str().unwrap(), site_key);

        // Beacons ingest without auth and without a JSON content type.
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!({ "k": "pv", "p": "/pricing", "r": "https://news.ycombinator.com/" }),
            )
            .await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!({ "k": "ev", "p": "/pricing", "n": "signup" }),
            )
            .await,
            StatusCode::ACCEPTED
        );

        // Unknown key 404s; malformed body 400s.
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{}", "0".repeat(32)),
                serde_json::json!({ "k": "pv" }),
            )
            .await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            beacon(
                &collect_app,
                &format!("/collect/{site_key}"),
                serde_json::json!("nope")
            )
            .await,
            StatusCode::BAD_REQUEST
        );

        // Stats aggregate what was ingested.
        let (status, stats) = request(
            &app,
            "GET",
            &format!(
                "/api/projects/{}/analytics/first_party/stats?days=7",
                project.id
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["enabled"], serde_json::json!(true));
        assert_eq!(stats["receiving"], serde_json::json!(true));
        assert_eq!(stats["pageviews"], serde_json::json!(1));
        assert_eq!(stats["deviceSignatures"], serde_json::json!(1));
        // Bots are excluded by default, and the count of what was filtered is
        // surfaced so a filtered figure never reads as a quiet day.
        assert_eq!(stats["includingBots"], serde_json::json!(false));
        assert!(stats["botsExcluded"].is_number());
        assert_eq!(stats["topPages"][0]["name"], serde_json::json!("/pricing"));
        // Grouped by HOST, not raw URL: a hundred deep links from one site are
        // one source, not a hundred rows.
        assert_eq!(
            stats["topReferrers"][0]["name"],
            serde_json::json!("news.ycombinator.com")
        );
        // First-touch source/medium from the shared referrer map (no utm_*).
        assert_eq!(
            stats["topSources"][0]["name"],
            serde_json::json!("news.ycombinator.com / referral")
        );
        assert_eq!(stats["topEvents"][0]["name"], serde_json::json!("signup"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn disabled_project_reports_honestly() {
        let root = crate::test_support::test_root();
        let home = tempfile::tempdir().unwrap();
        let _guard = env_lock::lock_env([
            ("HOME", Some(home.path().to_str().unwrap())),
            ("PERMAGENT_PATH_ROOT", Some(root.to_str().unwrap())),
        ]);
        let (app, _collect_app, pool, _state) = test_app().await;
        let project = seed_project(&pool, "fp-analytics-disabled").await;

        let (status, setup) = request(
            &app,
            "GET",
            &format!("/api/projects/{}/analytics/first_party", project.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(setup["enabled"], serde_json::json!(false));
        assert_eq!(setup["siteKey"], serde_json::json!(null));

        let (status, stats) = request(
            &app,
            "GET",
            &format!("/api/projects/{}/analytics/first_party/stats", project.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["enabled"], serde_json::json!(false));
        assert_eq!(stats["receiving"], serde_json::json!(false));
        assert_eq!(stats["pageviews"], serde_json::json!(0));

        // Unknown project 404s.
        let (status, _) = request(
            &app,
            "GET",
            "/api/projects/does-not-exist/analytics/first_party",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
