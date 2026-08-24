# The Forecaster — design

**Date:** 2026-08-24 · **Status:** design, nothing built · **Spike:** `scripts/bench/forecaster_spike.py`
**Decision it implements:** Jesse — "if TimesFM runs on Mac, build a new agent called the Forecast{ed→er}" whose job is
**insights on where the market around each project is going, so no direction is missed.**

> **Naming:** Jesse wrote "the Forecasted"; read as a typo. Display name **The Forecaster**, stable roster id `forecaster`.
> **Not the Seer.** `UNLAZY_TIMESFM_DEEPAGENTS_2026-08-24.md` §3 designs a Seer over *internal* metrics and correctly
> concludes the data is too short. The Forecaster looks *outward* — at other people's numbers, the one class of series
> that is already long, already public, and already free to backfill.

## 0. Phase A verdict: TimesFM runs on this Mac

Measured 2026-08-24, arm64 / 16 GB. Full numbers in the spike header; the four that drive this design:

| | |
|---|---|
| Warm process start | **2.06 s** (0.70 s import + 1.36 s `from_pretrained`) |
| Forecast latency | **172–615 ms** single; **178 ms/series** batched 12-up |
| Peak RSS | **1.93 GB** |
| Device | **CPU.** `timesfm_2p5_torch.py:72-77` has no MPS branch; `mps.is_available()` is True and never consulted |

And the result that shapes everything below: on a clean seasonal synthetic series TimesFM beat seasonal-naive at
**MASE 0.193**; on 1024 real NVDA daily closes it **lost** to naive-last at **MASE 1.104**. Length is necessary, never
sufficient — TimesFM is a candidate method, not the method.

## 1. What "the market around a project" means

Jesse has **20 projects** in `projects` (`~/.permagent/spectral/permagent.db`). There is no `category` column — only
free-text `tags` — so categories below are inferred from tags + `site_url`, and the registry keys off tags, not a new enum.

**The join already exists.** `project_intel(project_id, kind CHECK(competitor|partner|adjacent), name, source_url)`
(schema v35, `spectral_schema.rs:2005-2030`) is *already* the per-project list of who the market is, already review-gated
by `propose_project_intel`/`dismiss_project_intel`. The Forecaster builds no second list of subjects: **it attaches
numeric series to `project_intel` rows that already exist**, plus a few project-level series. This is the single most
important structural decision here.

| Category (by tags) | Projects | 3–6 market-direction series |
|---|---|---|
| Dev-tool / infra | Permagent, Permagent Runtime, Spectral | npm + crates.io + PyPI downloads of competitor & adjacent packages; GitHub stars/forks/open-issues of competitor repos; HN mentions/week of the category term; arXiv papers/week (agents, memory-graph) |
| Consumer AI app | Plant Nanny, Reckonize, Kinrows, Grocery Savers | Wikipedia pageviews of the category article; HN + Stack Exchange mention counts; App Store top-chart rank of named competitors (snapshot-polled); GitHub stars of open-source alternatives |
| Marketplace / two-sided | Plekk, Teenity, Evntally, GetLadle | Wikipedia pageviews of the category & of the incumbent's article; HN mentions/week; public equity close for a listed incumbent where one exists (via the Financier) |
| B2B SaaS / proptech | LAUFT, Wealthie, Atlas Atlantic | Wikipedia pageviews (coworking, wealth management); HN mentions; listed-incumbent close via the Financier; Stack Exchange tag velocity where a technical category exists |
| Nonprofit / community | World Litter Run, Aidvocate, LOVE Nova Scotia, Harbourview RA, Port CLC | Wikipedia pageviews of the cause article; HN/news mention counts. **These have thin, noisy market signal — the registry should say so and refuse, not manufacture a trend.** |

### Sources — verified 2026-08, with their honesty labels

Backfill is the deciding property: a collector that only starts accumulating today is useless for six months.

