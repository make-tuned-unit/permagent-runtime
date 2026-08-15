# SOC 2 scoping + codebase gap analysis

> **Status: DESIGN / RESEARCH — for review. Doc-only; no code changed.**
> Scopes SOC 2 for Permagent and maps the standard to *this* codebase. Every
> "current behaviour" claim cites `file:line`; claims are marked **[VERIFIED]**
> (read in code on `design/soc2-scoping` off `origin/main` @ `ec94be5a`),
> **[DESIGN]** (specced but not implemented), or **[ASSUMED]** (inference, not
> confirmed in code).
>
> **Audit pins:** repo `origin/main` @ `ec94be5a`. SOC 2 standard details are
> grounded in external sources (AICPA TSC 2017, rev. 2022; Vanta/Drata/Secureframe
> practitioner guides — cited in §1). The *application* to Permagent is grounded
> in the code.

---

## 0. TL;DR — the one insight

SOC 2 is a compliance framework for **service organizations that hold and
process their customers' data in the service organization's own systems.** It
audits the controls *you* operate over *data that isn't yours.*

**Permagent is local-first.** The desktop client holds the **user's own data on
the user's own machine.** Permagent is not the custodian of that data — the user
is. So the desktop client is **largely out of SOC 2 scope**: there is no
service-operated system holding customer data to audit.

SOC 2 applies to the **gated cloud/team services** Permagent will operate under
the open-core strategy — Mesh (pooled compute), the federation relay, enterprise/
team multi-tenant hosting, managed team-sync, hosted inference, the marketplace,
and the small set of cloud endpoints that exist *today* (the PostHog analytics
beacon, the download/release infra, and any future crash-report upload).

**The consequence is a competitive advantage, not a burden:** local-first makes
Permagent's SOC 2 scope *smaller* and its data-protection story *stronger* than a
typical SaaS. Most of the sensitive processing that a cloud agent platform must
bring into audit scope — user prompts, memory, working context — **never enters a
Permagent-operated system at all.** We can say that with evidence (§3), because
the data boundary is enforced in code at a single fail-closed choke point.

---

## 1. What SOC 2 is

SOC 2 (System and Organization Controls 2) is an attestation report produced by a
licensed CPA firm against the AICPA's **Trust Services Criteria (TSC)**. It is not
a certification you "pass/fail" like ISO — it is an auditor's opinion on whether a
service organization's controls meet the criteria in scope.

### 1.1 The five Trust Services Criteria (TSC categories)

| Category | Required? | What it covers |
|---|---|---|
| **Security** (a.k.a. Common Criteria) | **Yes — always** | Protection of systems/data against unauthorized access, disclosure, and damage. The floor of every SOC 2. |
| **Availability** | Optional | The system is available for operation and use as committed (uptime, SLAs, DR). |
| **Processing Integrity** | Optional | Processing is complete, valid, accurate, timely, authorized. |
| **Confidentiality** | Optional | Information designated confidential is protected (beyond just personal data). |
| **Privacy** | Optional | Personal information is collected, used, retained, disclosed, and disposed of per the entity's privacy notice + criteria (maps to GDPR/CCPA themes). |

Only **Security is mandatory.** You add the others when customer commitments or
the nature of the service demand them.

### 1.2 The Common Criteria (CC-series)

The Security category is delivered through the **Common Criteria, CC1–CC9** — so
called because they underpin all the other categories:

- **CC1** — Control environment (governance, org structure, integrity/ethics) → maps to COSO.
- **CC2** — Communication and information.
- **CC3** — Risk assessment.
- **CC4** — Monitoring activities.
- **CC5** — Control activities.
- **CC6** — **Logical & physical access controls** (authn/authz, encryption, key management, provisioning/deprovisioning). *The heaviest engineering lift.*
- **CC7** — **System operations** (monitoring, detection, incident response).
- **CC8** — **Change management** (SDLC, code review, CI/CD, approvals).
- **CC9** — Risk mitigation (vendor/subprocessor management, business disruption).

