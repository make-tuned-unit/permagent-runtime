# Crash-report upload destination — evaluation & recommendation (#327)

**Status:** DESIGN — decision-ready. Doc-only; no code changed. The issue stays open until Jesse rules.
**Date:** 2026-07-22
**Scope:** Decide *where* (if anywhere) captured crash reports should be uploaded, judged against Permagent's local-first / sovereignty thesis. This is a destination + posture decision, not a vendor integration.

---

## 1. What exists today, and what the gap is

Crash **capture** already shipped (#299). Crash **upload** does not exist — that is precisely this issue. The module header says so in as many words:

> `crates/goose/src/session/crash_capture.rs:12-13` — *"Out of scope (→ #327): uploading crash reports anywhere. This lane captures locally and bundles-on-consent; there is no network path."*

### 1.1 What is captured, and where it lives

- A global panic hook is installed on daemon/CLI start and writes a human-readable report on every panic — `crash_capture.rs:258` (`install_panic_hook`), hook body `crash_capture.rs:269-299`.
- The report is a fixed, structured text file: timestamp, thread, location (`file:line:col`), panic message, full backtrace — struct `CrashReport` `crash_capture.rs:87-93`, rendered by `to_text()` `crash_capture.rs:96-107`.
- Reports are written to **`<state_dir>/crashes/crash-<ts>-<pid>.log`** on the user's own disk — `crash_dir()` `crash_capture.rs:43-45`, `record_crash()` `crash_capture.rs:123-129`.
- The store self-prunes to the **20 most recent** files — `MAX_CRASH_FILES` `crash_capture.rs:24`, `prune()` `crash_capture.rs:132-156`.
- A panic **circuit-breaker** (unrelated to upload) forces a clean `exit(1)` on a panic cluster so launchd relaunches — `crash_capture.rs:240-296`. Noted only so it is not confused with an upload path; it stays untouched here.

Capture is always on: it is just a log file on local disk. Nothing about capture is a sovereignty concern.

### 1.2 The consent gate that already exists

- Whether a crash report may ever be **shared** is gated by `crash_reports_consented()` `crash_capture.rs:53-62`, which delegates to `crate::posthog::is_telemetry_enabled()`.
- That opt-in is **default-OFF and explicit**: `is_telemetry_enabled()` returns `get_telemetry_choice().unwrap_or(false)` — false until the user makes a choice — `posthog.rs:47-49` / `posthog.rs:32-39`. An env kill-switch `GOOSE_TELEMETRY_OFF` forces it off — `posthog.rs:22-27`. Config key `GOOSE_TELEMETRY_ENABLED` (`TELEMETRY_ENABLED_KEY`, `posthog.rs:20`).
- Without the `telemetry` feature compiled in, consent is denied by construction — `crash_capture.rs:58-61`.

### 1.3 Redaction machinery that already exists

`posthog.rs` already carries a scrubber that any upload path would reuse:

- `sanitize_string()` `posthog.rs` — runs a list of regexes (`SENSITIVE_PATTERNS`) that replace `/Users/<name>`, `/home/<name>`, `C:\Users\<name>`, `sk-`/`pk-` keys, `key…`/`token…`/`bearer …` secrets, emails, `user:pass@` URLs, and UUIDs with `[REDACTED]`.
- `sanitize_value()` recurses that over JSON, and `emit_event()` additionally drops any property whose key contains `key/token/secret/password/credential`.

### 1.4 The precise gap (three claims corrected against code)

1. **There is no network path for crash reports.** No production code sends a crash report anywhere. Confirmed: `posthog_capture()` exists but only `emit_event()` calls it, and `emit_event()` early-returns for everything except `onboarding_*` / `telemetry_preference_set` events (`posthog.rs` generic-event branch). The `app_crashed` / `error_occurred` handling inside it is unreachable today.
2. **The "consent-gated diagnostic zip bundle" is not wired.** `collect_crash_logs()` `crash_capture.rs:160-188` exists to hand `(filename, bytes)` pairs to a bundler, but a repo-wide search shows its only consumer is the evidence test (`crates/goose/tests/crash_capture_evidence.rs:21`). No production diagnostic-bundle path consumes crash logs yet. (An earlier issue comment citing `diagnostics.rs:216-222` overstates what is on main.)
3. **The consent UI is dead.** `ui/command-center/src/components/settings/SettingsView.tsx:824,835` renders a "Share anonymous diagnostics — *Crash reports and timing. Never your prompts.*" toggle, but it is `useState(true)`-only (defaults ON in the widget, contradicting the default-OFF backend) and never writes `GOOSE_TELEMETRY_ENABLED`.

**So "upload" would add:** (a) a transport that moves a captured report off the machine, (b) a redaction step on that path, (c) a real consent surface, and (d) a decision about *where the bytes land* — the subject of this doc.

---

## 2. Why this is a sovereignty decision, not just a vendor pick

A crash backtrace is not neutral telemetry. Panic messages and backtraces routinely embed **prompt fragments, file paths, argument values, and user data** — exactly the material Permagent promises stays local. The product's sharpest sovereignty claim is physical: *"pull the ethernet cable, do real work, watch zero outbound"* ([[sovereignty-architecture-direction]]). An ambient crash uploader is a standing exception to that claim.

Two facts make this concrete:

- **The sovereignty router does NOT cover this egress.** `SovereignGuardProvider` (#765) wraps only `Arc<dyn Provider>` inference calls at the two factory closures; it refuses cloud *inference* under sovereign mode and writes an append-only `egress_audit` record for every cloud call (`crates/goose/src/sovereignty/mod.rs`, `providers/sovereign_guard.rs`). A crash upload is an HTTP POST from a completely different code path — it would be **egress the sovereignty story does not currently see or log.** For a sovereign-mode user, an unlogged crash POST is a direct contradiction of the guarantee the guard exists to make.
- **Redaction is mitigation, not physics.** `sanitize_string()` is a regex allowlist; it catches known shapes (home paths, key prefixes, UUIDs, emails) but cannot prove a backtrace is free of user content. It reduces, it does not eliminate. Any destination must be judged assuming *some* residual sensitive text can slip through.

The bar this doc holds every option to: **does it default to no ambient outbound, make each egress explicit and logged, and keep the collector self-hostable** — so the sovereignty claim survives the feature.

---

## 3. Destination options

Each option assumes redaction via the existing `sanitize_string()` is applied before any byte leaves; that is table stakes, not a differentiator between options.

### (a) Sentry SaaS (`sentry.io`)
- **Privacy / egress:** Reports leave to Sentry's cloud (US or EU region). Best-in-class crash triage: grouping, release-health, symbolication. But it is third-party ambient egress of backtraces — the thing sovereign mode forbids.
- **Effort:** New SDK in up to three surfaces (daemon Rust `sentry` crate, CLI, eventually the webview), a new DSN secret to ship in the binary, a new account to run. Moderate.
- **Cost:** Free developer tier exists; scales with event volume.
- **Offline:** SDK buffers and retries; fine.
- **Sovereignty fit:** Poor as a default. Adds a US/EU SaaS egress the router can't see. Acceptable only behind an explicit opt-in with the send logged to `egress_audit`, and only for non-sovereign contexts.

### (b) Self-hosted Sentry-compatible collector — **GlitchTip** (or self-hosted Sentry)
- **Privacy / egress:** Crash data lands on infrastructure *Jesse* controls, not a vendor. GlitchTip speaks the Sentry wire protocol, so the client is protocol-standard and can move self-hosted ↔ SaaS without a rewrite. Full triage/grouping like Sentry.
- **Effort:** Same client work as (a), plus standing up and maintaining a collector. GlitchTip is materially lighter than self-hosted Sentry or self-hosted PostHog (Django + Postgres + a worker; no ClickHouse/Kafka — contrast the PostHog stack flagged in [[grow-image-analytics-feasibility]]). Still real ops for a solo maintainer, and — like the analytics collector problem — a LAN-only host can't receive internet POSTs without a tunnel/VPS.
- **Cost:** A small always-on host (VPS, or the mini + a tunnel).
- **Offline:** Same buffering story as (a).
- **Sovereignty fit:** **Best of the "real pipeline" options.** Self-hostable = the collector can be operator-controlled; protocol-standard = not locked to one vendor; still requires the same opt-in + `egress_audit` logging + sovereign-mode suppression as any network path.

### (c) S3 bucket (raw upload)
- **Privacy / egress:** Raw `crash-*.log` files (or JSON) uploaded to an AWS bucket. Egress to AWS.
- **Effort:** Small to write (signed PUT), but **you build the entire read side** — no grouping, no triage, no dedup, no search. A pile of logs in a bucket is not a debugging tool.
- **Cost:** Trivial storage cost.
- **Offline:** Trivial retry.
- **Sovereignty fit:** No better than SaaS on egress (data still leaves to a cloud), and worse on product value. Rejected: it trades all triage value for marginal simplicity and gains nothing on the sovereignty axis.

### (d) PostHog (`us.i.posthog.com`) — the already-wired path
- **Privacy / egress:** The client already exists — `POSTHOG_CAPTURE_URL = https://us.i.posthog.com/capture/` `posthog.rs:17`, keyed by a UUID `installation_id`. PostHog's native `$exception` Error Tracking gives grouping for free. **But the endpoint is a US SaaS**, and reusing it for crashes means backtraces flow to a US analytics vendor by default.
- **Effort:** Lowest — zero new dependency, zero new secret, zero new account. Consent already routes through `is_telemetry_enabled()`. This is the entire pragmatic case, and it is a real one.
- **Cost:** Free tier; scales with volume.
- **Offline:** Fire-and-forget POST; lost on failure unless a drain/retry is added.
- **Sovereignty fit:** **Two problems beyond the generic egress concern.** (1) It deepens a dependency that is *itself unratified*: [[grow-image-analytics-feasibility]] records an open provider-canon conflict — GrowView's written plan says self-hosted PostHog, but the analytics feasibility recommends Plausible/GoatCounter, and CE's Stats API v2 is cloud-only. Wiring crashes onto PostHog picks a side of an unsettled ruling by inertia. (2) US-SaaS-by-default is the weakest posture for the "pull the cable" pitch. It is the cheapest option and the least consistent with the thesis.

### (e) Opt-in-only / no ambient upload (local-only + user-triggered "send report")
- **Privacy / egress:** **Zero ambient outbound.** Capture stays 100% local (already built). The user, on their own initiative, triggers a send of a specific report they can inspect first. Egress is user-driven and per-report, not a background process. This is the only option that stays true under "pull the ethernet cable."
- **Effort:** Smallest end-to-end that ships value: wire `collect_crash_logs()` into an exportable, redacted bundle behind a "Report a problem" action, showing the user the exact redacted payload before it moves. No collector required if the user attaches the file to their own support email / GitHub issue (out-of-band egress, fully user-controlled). Reuses the existing capture + redaction; the missing piece is a real consent/preview surface.
- **Cost:** Zero infra.
- **Offline:** N/A — nothing is sent without an explicit user action.
- **Sovereignty fit:** **Highest.** Explicit consent per report, user sees what leaves, no code path emits without a click, and it can still be logged to `egress_audit` for a complete outbound ledger. Its limit: lower crash-collection volume than an ambient pipeline (users must choose to send), and no automatic grouping/triage until it lands somewhere structured.

---

## 4. Recommendation

**Ship explicit-consent, redacted, user-triggered reporting first (option e); defer any ambient pipeline, and if/when one is built, target a self-hostable Sentry-compatible collector (option b, GlitchTip) — not PostHog.**

This is the option set the thesis points at: **opt-in, redacted, self-hostable.** Rationale:

- **It preserves the load-bearing claim.** No background code path emits a crash report. A sovereign-mode user, and any user who "pulls the cable," sees zero outbound — including from crashes. That is worth more to the aspirin ICP (regulated / IP-sensitive professionals, [[sovereignty-architecture-direction]]) than the marginal debugging convenience of ambient collection.
- **It ships now with near-zero new surface.** Capture and redaction already exist; the work is a consent/preview UI plus wiring `collect_crash_logs()` into an exportable redacted bundle. No new dependency, no new secret, no new infra, no unratified-vendor commitment.
- **It keeps the destination decision open and reversible.** Starting local-only does not foreclose an ambient pipeline later; it means the destination is chosen *after* seeing whether user-driven reports supply enough signal.
- **When an ambient pipeline is justified, GlitchTip beats PostHog and S3.** Self-hostable (operator-controlled), protocol-standard (self-hosted ↔ Sentry SaaS with no client rewrite), and it comes with real triage — where S3 gives none and PostHog forces a US-SaaS default plus deepens an unratified analytics dependency.

### Phased path

- **Phase 0 — done (#299):** local capture + default-OFF consent gate + redaction machinery.
- **Phase 1 — MVP, recommended now:** *No Permagent-initiated network upload.* Add a "Report a problem / Send crash report" action that (1) surfaces a captured report, (2) runs it through `sanitize_string()`, (3) **shows the user the exact redacted payload**, (4) on explicit click exports a bundle the user attaches to their own support channel — or POSTs it once, logged to `egress_audit`. Fix the dead consent toggle (`SettingsView.tsx:835`) to write/read `GOOSE_TELEMETRY_ENABLED` and default OFF to match the backend. Fully sovereign, zero infra.
- **Phase 2 — opt-in ambient drain, only if volume justifies it:** a separate, explicit "automatically send crash reports" opt-in enabling a deferred-drain on daemon start (enumerate `<state>/crashes/*.log` → redact → send → delete on success). Destination = **self-hostable GlitchTip** (Sentry-compatible; SaaS Sentry as a zero-ops fallback). Every send written to `egress_audit`; **hard-suppressed under global sovereign mode.** Prefer routing frontend crashes through the daemon so there is one consent source and one redactor.
- **Deferred past Phase 2:** Tauri shell/webview crash capture; auto-upload of the full diagnostic zip (keep manual); release-health alerting.

---

## 5. Open decisions for Jesse

1. **MVP shape.** Ship local-only + user-triggered export first (recommended), or go straight to an automated opt-in drain? *(Recommendation: local-only first.)*
2. **Ambient-pipeline destination, when built.** GlitchTip self-hosted (recommended — self-hostable, protocol-standard, triage) vs Sentry SaaS (zero-ops) vs PostHog (already wired, but US-SaaS-default and deepens an unratified dependency) vs S3 (rejected here). **This intersects the unresolved analytics provider-canon ruling in [[grow-image-analytics-feasibility]] — resolve them together so crash + analytics don't split across vendors by accident.**
3. **Sovereign-mode behavior.** Must a crash upload be hard-suppressed under global sovereign mode and written to `egress_audit` even when the user has consented to crash sharing? *(Recommendation: yes — sovereign mode = zero ambient outbound, crash uploads included; a user-initiated manual export is still allowed because the user is the actor.)*
4. **Consent granularity.** Today `crash_reports_consented()` piggybacks on the single telemetry opt-in (`is_telemetry_enabled()`). Should crash-report sharing be a **separate** toggle from product analytics, so a user can keep analytics off but help fix crashes (or vice versa)? *(Recommendation: split them — the "Never your prompts" crash toggle and a product-analytics toggle are different consent asks.)*

---

*Evidence anchors: `crates/goose/src/session/crash_capture.rs`, `crates/goose/src/posthog.rs`, `crates/goose/src/sovereignty/mod.rs`, `crates/goose/src/providers/sovereign_guard.rs`, `ui/command-center/src/components/settings/SettingsView.tsx`, `crates/goose/tests/crash_capture_evidence.rs`.*