| Source | Endpoint | History | Auth / limit | Status |
|---|---|---|---|---|
| npm downloads | `api.npmjs.org/downloads/range/…` | **backfills 18 mo daily** | none | official |
| crates.io downloads | `crates.io/api/v1/crates/{c}/downloads` | **backfills** (+ full db-dump) | none; self-throttle **1 req/s** + contact UA (RFC 3463) | official |
| PyPI downloads | `pypistats.org/api/…` (180 d) or BigQuery `pypi.file_downloads` (2016→) | **backfills** | none / free BQ sandbox 1 TiB-mo | official |
| Wikimedia pageviews | `wikimedia.org/api/rest_v1/metrics/pageviews/…/daily/…` | **backfills to 2015-07** | none | official |
| HN mentions | `hn.algolia.com/api/v1/search` + `numericFilters` on `created_at_i`, read `nbHits` | **backfills to 2006** | none; ~10 k/hr is a *community* figure, unverified | free, sanctioned |
| arXiv counts | `export.arxiv.org/api/query` date-filtered | **backfills** | none; ToS **1 req / 3 s** | official |
| Stack Exchange tags | `api.stackexchange.com` | **backfills** | free key → 10 k/day (300/day without) | official |
| GitHub repo counts | `GET /repos/{o}/{r}` → stars/forks/open_issues | **snapshot only — self-accumulate** | PAT → 5 000/hr | official |
| Equity close | `query1.finance.yahoo.com/v8/finance/chart` — **already in `market_data.rs:36`** | backfills years | none | **unofficial** |

**Explicitly not proposed, and why.**
- **GitHub stargazers-with-timestamps is dead.** `application/vnd.github.star+json` on `/stargazers` was restricted to
  repo admins/collaborators on **2026-06-30**; the classic "star history" backfill no longer exists for repos you do not
  own. Hence GitHub is snapshot-poll-only above.
- **Google Trends: no.** The API announced 2025-07-24 is still application-gated alpha; `pytrends` scrapes an
  undocumented internal endpoint against Google's ToS. Do not build on it.
- **Reddit: no, for now.** Free tier survives, but self-serve registration closed under the Responsible Builder Policy —
  every client goes through a manual approval queue that may say no. Not a dependency to design around.
- **App Store rank: snapshot only.** `rss.applemarketingtools.com/api/v2/…` is live and free with no history endpoint;
  App Store Connect is owner-only.
- **Stooq/Yahoo are tolerated, not sanctioned.** We use Yahoo anyway because `market_data.rs` already does — stated out
  loud rather than pretended otherwise.