When you add Availability / Confidentiality / Processing Integrity / Privacy, each
brings its own supplemental criteria (the **A-, C-, PI-, P-** series) layered on
top of the Common Criteria.

### 1.3 Type I vs Type II

- **Type I** — attests that controls are **suitably designed** at a single
  **point in time.** Faster, cheaper, a snapshot. Good as a first milestone / to
  show intent.
- **Type II** — attests that controls **operated effectively over a period**
  (typically **6–12 months**). The auditor samples evidence across the window.
  This is what enterprise buyers actually ask for.

The normal path: **Type I first** (prove design), then a **Type II observation
window** (prove it runs). You cannot shortcut to Type II — it requires a history
of the controls operating.

*Sources:* AICPA TSC; [CSA — 5 TSC explained](https://cloudsecurityalliance.org/blog/2023/10/05/the-5-soc-2-trust-services-criteria-explained), [Vanta — SOC 2 TSC](https://www.vanta.com/collection/soc-2/soc-2-trust-service-criteria), [Drata — TSC](https://drata.com/learn/soc-2/trust-services-criteria), [Secureframe — Type 1 vs Type 2](https://secureframe.com/hub/soc-2/type-1-vs-type-2).

---

## 2. Scope boundary for Permagent

The load-bearing question for any SOC 2 is: **which systems process customer data
under our control?** That set is the scope. Everything else is explicitly *out.*

### 2.1 OUT of scope — the local-first client

The desktop app + local daemon (`goose-server`) + local Brain (Spectral SQLite):

- Runs **on the user's own machine.** The user is the data custodian, not us.
- The user's prompts, memory, files, and working context live in the user's home
  directory (`Paths::data_dir()` / `state_dir()`), under the user's OS account.
- We operate **no server** that holds this data. There is nothing for an auditor
  to inspect on "our" side, because there is no "our side" for local data.

This is the same reason a desktop text editor or a local database engine is not a
SOC 2 subject: shipping software that runs on someone else's computer does not
make you a service organization *for the data on that computer.* (Our *own*
corporate IT — email, source control, laptops — is in scope for CC1/CC6/CC7 as an
organization, but that is org-level hygiene, not product scope.)

**Caveat — the boundary is where data leaves the machine.** The client stays out
of scope *only while it stays local.* The moment the client sends data to a
Permagent-operated service (mesh peer, relay, hosted inference), that egress and
the receiving service are **in** scope. Permagent already instruments exactly this
boundary in code (§3.2) — which is what lets us draw the line honestly.

### 2.2 IN scope — the gated cloud/team services (open-core)

Per the ratified open-core strategy, the paid/gated tier is a set of **cloud
services** Permagent operates. Each is a SOC 2 subject because it holds or
processes data on infrastructure we control:

| In-scope service | Status today | Why it's in scope |
|---|---|---|
| **Federation relay + team multi-tenant hosting** | **[DESIGN]** — `docs/design/federation-security-spec.md`; identity layer partially built (§3.4) | Relays/stores E2E-encrypted memory packs between hubs; multi-tenant → tenant isolation, access control, availability all auditable. |
| **Managed team-sync** | **[DESIGN]** | Operates the sync fabric + realm-key distribution for teams; processing integrity + confidentiality of shared memory. |
| **Mesh (pooled compute)** | **[DESIGN/vision]** — refuted for volunteer split-inference (activation-inversion); trusted-fleet LAN pool is the near-term shape (see MEMORY: mesh-cheap-tier-reality) | If/when Permagent operates a compute-brokering service, it routes user work to third-party nodes → confidentiality + processing integrity. |
| **Hosted inference** | **[DESIGN/vision]** | Runs user prompts on our infra → the highest-sensitivity in-scope processing (Security + Confidentiality + Privacy). |
| **Marketplace** | **[DESIGN/vision]** | Distributes recipes/extensions; supply-chain integrity, account data, payments (payments likely push toward SOC 1 / PCI too). |
| **PostHog analytics beacon** | **[VERIFIED]** — `crates/goose/src/posthog.rs` | *Exists today.* Opt-in product analytics egress to a third party → Privacy + vendor/subprocessor management. |
| **Download / release infra** | **[VERIFIED]** — `.github/workflows/release.yml` | Distributes the signed binary users run → supply-chain / integrity (CC8 + Processing Integrity). |
| **Crash-report upload** | **[DESIGN]** — `docs/design/crash-report-upload-destination.md` (no network path exists yet) | *If built,* uploads redacted crash data → Privacy + Confidentiality. |

**Tie to open-core:** the free, local, single-user product is out of scope by
construction; **the things we charge for are exactly the things that put data on
our infrastructure — and those are the SOC 2 subjects.** The scope boundary and
the monetization boundary are the same line. That alignment is the strategic
point: we only take on audit burden where we're also taking revenue.

### 2.3 The boundary, drawn

```
   USER'S MACHINE (OUT of SOC 2 scope)          PERMAGENT-OPERATED (IN scope)
  ┌───────────────────────────────────┐        ┌──────────────────────────────┐
  │ Desktop app + goose-server daemon │        │ Federation relay / team host  │
  │ Local Brain (Spectral SQLite)     │        │ Managed team-sync + keyserver │
  │ Provider API keys (OS keyring)    │        │ Mesh compute broker           │
  │ Prompts / memory / files          │        │ Hosted inference              │
  │                                   │        │ Marketplace                   │
  │  sovereignty guard  ├──egress────▶│───────▶│ (PostHog beacon — today)      │
  │  egress_audit (append-only log)   │        │ (Download/release — today)    │
  └───────────────────────────────────┘        └──────────────────────────────┘
        data is the USER'S                            data on OUR infra
        (they are the custodian)                      (we are the custodian)
                                     ▲
                          the single instrumented
                          data-boundary choke point (§3.2)
```

---

## 3. Codebase gap analysis

For the in-scope services, this maps SOC 2 requirements to what the code already
provides vs. what is missing. **Most in-scope cloud services are not built yet**,
so much of this is "controls we can carry forward from the local architecture" vs.
"controls that must be built when the service is built." That is itself the
finding: the *primitives* are unusually strong; the *service-operations* layer is
absent because the services are absent.

### 3.1 Access control / authn / authz — CC6

**What exists [VERIFIED]:**
- **Transport auth is fail-closed and constant-time.** The daemon's protected
  routes require `Authorization: Bearer <token>` validated against the master
  `daemon_token` or a per-device token — `crates/goose-server/src/middleware/auth.rs:63-106`.
  No token configured → **503, never anonymous allow-through**
  (`validate_token_value` `middleware/auth.rs:67`, tested `:185-196`). Comparison
  is constant-time via `subtle::ConstantTimeEq::ct_eq` (`middleware/auth.rs:70`),
  and the master-vs-device decision runs both checks with no early exit so timing
  doesn't leak which class matched (`:87-106`).
- **Per-device tokens with revocation.** `device_registry.rs` mints per-device
  bearer tokens, persists **only SHA-256 hashes** (`hash_token` `device_registry.rs:100-101`),
  never echoes hashes on list endpoints (`DeviceView` `device_registry.rs:54-72`),
  and supports single-device revoke without rotating the master.
- **Federation identity layer (partial).** `crates/goose-server/src/auth.rs`
  implements per-hub **Ed25519** signing + **X25519** encryption keypairs
  (`FederationIdentity` `auth.rs:219-345`), an `author_id` bound to Spectral's
  OR-Set (`auth.rs:100-108`), **TOFU peer pinning** with rollback protection
  (`PeerRegistry::pin` `auth.rs:474-510`), a **verified-wrap-target** gate so an
  unverified peer can never receive a realm key (`is_verified_wrap_target`
  `auth.rs:532-543`), and Signal-style **safety numbers** for out-of-band
  verification (`auth.rs:561-580`).

**What's missing:**
- **No multi-user / RBAC.** Today the daemon is single-user (`DEFAULT_USER_ID =
  "default"`; federation spec §0). Multi-tenant team hosting needs real user
  accounts, roles, provisioning/**deprovisioning**, and tenant isolation — none
  exist. (CC6.1–CC6.3.)
- **Bearer token is device-pairing, not per-person identity** (`auth.rs`
  module docs `:11-19`) — the federation identity work fills part of this but the
  server-side authz that consumes it is [DESIGN].
- No SSO/SCIM, no session management for a hosted console, no admin audit of
  privileged actions on a service.

### 3.2 Encryption in transit + at rest — CC6.1, CC6.7

**What exists [VERIFIED]:**
- **The sovereignty data boundary is the standout control.** Every provider is
  wrapped by `SovereignGuardProvider` at the single factory choke point
  (`providers/sovereign_guard.rs:49`); all inference egress flows through the
  guarded `stream`/`create_embeddings` (`sovereign_guard.rs:113-159`). In a
  sovereign context, cloud egress is **refused before any bytes leave**
  (`gate` `sovereign_guard.rs:59-104`) — fail-closed, the inner provider is never
  reached (test `:303-325`).
- **Secrets at rest in the OS keyring.** Provider API keys and the federation
  identity live in the platform secret store (`keyring` v3.6.2), not flat files;
  a present-but-unreadable identity is a **hard error, never silently
  regenerated** (`auth.rs:229-240`, module docs `:16-20`). Trust-state files
  (peer registry) are written atomically at **0600** via `secure_fs`
  (`auth.rs:454-467`).
- **Federation E2E encryption is designed** (encrypt + sign packs on the wire;
  relay is blind) — `federation-security-spec.md` §4/§5.

**What's missing:**
- **Local Brain (Spectral SQLite) is not encrypted at rest** — it relies on OS
  disk/account protection. Fine for a local file the user owns; **not** acceptable
  for any hosted/multi-tenant store, which needs KMS-backed at-rest encryption
  (cf. Opal: "AWS KMS at rest"). [ASSUMED — not independently verified that no
  SQLCipher layer exists; grep found none.]
- **Federation crypto is design-only** — "No crypto is implemented by this
  document" (`federation-security-spec.md` header). The relay's in-transit + at-
  rest encryption is unbuilt.
- **No TLS termination / cert management story for any operated service** (none
  exist yet). The daemon uses `rcgen` self-signed + fingerprint pinning locally
  (fed spec §0), which is a local pattern, not a public-service TLS posture.

### 3.3 Audit logging + retention — CC7.2, CC7.3

**What exists [VERIFIED]:**
- **The egress audit log is a genuine, tamper-evident audit control.** Every
  cloud call — allowed *or* blocked — is recorded in an **append-only** SQLite
  table before it proceeds (`record_egress` `sovereignty/mod.rs:441-464`; the
  guard writes the row *before* delegating, `sovereign_guard.rs:71-83`). The table
  **rejects UPDATE and DELETE at the DB layer** (triggers; proven by test
  `sovereignty/mod.rs:700-705`) — "a deletable log is a lying log."
- **No-unlogged-egress option.** With `sovereign_strict_audit` on, an allowed
  cloud call whose audit write failed is **refused** (`audit_failure_fails_call`
  `sovereignty/mod.rs:259-261`; guard `sovereign_guard.rs:96-102`). Audit-write
  failures are always logged loudly under `target: "sovereignty"`
  (`sovereignty/mod.rs:449-462`).
- **Content is hashed, not stored, by default** — only a SHA-256 content hash is
  recorded unless `sovereign_capture_prompts` is explicitly enabled
  (`sovereignty/mod.rs:41-42, 337-342`). Good data-minimization posture.
- **Non-inference egress is under the same boundary** (`guard_outbound_egress`
  in `sovereignty/mod.rs`, wired into PostHog telemetry `posthog.rs:78-86`, the
  analytics fetch and drain, and the Guard's code scans).
- **The egress log is user-visible** via `GET /api/security/egress-log`
  (`routes/security.rs:126-130`) and surfaced in the **Governance** control
  surface (`sovereignty/mod.rs:93-128`).

**What's missing:**
- This log audits **egress from the client**, which is exactly right for the
  local product — but there is **no centralized, server-side audit log for any
  operated service** (access logs, admin actions, auth events on the relay/host).
  SOC 2 CC7 wants tamper-evident, retained, monitored logs on the *service* side.
- **No log aggregation / SIEM / retention policy.** The egress table grows
  unbounded on the user's disk; a hosted service needs defined retention, secure
  storage, and alerting on the logs.
- The daemon access log deliberately strips query strings so tokens don't leak
  (`middleware/auth.rs:22-24`) — good local hygiene, but not a service audit
  trail.

### 3.4 Change management — CC8

**What exists [VERIFIED]:**
- **CI gates on every PR.** `.github/workflows/ci.yml`: `cargo fmt --check` +
  `cargo clippy -D warnings` (`ci.yml:202-204`), full workspace test matrix on
  ubuntu + macOS (`ci.yml:115-125`, `:30`), frontend build/test (`:149-151`),
  Tauri shell fmt/test (`:170-187`), all `--locked`.
- **Branch protection on `main` is real [VERIFIED via GitHub API].** Required
  status checks: `build`, `lint`, `test (macos-15)`, `test (ubuntu-latest)`,
  `frontend`, `tauri-shell`; **linear history required**, force-push and deletion
  **disabled**.
- **Dependency supply-chain gates.** `dependency-audit.yml` runs `cargo-deny`
  (advisories + yanked) on push/PR **and a daily cron** (`:36-37`), plus an npm
  audit leg; **Dependabot** is configured (`.github/dependabot.yml`).
- **Release integrity.** `release.yml` produces a **code-signed + notarized**
  macOS bundle with a **Tauri updater signature**, and hard-fails a publish if the
  updater pubkey is still the placeholder (`release.yml:100-104`).

**What's missing (this is a real, closeable gap):**
- **No required PR review.** Branch protection has `required_approving_review_count:
  0` and `require_code_owner_reviews: false` — merges to `main` need passing CI but
  **no human approval**. CC8 auditors expect an enforced review/approval step.
  (CODEOWNERS exists but only covers `/documentation/` and isn't required.)
- **`enforce_admins: false`** and **no required signed commits** (`required_signatures:
  false`) — admins can bypass protections; commit provenance isn't cryptographically
  enforced.
- **Apple signing secrets are optional** — the pipeline still builds **unsigned**
  if they're absent (`release.yml:21-22, :112`), and the release infra is
  "~90% built but never run" with a placeholder pubkey (see MEMORY:
  production-readiness-gate). For SOC 2, the signing/notarization path must be
  provably-operating, not optional.
- No formal, documented SDLC / change-management **policy** (the controls exist;
  the written policy + evidence-of-operation don't).

### 3.5 Monitoring / alerting + incident response — CC7.1, CC7.4, CC7.5

**What exists [VERIFIED]:**
- **Crash capture** writes structured local panic reports and self-prunes
  (`crash-report-upload-destination.md` §1.1) — local diagnostics only.
- The **Decision Inbox** is a human-in-the-loop approval seam: nothing destructive
  or irreversible proceeds without user sign-off (`decision_inbox/mod.rs:19-61`).
  This is a *product* control (a strong one), and it maps to CC-style authorization
  of high-risk actions — but it governs the *agent's* behavior for the user, not
  operations of a hosted service.

**What's missing:**
- **No production monitoring/alerting/on-call for any operated service** (uptime,
  error rates, security alerts) — because no service runs yet.
- **No incident-response plan / runbook / breach-notification process** (written).
- **No intrusion detection, no anomaly alerting** on a service side.

### 3.6 Vendor / subprocessor management — CC9.2

**What exists [VERIFIED]:**
- Third-party egress is *inventoried in code and gated*: PostHog is the single
  analytics endpoint (`posthog.rs:16-17`), routed through the sovereignty guard
  and the opt-in consent gate (§3.7). Redaction runs before any send
  (`posthog.rs:530-547`, `:600-611`).

**What's missing:**
- **No subprocessor register / DPA tracking.** Known/likely subprocessors already
  in play: **PostHog** (analytics), **GitHub** (source + release hosting + CI),
  **Apple** (notarization), **Tailscale** (intra-user transport), plus whatever
  IaaS the future services run on. SOC 2 CC9 + Privacy need a maintained list,
  DPAs, and periodic vendor risk review.

### 3.7 Data classification + privacy — Privacy TSC, P-series

**What exists [VERIFIED] — and it's strong:**
- **Analytics is opt-in, default OFF.** `is_telemetry_enabled()` returns false
  until the user explicitly opts in (`posthog.rs:50-52`, `:33-40`); a fresh
  install makes **zero** analytics network calls — the former onboarding-funnel
  bypass was removed (#852) and proven by an egress-audit assertion test
  (`posthog.rs:651-686`). Env kill-switch `GOOSE_TELEMETRY_OFF` (`posthog.rs:22-27`).
- **Two-layer redaction before egress** — regex scrubber (`sanitize_string` →
  `crate::privacy::redact`, `posthog.rs:530-532`) plus a key-name drop filter for
  anything containing key/token/secret/password/credential (`posthog.rs:600-611`).
- **Crash-report sharing reuses the same consent gate + redactor** and has **no
  network path at all today** (`crash-report-upload-destination.md` §1.2–1.4).
- **Sovereign offboarding** is designed as a clean-divorce data-portability model
  (`docs/design/sovereign-offboarding.md`) — directly relevant to Privacy
  (data subject rights / portability / disposal).

**What's missing:**
- **No formal data-classification scheme** (public / internal / confidential /
  restricted) written down and applied to the in-scope services.
- **No privacy notice mapped to the P-series criteria**, no documented
  retention/disposal schedule for service-held data, no DSAR process for hosted
  data.

### 3.8 Backup / availability / DR — Availability TSC

**What exists:** nothing operated — local data is the user's own files (their
backup responsibility). **[VERIFIED]** no service backup/DR because no service.

**What's missing:** backup, restore-testing, DR/BC plan, and an uptime/SLA
commitment for every in-scope hosted service. Availability is optional in SOC 2 —
but any *team-hosting* customer will demand it.

### 3.9 Gap scorecard

| SOC 2 area (criteria) | Local primitives | Service-ops layer | Verdict |
|---|---|---|---|
| Access control (CC6) | **Strong** (fail-closed bearer, per-device tokens, fed identity/TOFU) | **Missing** (multi-tenant RBAC, provisioning) | Build service authz |
| Encryption (CC6.1/6.7) | **Strong** (sovereignty guard, keyring, E2E design) | **Partial** (no at-rest KMS, fed crypto unbuilt) | Build for hosted stores |
| Audit logging (CC7.2/7.3) | **Strong** (append-only egress log, tamper-evident) | **Missing** (server-side audit, SIEM, retention) | Build service logging |
| Change mgmt (CC8) | **Good** (CI, branch protection, deny/dependabot, signed release) | **Gaps** (no required review, unsigned-fallback, no policy) | Tighten + document |
| Monitoring / IR (CC7.1/7.4) | Local crash capture; Decision Inbox | **Missing** (no monitoring/alerting/IR plan) | Build with services |
| Vendor mgmt (CC9.2) | Egress inventoried in code | **Missing** (no register/DPA) | Documentation lift |
| Data classification / Privacy (P) | **Strong** (opt-in default-off, redaction, offboarding) | **Missing** (scheme, notice, retention policy) | Documentation lift |
| Availability / DR (A) | N/A (user's files) | **Missing** (no backup/DR/SLA) | Build with services |

---

## 4. Roadmap

SOC 2 is **~70% policy + process + a third-party audit, ~30% technical controls.**
The code above is the strong 30%; the missing pieces are mostly written policies,
service-side operations, and evidence-of-operation over time.

### Phase 0 — Decide scope (weeks, no build)
1. Pick the **first service to certify** (see §5 — recommend federation relay +
   team hosting, since that's the enterprise-facing paid tier).
2. Choose **TSC categories**: **Security (required) + Confidentiality + Privacy**
   for the memory-relay use case; add **Availability** when a team-hosting SLA is
   sold. Defer Processing Integrity unless payments/marketplace demand it.
3. Select **tooling + auditor** (§5).

### Phase 1 — Write the policies (the bulk of the work)
Information security policy, access-control policy, **change-management policy**
(codify what CI/branch-protection already do + add required review), incident-
response plan, vendor/subprocessor register + DPAs, **data-classification scheme**,
data-retention/disposal schedule, privacy notice, risk assessment, BC/DR plan,
onboarding/offboarding (HR) controls. Most map onto controls that already exist in
code — this is largely *documenting* reality plus filling the named gaps.

### Phase 2 — Close the technical gaps (build, gated on the service existing)
- **Required PR review + `enforce_admins` + signed commits** on `main` (fast,
  do now — it's a settings change + team habit).
- Make the **signing/notarization path non-optional** in release.
- For the first in-scope service: **server-side audit logging**, **at-rest KMS
  encryption**, **multi-tenant access control + provisioning/deprovisioning**,
  **monitoring/alerting/on-call**, **backup + DR**.
- Implement the **federation crypto** the spec designs (it's the confidentiality
  control for the relay).

### Phase 3 — Type I (point-in-time)
Engage the auditor for a **Type I** on the first service: proves the controls are
*designed*. Achievable once Phase 1 + the service's Phase-2 controls exist.

### Phase 4 — Type II (observation window)
Run the controls for **6–12 months** with evidence collection (the compliance
platform automates most of this), then the **Type II** audit. This is the report
enterprise buyers ask for.

### Competitive context
The nearest cloud-agent comparators are already down this road, which sets the bar:
- **Opal Security** (opal.dev, access-governance) is **SOC 2 Type II** today, with
  annual pen tests, monthly vuln scans, TLS 1.2+/AWS-KMS-at-rest/daily encrypted
  backups ([opal.dev](https://www.opal.dev/)). *(Note: distinct company from the
  agent-builder "Opal / deployopal.com" referenced in our offboarding doc — the
  one whose Trust Center lists SOC 2 / ISO 27001 / ISO 42001 as **underway**, per
  Jesse's brief.)*
- The pattern is clear: enterprise agent platforms treat **SOC 2 Type II as table
  stakes**, and increasingly pair it with **ISO 27001** (infosec management) and
  **ISO 42001** (AI management systems).

Permagent's differentiator isn't matching that checklist item-for-item on the
client — it's that **our scope is smaller** (local-first keeps user data out) and
our data-protection primitives are **provable in code**, so the SOC 2 we do pursue
is narrower and cheaper to reach than a full-SaaS competitor's.

---

## 5. Honest caveats + open decisions for Jesse

**Caveats:**
- **SOC 2 is mostly not code.** The engineering is real but a minority of the
  effort; the majority is policy authorship, evidence collection, and paying a CPA
  firm. Don't expect a "build it and we're compliant" path.
- **You can't certify a service that doesn't run.** Most in-scope services are
  [DESIGN]/vision. SOC 2 only becomes actionable once at least one gated service is
  operating on our infra with real customers. Pursuing it earlier is premature.
- **Local-first is a genuine scope-reducer — say it, but don't overclaim.** The
  moment data crosses the boundary (§2.1 caveat), it's in scope. The egress audit
  is what lets us prove where the line is; keep it honest.
- **The strongest existing controls (sovereignty guard, egress log, opt-in
  telemetry) live on the *client* — which is out of scope.** They're superb for
  the *story* and for Privacy, but they are **not** the CC6/CC7 controls an
  auditor samples on a *service*. Those still have to be built service-side.

**Open decisions:**
1. **Which service first?** Recommend **federation relay + team hosting** (it's the
   enterprise paid tier that will be *asked* for SOC 2). Alternative: start with the
   two cloud endpoints that already exist (PostHog beacon + release infra) as a
   minimal first scope — cheaper, but low buyer value.
2. **Which TSC categories?** Recommend **Security + Confidentiality + Privacy**
   first; add **Availability** when you sell an SLA. Confirm.
3. **Timeline / trigger.** Tie the SOC 2 kickoff to the *first paying team
   customer* (or the first serious enterprise ask), not to a calendar date.
4. **Tooling.** **Vanta** vs **Drata** vs **Secureframe** vs **Comp AI** (the
   compliance-automation platforms that auto-collect evidence and pre-map controls).
   Pick one early — it shapes Phase 1. (Roughly $7–25k/yr + audit fee.)
5. **Auditor.** A CPA firm does the attestation; the platform doesn't. Choose one
   that knows dev-tool/AI startups.
6. **ISO 27001 / 42001 jointly?** The competitor bar includes both. **ISO 27001**
   shares ~80% of controls with SOC 2 (do them together if selling internationally);
   **ISO 42001** (AI management system) is increasingly asked of AI vendors and
   would be a *differentiator* to pursue early given the sovereignty story. Decide
   whether to scope 27001 alongside SOC 2 from the start.
7. **Fast, free win regardless:** turn on **required PR review + `enforce_admins` +
   signed commits** on `main` now (§3.4). It's a settings change that closes a real
   CC8 gap and costs nothing.

---

## Appendix — verified evidence index

| Control | Evidence (`file:line`) |
|---|---|
| Fail-closed bearer auth + constant-time | `crates/goose-server/src/middleware/auth.rs:63-106` |
| Per-device tokens, hashes-only, revoke | `crates/goose-server/src/device_registry.rs:100-101, :54-72` |
| Federation identity (Ed25519/X25519), TOFU, safety numbers | `crates/goose-server/src/auth.rs:219-345, :474-543, :561-580` |
| Secrets/keys at rest in OS keyring; hard-error not regenerate | `crates/goose-server/src/auth.rs:35, :229-240` |
| Sovereignty guard choke point (fail-closed egress) | `crates/goose/src/providers/sovereign_guard.rs:49, :59-104, :113-159` |
| Append-only egress audit (rejects UPDATE/DELETE) | `crates/goose/src/sovereignty/mod.rs:381-464, :700-705` |
| Strict-audit (no-unlogged-egress) | `crates/goose/src/sovereignty/mod.rs:248-275`; guard `:96-102` |
| Content hashed not stored by default | `crates/goose/src/sovereignty/mod.rs:41-42, :337-342` |
| Telemetry egress under boundary | `crates/goose/src/sovereignty/mod.rs:488-525`; `posthog.rs:78-86` |
| Opt-in analytics, default OFF, zero-beacon proof | `crates/goose/src/posthog.rs:50-52, :567-569, :651-686` |
| Redaction before egress | `crates/goose/src/posthog.rs:530-547, :600-611` |
| Egress-log viewer + Governance surface | `crates/goose-server/src/routes/security.rs:126-130`; `sovereignty/mod.rs:93-128` |
| Decision Inbox (human-approval seam) | `crates/goose/src/decision_inbox/mod.rs:19-61` |
| CI gates (fmt/clippy/test/locked) | `.github/workflows/ci.yml:115-125, :202-204` |
| Branch protection on `main` (no required review — gap) | GitHub API `branches/main/protection` |
| Dependency audit (cargo-deny + npm + daily cron) | `.github/workflows/dependency-audit.yml:36-59`; `.github/dependabot.yml` |
| Signed/notarized release + placeholder-pubkey guard | `.github/workflows/release.yml:100-104, :225-254` |
</content>
</invoke>