**What Permagent already has.** *Exists already:* equity quotes (`market_data.rs::daily_closes`); the subject list (`project_intel`); internal
analytics (`analytics_events`, `growth_actions` — the Seer's territory, not ours); Brain memory counts.
*Needs a new collector:* every source in the table above except equity, plus the series↔intel binding row.

**A deliberate departure.** The Financier *never stores quotes* (`spectral_schema.rs:1258-1260`, "a price is only a price
at read time"). The Forecaster **must** store, because history is the product. The rule is scoped, not broken: the
Financier answers *what is it now*, the Forecaster *where is it going*, and only the second needs a past.

## 2. Pipeline

```
project_intel row + project tags → forecaster_series (registry, review-gated)
  → collectors (Rust, one per SourceKind; backfill on bind, then incremental)
  → forecaster_points (ts, value; append-only)
  → n < N ? seasonal-naive/ETS : rolling-origin backtest picks the winner
  → Forecast{ point, p10..p90, method, mase_vs_baseline }  → LLM market brief
```

**Storage** (new tables, existing SQLite pattern):
- `forecaster_series(id, project_id, intel_id NULL, source_kind, subject, cadence, label, status, created_at)`,
  `UNIQUE(project_id, source_kind, subject)`. `source_kind` is a **closed enum** (`Npm|Crates|PyPI|WikiPageviews|
  HnMentions|ArxivCount|StackExchangeTag|GithubRepo|EquityClose`) mirroring `TargetMetric`'s closed-set discipline
  (`growth/metrics.rs:74-79`); `subject` is free text (package/article/query/ticker) because every project has different
  competitors — but a series goes `active` only through the same propose/approve gate as `project_intel`.
- `forecaster_points(series_id, ts, value, PRIMARY KEY(series_id, ts))` — append-only, idempotent re-collection.
- `forecaster_forecasts(series_id, made_at, horizon, method, point_json, quantiles_json, mase_vs_baseline)`.

**Schedule.** `goose-server/src/forecaster_sweep.rs`, copying `growth_sweep.rs:35`: `STARTUP_DELAY = 600 s`, then
`TICK = 24 h` for collection. Forecasting and synthesis run **weekly** — direction does not move on a one-day scale, and
weekly keeps LLM cost at one brief per project.

**Minimum-length gate.** `N_min = 180` daily, `104` (two years) weekly. TimesFM 2.5's input patch is 32, so under ~64
points it sees two patches and nothing else; 180 daily gives ≥25 non-overlapping rolling-origin folds at H=7, the
smallest number at which §4's fold-win test has power, and 104 weekly is that argument at weekly resolution. npm
backfills only 78 weeks, so npm-only series sit below the gate for months — the UI says so rather than hiding it. Below
`N_min` the series is served by the Rust baseline and never by TimesFM.

**Serving TimesFM — one-shot, on the M1, over SSH.** *(Amended 2026-08-24 by Jesse, superseding the
"local one-shot on the hub M4" decision this section originally carried: "load the model on the M1 via SSH and the
Forecaster calls it there so we aren't putting too much on my M4.")*

§3 of the prior doc proposed a long-lived `seerd` FastAPI process. **The measurements say do not**, and that part of the
original argument stands: warm start is 2.06 s; a weekly sweep of ~20 projects × 5 series = 100 series batched at
178 ms/series ≈ 18 s. Paying 2 s once a week beats holding **1.93 GB resident** anywhere. What changed is only *which
machine pays it*.

**Where it runs.** The headless 16 GB **M1 mini**, not the hub M4. The M4 already holds the local LLMs and the
interactive session; 1.93 GB of transient CPU inference is small, but it is exactly the kind of thing that has no
business competing with the machine the user is typing on. The M1 already runs Ollama on `:11434` and a nightly
llama.cpp RPC split for the Librarian under launchd (**01:50–06:10**, ~8 GB Metal in use). TimesFM at 1.9 GB CPU sits
alongside that comfortably — but the weekly sweep is **scheduled outside that window** regardless, because "comfortably"
is a claim about averages and the split is not.

**How it is reached.** `ssh <target> <cmd>`, series in on stdin as JSON, forecasts out on stdout as JSON. Not HTTP: no
listener to secure, no port to expose, no service to keep alive, and Tailscale already carries the identity. The target
is a **config key**, never a hardcoded address — the same pattern `OLLAMA_HOST` already follows in
`~/.permagent/config.yaml`:

```
forecaster.timesfm_ssh_target   default "jesses-mac-mini-2"   # Tailscale name, stable across reboots
forecaster.timesfm_remote_dir   default "~/.permagent/forecaster"
forecaster.timesfm_enabled      default true
```
Tailscale (`100.74.232.95`) is preferred over the direct Ethernet link-local address for anything scheduled: the
link-local address changes on every reboot, so `ssh m1` is a convenience for a human at a terminal and not a dependency
a weekly job may hold.

**What is installed there.** An idempotent bootstrap script in `scripts/`, run over SSH, that creates
`~/.permagent/forecaster/venv` with a **uv-managed** CPython (per the spike: Homebrew's python3.12 returns an empty
`platform.mac_ver()` and uv refuses it outright) and installs `timesfm[torch]`, plus the one-shot
`~/.permagent/forecaster/forecast.py`. Re-running it is a no-op. **Correction to this doc's earlier draft:**
`crates/goose/src/python_runtime.rs` — with `ensure_venv`, `pip_install` and the un-timeouted `run` this section
originally cited — **does not exist in the repository**. There was nothing to reuse and nothing to avoid; the venv
bootstrap is written fresh, and it is written for the *remote* host anyway.

**Measured on the M1, 2026-08-24, over Tailscale.** The bootstrap ran clean on the first attempt: uv 0.12.5 installed a
managed CPython 3.12.13, `timesfm[torch]` (torch 2.13.0) into a **591 MB** venv, and the 882 MB checkpoint downloaded on
first use. Cold first call, download included: **2 m 12 s**. Warm, three consecutive runs of a **12-series batch
(n = 512, H = 21)**: **5.69 s / 5.59 s / 5.40 s** end-to-end wall clock — **≈460 ms per series**, and that number
*includes* SSH setup, the 2 s process-and-model warm start, and 122 KB of JSON each way. The spike's 178 ms/series was
pure compute with the model already resident; the difference is the fixed cost, which amortises further across a real
100-series sweep. The 120 s timeout therefore carries roughly 20× headroom. Quantile columns verified empirically:
`p10 <= point <= p90` held on every step of every series, confirming that the continuous head's 10 columns are
`[mean, q0.1 … q0.9]` and that indices 1 and 9 are the right ones to read.

**One caution worth writing down: the M1's data volume is at 96%, 17 GiB free.** TimesFM's ~1.5 GB fits and the
bootstrap refuses below 4 GiB — but this is not a machine with room to spare, and the next thing installed there should
check first.

**The invocation owns its own timeout.** The Forecaster spawns `ssh`, writes the batch, and waits under
`tokio::time::timeout(Duration::from_secs(120), …)` — 6× headroom over the measured 20 s of compute, with the rest
absorbing SSH setup over Tailscale.
```rust
pub async fn forecast_batch(
    cfg: &RemoteConfig, reqs: &[SeriesRequest],   // {series_id, values: Vec<f32>, horizon}
) -> Result<Vec<Forecast>, ForecastError>         // Unreachable | Timeout | Malformed | NotInstalled
```
On any of those four the sweep **falls back to the Rust baseline and relabels the method**. It never emits a forecast
whose `method` does not match what produced it, and it **never silently runs TimesFM on the M4** — an unreachable M1 is
a degraded state that says so, not a reason to move the load onto the machine this decision exists to protect. That is
the `picker.rs:1-20` discipline: own the client, not the process, and distinguish "unreachable" from "reachable and has
nothing".

**Health is a surface, not a log line.** A `forecaster_health` tool and route report M1 reachability, venv presence and
model presence; when degraded, the Market card's method label says so, so a week of baseline-only forecasts cannot be
mistaken for a week in which the model agreed with the baseline.

**Refusal state.** `Forecast.method` is a non-optional enum, as `SpendForecast.method` is a mandatory label
(`finance_ledger.rs:618`, literally *"this is a trailing average, not a model"*). Add `Refused { reason:
InsufficientHistory | CollectorStale | NoMethodBeatsBaseline }` so an unforecastable series renders the reason, not an
empty chart — `growth/power.rs::judge`'s guard order applied to a new domain.

**LLM synthesis.** One brief per project per week: forecast table (~12 rows), the last 3 briefs' verdicts, and recent
project memories in; a short "market direction" paragraph out. ≈1.5 k in / 300 out — small enough for the on-device
Apple Foundation Models path per the Apple Intelligence directive, with `cost_router` as fallback for long intel lists.
The prompt's hard rule, enforced by a test the way `power.rs:822` forbids "caus*": **the brief may only restate
direction, magnitude, interval and method — no causal claim, no recommendation, no number absent from the input.**

## 3. Agent registration

**Roster** — `world/agents/roster.ts`, following the Financier entry at :121-136:
`{ id: 'forecaster', name: 'The Forecaster', role: 'agent', trimColor: AGENT_TRIM.forecaster, isHenry: false,`
`  mezzanineLocked: false, home: { x: 8.2, y: 0, z: 5.0 }, weathering: 0.15 }`
New `AGENT_TRIM.forecaster` in `world/shared/palette.ts:33-58`: propose **`#8E7CC3`** — the violet band is unoccupied and
not confusable with `watcher #9FB8D8` or `financier #C4A35A`. **`henry` stays `henry`** — a stable id key whose display
name is "Aria"; nothing here renames an existing id.

**Extension** — `platform_extensions/forecaster.rs` on the `finance.rs` template: `EXTENSION_NAME = "forecaster"` (:47),
`pub struct ForecasterClient { info, context }` (:193), `new(ctx)` building `InitializeResult` with `.with_instructions(…)`
(:207), `get_tools()`, `impl McpClientTrait` with `list_tools` (:1007) and a `call_tool` match (:1020-1054) bracketed by
`announce("working")`/`announce("available")` → `events::agent_state_changed("forecaster", "The Forecaster", state)`
(:198). Registered in `PLATFORM_EXTENSIONS` (`platform_extensions/mod.rs:127`; finance's block at :728-775 is the shape).
**Tools (four).**
```rust
// the honesty surface: makes §1's table queryable instead of aspirational
forecaster_series(project: Option<String>)
  -> [{ series_id, project, source_kind, subject, cadence, points, span_days,
        verdict: Forecastable | InsufficientHistory | CollectorStale | NotBound }]
// proposes only; approval is the project_intel gate, never the model's call
forecaster_bind(project: String, source_kind: SourceKind, subject: String, intel_id: Option<String>)
  -> { series_id, status: "proposed", backfill_available_points }
// refuses below N_min rather than returning a number
forecaster_forecast(series_id: String, horizon: u32)
  -> { point: [f64], p10: [f64], p90: [f64], method, mase_vs_baseline, made_at }
// the weekly synthesis, on demand
forecaster_brief(project: String)
  -> { direction_summary, per_series: [...], generated_at, method_mix }
```

**Self-knowledge** — a `SELF_KNOWLEDGE_FEATURE: FeatureDescriptor` mirroring `finance.rs:1072-1104` (`category: Worker`,
`state_source: Queryable`, `teaching: &[TeachingStep{…}]`) so the main agent knows the ability exists and how to reach it.


**One UI home.** A **Market** card in `projects/ProjectDetails.tsx`, directly beneath the Intelligence (Ecosystem) panel
that reads `GET /api/projects/:id/intel` — the series hang off those exact rows, so the two are one concept and belong
adjacent. Not a new tab, not a Dashboard copy, not a world-view surface. Each row: sparkline, direction, interval, and
the `method` label always visible; an `InsufficientHistory` series renders "N of 180 — baseline only", never a blank chart.

## 4. Backtest gate

TimesFM earns `method: "timesfm-2.5-200m"` for a series **only** by passing both tests below; otherwise the series is
served by, and labelled as, the baseline.

**Metric.** Rolling-origin **MASE**, seasonal-naive denominator scaled in-sample (Hyndman):
`MASE = mean(|y_t − ŷ_t|) / mean_train(|y_t − y_{t−m}|)`, `m = 7` daily / `52` weekly / `1` if no seasonality is
registered. Origins roll forward by `H`; **≥8 folds** required — what `N_min` was sized to guarantee.

**Gate (both must hold).** (1) `median_folds(MASE_timesfm) ≤ 0.90 × median_folds(MASE_seasonal_naive)` — a 10% margin,
not a tie-break. (2) TimesFM wins **≥6 of 8** folds — a sign test, so one lucky fold cannot promote a method.

Re-evaluated every weekly sweep; a method that stops winning is demoted that week. The spike's NVDA result (MASE 1.104)
is the worked example of the gate firing correctly — and a reminder that for equity closes the honest answer is usually
"random walk," which the baseline already says.

## 5. Cost

| Line | Cost |
|---|---|
| Collector API calls | **$0.** Every source in §1 is free at our volume; the only key is a free Stack Exchange one. Politeness limits (crates.io 1 req/s, arXiv 1 req/3 s) shape the sweep's pacing, not its price. |
| TimesFM inference | **$0** — Apache-2.0 weights and code, CPU **on the M1 mini** over SSH. ~20 s wall-clock/week plus SSH setup, 1.93 GB transient there, released at process exit. Nothing resident on the hub M4. |
| LLM synthesis | ~1.5 k in / 300 out per project per week → ~36 k in / 6 k out weekly for all 20. **$0 on the on-device Apple Foundation Models path**; a cloud fallback is a small-model workload the `cost_router` prices — quote the router's number, do not hardcode one here. |
| Storage | 20 projects × ~5 series × 365 daily points ≈ 36 k rows/yr ≈ **1.5 MB/yr**. |
| Disk, one-time | 882 MB weights + 600 MB venv ≈ **1.5 GB** — on the M1, not the M4. Check `df -h` there before bootstrapping. |

## 6. Implementation, three slices

**Slice 1 — collectors and the registry. No forecasting at all.** Tables; the closed `SourceKind` enum; collectors for
the four best backfillers (npm, crates.io, Wikimedia, HN Algolia); `forecaster_sweep.rs`; `forecaster_series` and
`forecaster_bind`.
*Acceptance:* binding a series backfills its full available history in one pass and reports the real point count; a re-run
inserts zero duplicate rows; a source with no history reports `snapshot_only`, never a fabricated series.
*Tests:* `binding_a_series_backfills_before_it_starts_accumulating`, `recollecting_the_same_window_is_idempotent`,
`a_snapshot_only_source_reports_snapshot_not_a_series`, `an_unknown_source_kind_is_rejected_before_it_becomes_a_url`.

**Slice 2 — baseline forecasting, the backtest, the Market card. Ships useful with no Python.**
`forecaster/baseline.rs` (seasonal-naive + Holt-Winters ETS, pure, DB-free); rolling-origin MASE harness; the `N_min`
gate and `Refused` states; `forecaster_forecast`; the Market card.
*Acceptance:* a series under `N_min` returns `Refused{InsufficientHistory}` and the card renders the reason, not an empty
chart; every forecast carries a `method` matching what produced it; the backtest reports MASE on ≥3 real series.
*Tests:* `a_series_under_the_minimum_is_refused_rather_than_forecast`, `seasonal_naive_reproduces_a_hand_computed_forecast`,
`the_method_label_matches_the_method_that_ran`, `fewer_than_eight_folds_yields_no_verdict_not_a_weak_one`.

**Slice 3 — TimesFM on the M1 behind the gate, plus synthesis and registration.** The remote bootstrap script and
`forecast.py`; `forecast_batch` as a one-shot `ssh` spawn under its own timeout; `forecaster_health`; the two-part
promotion gate wired to the remote method; roster + palette + self-knowledge; `forecaster_brief` synthesis on the
on-device Apple Foundation Models path with the cheap cloud route as fallback.
*Acceptance:* TimesFM is selected only where it clears both gate conditions; an unreachable or hung M1 degrades to the
baseline with a relabelled method and no user-facing error, and never moves the load onto the M4; the brief contains no
number absent from its input.
*Tests:* `timesfm_that_loses_the_backtest_is_not_selected`, `a_hung_remote_call_times_out_and_falls_back_to_the_baseline`,
`an_unreachable_host_falls_back_rather_than_running_locally`, `a_forecast_from_the_fallback_is_not_labelled_as_the_model`,
`the_brief_states_no_causal_claim`. The SSH transport is tested against a fake `ssh` shim on `PATH`; no live network.

## 7. Open questions

1. **Who approves a bound series?** Reuse `project_intel`'s propose/dismiss gate exactly, or let the Forecaster
   self-bind on `kind='competitor'` rows a human already approved? (Leaning: self-bind on approved intel, propose otherwise.)
2. **Weekly or daily cadence for collection?** Daily costs nothing and makes `N_min = 180` reachable in six months for
   snapshot-only sources; weekly halves the row count and matches how fast the answer actually changes.
3. **Five nonprofit/community projects have thin market signal.** Register a permanent `Refused{NoUsableSource}` and be
   honest, or leave them off the Market card entirely?
4. **Does `subject` need normalization?** "langchain" as npm package, PyPI package, wiki article and HN query is four
   strings for one subject. One `subject_group` per competitor, or accept the duplication?
5. **Do we deepen the Yahoo dependency?** `market_data.rs` already uses an unofficial endpoint for read-time quotes;
   persisting closes makes us durably dependent on it. Stooq is marginally safer but also undocumented — neither is sanctioned.
