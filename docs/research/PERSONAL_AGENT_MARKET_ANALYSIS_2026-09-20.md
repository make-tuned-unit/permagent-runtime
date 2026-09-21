# Grok Bot, Muse, and what the market wants from a personal agent — analysis for Permagent

**Date:** 2026-09-20 · **Status:** research, nothing built · **Asked by:** Jesse — "deep insight into their features and
how we could give Permagent capabilities that the market is wanting and adopting."
**Method:** three web-research lanes (Grok Bot teardown; Muse teardown; cross-vendor market study) + one read-only
capability map of this repo. Raw notes with every URL (≈2,600 lines): `docs/research/agent-market-evidence/`.
Companion: `BONSAI2_27B_REVIEW_2026-09-20.md`.

> **Labels.** `[VENDOR]` the company's own claim · `[PRESS]` reported · `[USER]` a named user's report ·
> `[DATA]` a survey or measurement with a method · `[UNVERIFIED]` could not be confirmed. All three web lanes exhausted
> their search budget; gaps are listed in §9, not papered over. Much 2026 agent coverage is SEO paraphrase of press
> releases — primary docs and named users were preferred.


> **CORRECTED AFTER ADVERSARIAL REVIEW (2026-09-21).** Codex Astra (`gpt-6-astra`) attacked this document; I verified its claims against the source. Full exchange and the revised plan: `CODEX_DEBATE_2026-09-21.md`. Errors found here:
> - **Menlo denominators were misread** (verified against the source): the 72/41/24 % figures are shares of *AI users*; the 92 % is agent users who pay for *AI*, not for agents; the access list is what users *have granted*, not a ranked preference.
> - **"Prosumer/operator" is a hypothesis, not a finding** — testimonials show possible jobs, not retention or willingness to run a Mac mini.
> - **The structural claims need qualifying:** no password hand-off is not no credential authority (the agent acts through authenticated cookies); local memory can still be lost to the agent, corruption or disk failure; no VM bill is not no hardware, power or upkeep.
> - **Build order revised:** one narrowly scoped real-browser workflow moves *earlier* — it is what this document's own evidence says users repeat. Watch approvals are deferred until phone/desktop approval friction is actually measured.
> - Our browser snapshot is not an accessibility tree; the brain router has two backfill POSTs, not one.
---

## 0. Verdict

1. **The market validated agents as a *trusted mode of a daily assistant*, not as autonomous workers.** Two of three
   frontier labs killed their separate agent surface this year (Google ended Project Mariner 5/4 on cost and latency;
   OpenAI removed agent mode in August with no notice). Autonomous checkout flopped on hard evidence. Big bot teams are
   the least-evidenced claim in the category. **Reliability and visible work beat every new capability.**
2. **What people actually repeat** is narrow and unglamorous: the target has **no API, so a logged-in browser session
   is the only way in** — supplier sourcing, permits and forms, refund chasing, ad-platform chores, ticket sniping.
   In every documented Grok Bot success, *operators beat knowledge workers*.
3. **Local-first wins on structure, not rhetoric.** Privacy ranks **third** among trust drivers (accuracy 45 % >
   trustworthiness 40 % > privacy 36 %) and a third of consumers reject on-device AI because they *don't see the use*,
   not because of privacy. The durable advantages are four: **no credential hand-off** (the user's own authenticated
   browser at their own IP), **memory nobody else can delete**, **no per-user VM cost**, and **Apple reach**
   (App Intents, on-device Foundation Models, watch). Build for people already running Ollama or Home Assistant.
4. **xAI conceded the architecture.** Grok Bot's default egress is shared datacenter IPs, so it shipped *"Route egress
   through this desktop"* — tunnel the cloud agent back out through the user's own Mac. A user who needed an inbound
   bridge built it on **a Mac mini running a LaunchAgent**. Every gap on Grok Bot's most-requested list is
   structurally solved by running where Permagent already runs; **uptime is the only advantage that needs someone
   else's computer, and an always-on Mac mini neutralises it.**
5. **Where self-hosted loses is polish and reliability, not capability** — *"OpenClaw broke every update for a month,
   so I gave up."* Setup takes 2–3 hours, sessions die on restart, "whatever breaks at 2am." Our own version of this:
   the always-on daemon needs `permagent setup` in a terminal, push is an ntfy topic from an env var, and Telegram is
   terminal-only. **Setup-friction parity is the differentiator that converts, not ideology.**
6. **The best ideas to steal are Meta's security design**, which is better than its trust position deserves:
   the agent never holds a credential; one process is the sole authority for egress; permissions are scoped
   *capabilities*, not chat intents; and data-taint tracking is used to **reduce** approval prompts.
7. **Cheapest wins first.** The capability map found ten gaps where most of the code already exists (§5, Tier 1).
   Several are hours, not weeks.

---

## 1. Grok Bot

**What it is.** xAI beta, 2026-08-11. After the parent closed a **$60B acquisition of Cursor on 08-14**, it is
operationally a Cursor product: Cursor-hosted VMs, Cursor's backend holds tokens, Cursor account/SSO/billing.

**Two corrections to the marketing.** (a) Bots do **not** each get a computer: there is **one shared Firecracker
microVM per user account**; each bot gets its own *screen*, which xAI calls "work surfaces, **not security
boundaries**", warning *"don't place a credential on the computer if another of your Bots shouldn't be able to use
it."* (b) "15–25 bots under a Chief of Staff" is unsupported: **group chats cap at 2–6 bots**, observed practice is
5–7, and **"Chief of Staff" is a prompt pattern, not a feature.** `[VENDOR]`

| Area | How it actually works |
|---|---|
| Sign-in | OAuth connectors: tokens stay on Cursor's backend, the model never sees them. Everything else: **"the Bot hands you the computer"** — you type passwords, 2FA, CAPTCHAs, payment. No vault, no CAPTCHA solving. |
| Approvals | Triggers: sending messages/invites, publishing, purchases/transfers, deleting/overwriting, permission changes, production changes, accepting legal terms. Allow once / Deny / Always allow. Team rules; personal rules may only tighten. **No spend limits of any kind.** |
| Teach a Task | Demonstrate a **browser** workflow once, ≤ 10 min, no audio → a **draft skill that "requires refinement before scheduling"** — you write the decision rules by hand. Desktop only. 50 routines/bot. |
| Teams | `@name` / `@everyone`, **text-only handoffs**; async bot-to-bot messages *wake* the recipient. Memory is per-bot; the *computer* is shared — shared at the substrate, private at the cognitive layer, the inverse of least privilege. |
| Memory | Preferences/facts/summaries; **no documented way to view, edit or delete one.** Each bot is **one unbounded thread** — no fresh session, no compaction, no context meter `[USER]`, confirmed by staff. |
| Multi-account | **Several Gmail addresses and Slack workspaces on one bot** — a reviewer's "made it genuinely useful from day one." |
| Egress | Shared static datacenter IPs across all customers; no dedicated IPs. Escapes: route through the member's desktop, or Tailscale / Cloudflare Tunnel. |
| Surfaces | iMessage-style app on desktop + mobile. **No inbound API. No voice, no web app, no X DMs/Telegram/SMS.** MCP client supported. Grok 4.6, auto-selected, **no BYO model.** |
| Audit | *"An audit view of Bot actions is coming."* Logs today are control-plane only and Enterprise-only; action recording is Enterprise, off by default. **No dry run — "a test run performs real work."** |
| Deployment | *"On-premises deployment, deployment inside your own perimeter, and bring-your-own-image deployment are not supported today."* |

**Pricing.** No standalone price and no free tier; entitlement across eight Cursor/SuperGrok tiers. Press: **$120/seat
team, $200 individual.** Weekly grant, then on-demand billed at token cost. **No published allowance, per-task charge,
concurrency cap or overage tariff.**

**Reception.** Hard numbers are thin: "thousands of organizations" and "millions of bots" are unattributable
`[UNVERIFIED]`; the honest signal is **69 marketplace bots from 43 creators** three and a half weeks in. The flagship
testimonial `[USER]` (HN top comment): *"I had one of my bots reach out to ~40 fabric suppliers in vietnam, negotiate
prices, lock one in, and get samples"* — and in the same breath, *"I've used less tokens in the last 5 years prior to
this month than I have this month."*

**Complaints, ranked:** (1) **token burn with no cap** — six agents ate 42 % of a weekly allowance on day one;
(2) **reliability** — the shared computer sticking takes down every bot at once, confirmed as architecture;
(3) **no isolation** between bots; (4) **account bans** — two named users had Gmail accounts disabled within days of
creating them through Grok Bot; (5) unbounded context; (6) billing confusion; (7) paywall; (8) no dry run or audit;
(9) no model or personality control. HN's second-order critique is sharp: *"The only cost was wasting the time of at
least 39 other people."*

**Sticky:** teach-by-recording, persistent logged-in sessions, multi-account connectors, bot-to-bot handoffs, giving a
bot its own email address. **Demo-ware:** big teams, the marketplace, "only comes back for approval."

**Vendor trust record.** Grok Build silently uploaded whole repos including API keys to a cloud bucket despite the
opt-out being off (July 2026); a Morse-code injection drained ~$150–200k through a Grok-connected bot; a zero-click
"cryptographic context injection" exfiltrates chat history from a summarised page — **encoding-layer attacks that
plaintext classifiers miss.** No injection-specific defence is documented.

---

## 2. Muse

**What it is.** Meta, 2026-09-08. US-only, 18+, iOS / Android / web / WhatsApp. Model: Muse Spark 1.3 (1M context).
**Muse for Mac** (09-19) reaches local Mail/Calendar/Notes/files — but inference stays in Meta's cloud.

**Mechanism** — from Meta's engineering post, the only document with real detail `[VENDOR]`:

- The "Secure VM" is a **`systemd-nspawn` container per user**, root-mapped to an unprivileged host user. Two services
  run **outside** it: `authd` and **Sentinel**.
- **The agent never holds a credential.** `authd` mints a *surrogate token*; Sentinel injects the real secret at the
  network boundary after authorising. **Sentinel is the sole permission authority for connector actions and network
  egress.**
- Grants are **capabilities**: one-time / session / task / time-bounded / perpetual, scoped to connector + use-case, and
  narrower than the OAuth scope (Gmail read minus settings).
- **eBPF "tainted egress"** marks processes that have read user data. Routine actions skip approval; actions touching
  user data do not. *This is the real answer to approval fatigue.*
- Browser: visible Chromium; DevTools off; the agent **cannot execute JS in the page**; the browsing sub-agent gets
  **accessibility-tree snapshots, not raw DOM**. **On user takeover or credential entry the agent pauses completely.**
- Payments: **Link by Stripe single-use cards — credential-tied, amount-limited, time-restricted.** **Checkout always
  requires human approval.** A card is required even on the free tier `[PRESS]`. No user-set spend caps.
- Proactivity: scheduled + event-driven tasks with the app closed; a control to reduce, increase or **disable**
  unprompted messages. Activity log, approval cards, **editable memory files**, goals tab. **No undo, no log export.**
- One person only — **no family, shared or group mode.** Connectors: Gmail, Google Calendar, Instagram, WhatsApp, Stripe
  Link (+ OpenTable, Peloton, Plaid per secondary sources). **Voice not shipped; glasses "coming soon."**
- Security-camera monitoring has **no published mechanism** — treat as demo-ware.
- Naming / avatar / style: present, and it generates animated avatars on request. **No reviewer leads with it.**

**Pricing.** Free = **100M tokens/week**; **$16 web / $20 iOS**; **$80 web / $100 iOS** (the difference is Apple's cut).
Tokens are the only meter; all tiers have identical capabilities. A reviewer who set out to exhaust the free tier used
**11 %**. Free compute is an acquisition subsidy.

**Adoption.** ~83k US iOS downloads in two days. One lane reports ~730k US in ten days and No. 1 free iPhone app on
09-18; another notes the early "#2" was *#2 in Productivity*. **The figures conflict and neither is company-confirmed**
`[UNVERIFIED]`. Both agree on **Google Play: No. 338 in Productivity** — an iOS-US media-cycle phenomenon. **No DAU,
retention or paid-conversion data exists, and no independent task-success study.**

**Trust.** `[DATA]` Oppenheimer, n=1,500: **8 % would trust Meta with passwords** (Google 30 %, Apple 23 %,
ChatGPT 16 %); **58 % would not give passwords to any agent**; **85 % demand visibility and control over their data.**
**Training on conversations and tool trajectories is default-ON.** Memory **cannot be fully disabled.** A Verge
reviewer was told: *"I read it through the account's API data, which surfaces more than the UI does."* Meta's own VP
Engineering: staff access "would still be technically possible." The bug bounty pays $130k for a one-user prompt
injection — an admission the class is unsolved. Internal-test incidents `[PRESS]`: exposed a user's iCloud photos
unrequested; a monitoring task "switched itself off for no apparent reason." **Confidential VM** (later 2026): skeptics
note attestation is unverifiable to a lay user, **key custody is unspecified — any recovery path means Meta holds it**,
a TEE hides data from the operator but not from the model, and trajectories are what Meta trains on.

**Reviewer consensus:** *"works and creeps me out."*

**Two usable by-products.** **Agent Plugins 1.0** — a vendor-neutral spec (manifest + skills dir + optional MCP config)
co-authored by Meta, OpenAI, Cursor, Microsoft, Amazon and Vercel. **Muse Glimmer 30B**, Apache-2.0, built "for
autonomous agentic tasks on consumer hardware" with an Apple-M-series speculative-decoding drafter — but 4-bit is
~20 GB and **does not fit our 16 GB minis.**

### Head to head

| | Grok Bot | Muse | Permagent today |
|---|---|---|---|
| Isolation | one shared VM per user, bots share credentials | container per user, credentials outside the runtime | runs on the user's Mac; write jail, egress inspector, sovereign guard |
| Credentials | OAuth on vendor backend; else user types into the VM | surrogate tokens, single egress authority | keychain for API keys; Gmail read-only OAuth; **nothing else** |
| Browser | cloud VM, datacenter IP | cloud VM, a11y-tree only, pause-on-takeover | **in-app Tauri webview only**, a11y-ref bridge, app must be open |
| Teach a task | browser demo → draft skill | — | repeated tool calls → skill proposal; **no GUI demonstration** |
| Approvals | per-action, tighten-only personal rules | capability grants + taint tracking | Decision Inbox, enforced tiers, earned-privilege ladder |
| Spend caps | **none** | **none** (single-use cards only) | LLM-token budgets only; **no real-money concept** |
| Memory | invisible, uneditable, unbounded thread | editable files, cannot disable, trains by default | 3D + list browser with provenance — **read-only; `forget` has no route** |
| Audit / dry run / undo | "coming" / none / none | log, no export / — / none | activity journal + traces; **no undo; no dry run** |
| Surfaces | own app only, no inbound | app, web, WhatsApp | desktop, iOS, watch, voice, Telegram (terminal setup) |
| Model choice | none | none | local + cloud roster, cost router |
| Self-host / export | refused / — | none / none | native / partial |
| Price | $120–200 | free / $16–20 / $80–100 | user's own hardware + their own model bills |

---

## 3. What the market has actually validated

**Adoption** `[DATA]` Menlo / Morning Consult, N=5,067 US adults, July 2026: **of AI users (not of all adults): 72 % aware of agents, 41 % tried, ~24 % use
regularly.** **92 % of agent users pay *for AI*** — not necessarily for agents — (vs 55 % of AI users); 21 % of them spend $100+/month. Consumer AI users grew
11 % YoY while spend tripled. **No cohort-retention number exists for any agent product, anywhere.** Scale check:
OpenAI's agentic tools reach ~10M weekly users against ChatGPT's 1B — **about 1 % of the base.** METR's 50 %-reliability
horizon for frontier models is ~2h17m — *at 50 %*.

**What current agent users *have granted* (not a willingness ranking):** **email 36 % > browser 33 % > messaging 31 % > files 29 % > calendar 27 % >
finance 20 %.** 32 % have ever let an AI act without approval. **17 % trust AI to complete a purchase; 9 %
unsupervised.**

**Jobs people repeat:** coding chores (the only unambiguous one); writing / information / practical guidance (78 % of
ChatGPT messages); everyday personal use with a **circadian rhythm** — habit, not novelty; then meeting notes,
scheduling, email triage. **Hard-evidenced flops:** autonomous checkout (OpenAI's Instant Checkout abandoned with ~30
merchants), pixel-level computer use, fully autonomous mode, the separate agent surface. **Unresearched, not flops:**
price monitoring, form filling, bills, home monitoring.

| Pattern | Status | Note |
|---|---|---|
| Approval gates with bounded mandates | **most convergent pattern anywhere** | Visa, Mastercard, Stripe ACP and Meta's Sentinel all landed on the same three primitives: a cryptographic agent credential, a bounded user-granted mandate, a signed auditable record per action |
| Scheduled / background work | emerging | universally shipped; ChatGPT Tasks "mostly tells you things" and **auto-pauses when ignored** — steal that |
| Teach a task | emerging | Anthropic "Record a Skill" (07-21) and xAI (Aug) converged within a month; **both treat the result as a draft you review**; zero usage data |
| Proactivity | emerging, fatigue is real | 77 % of employees say AI increased their workload; treat **3–5 notifications/day as a hard budget** |
| Messaging surface | validated for platform bots | **Meta banned third-party general AI chatbots from WhatsApp** — closed to us |
| Persona | validated for companions only | nobody reviews Muse for its avatar |
| Multi-agent teams | **hype** | least-evidenced claim in the market |
| Skills marketplaces | risky | **~12–20 % of OpenClaw's ClawHub skills were malicious** |
| Activity log / undo / editable memory | vendor-asserted | real evidence gap — but the 85 % "visibility and control" figure points here |
| Own computer vs the user's | **no user-preference data either way** | |

**The pain, in order.** False "done" — *vendor-admitted* (OpenAI: Work "can report a task as finished when part of it
isn't"). Cost — firms 3× over budget; a four-agent loop ran 11 days for $47k. **Prompt injection is the defining
story:** Anthropic's own figure is 31.5 % attack success without safeguards, 0.5 % with; Cowork was exploited within 48
hours by a Word document with invisible 1-pt text that uploaded financial documents and could create persistent
scheduled tasks; OpenClaw had a one-click RCE and up to 42k exposed instances. **Manus deleted eight months of users'
work on twelve days' notice** — the cleanest local-first argument that exists.

**Headwinds and tailwinds.** *Amazon v. Perplexity*: the injunction was **vacated by the Ninth Circuit on 08-04** — a
browser acting for its user does not "access" servers under the CFAA; that reasoning is strongest for an agent that
literally *is* the user's browser. Counter-trend: **Cloudflare Web Bot Auth** — 19 verified agents cover ~84 % of AI
browser traffic; an unaffiliated local agent is unverified by default. **WebAuthn's user-presence flag means a cloud
agent structurally cannot pass a passkey login.** MCP won its layer (97M monthly SDK downloads, governance at the Linux
Foundation). Four payment standards compete, no winner. **EU AI Act Article 50** (in force 08-02): an agent emailing on
your behalf in the EU must disclose it is AI. **Apple: SiriKit is deprecated — App Intents is the only path into Siri**,
and third parties get offline on-device Foundation Models.

**Local-first demand, honestly sized.** For: OpenClaw at 390k GitHub stars (live check), Ollama 181k; **Microsoft built
Scout on OpenClaw and is hardening it upstream.** Against: **no active-install number exists** (stars are not users);
no local agent has a published weekly-user count near six figures; local models trail frontier by ~10–20 points on
tool-use benchmarks; and people who score Meta at 8 % trust installed it anyway. **A prosumer opening, not mainstream.**

---

## 4. Where Permagent stands

Mapped from source — `CAPABILITY_INVENTORY.md` marks itself stale and `STATE_OF_AGENTIC_SUBSTRATE.md` understates the
current code.

**Works end-to-end:** launchd daemon that survives the UI closing (`goose-cli/src/commands/daemon.rs:6`); scheduler
with retries, missed-run recovery, **auto-pause after three failures** and Decision-Inbox escalation
(`scheduler.rs:24`) — *"every morning do X" works from UI and chat*; 13 background sweeps; durable goals across restart;
DAG orchestrator, bounded fan-out with per-child cost, audited agent-to-agent messaging; Decision Inbox with enforced
tiers, answerable from the phone; iOS app (53 Swift files); remote access over Tailscale; user-editable persona;
8-moment wizard; meeting write-ups, people/CRM, documents inbox, finance desks; hourly backups.

**What they lack and we have:** provable local-only inference, fail-closed, with an append-only egress audit
(`sovereignty/mod.rs:1-24`); on-device Apple Foundation Models; write jail + egress/adversary/security inspectors with
bypass accounting; a per-project **earned-privilege ladder**; a typed Decision Inbox that **learns from the user's
edits** (`decision_inbox/learn.rs`); a temporal knowledge graph with supersession and a 3D browser; an **honest liveness
contract** — the roster declares each desk `daemon | sim | static` and refuses to animate one with no emitter
(`roster.ts:14-26`); a local voice stack (17 modules); a job-health digest that enumerates *expected* jobs.

**Absent entirely:** acting in the user's real browser (no CDP, no Playwright, no extension — only an in-app Tauri
webview sharing one cookie jar, and the app must be open: `browser.rs:532`); screenshot/vision GUI control (AX-tree
only, flag-off); payments or any real-money limit (`cost_router/budget.rs` is LLM tokens only); connectors beyond
read-only Gmail and read-only EventKit calendar; iMessage, SMS, WhatsApp, Slack, email-as-interface; APNs; undo of
real-world actions; GUI teach-by-demonstration; a skills registry; user-defined triggers; federation between the two
minis (`federation/mod.rs` is slice 2 of 6, no transport).

---

## 5. What to build — ranked, mapped, local-first

Ordering rule: **market-validated × already-half-built.** Every item below stays on the user's hardware.

### Tier 1 — the code mostly exists

| # | Capability | Why the market says so | What exists → what's missing |
|---|---|---|---|
| 1 | **Memory you can edit, forget and export** | 85 % demand visibility and control; Grok Bot has none; Muse can't disable memory; Manus deleted users' work | `SafeBrain::forget / forget_scope / forget_keys` written and tested (`brain_handle.rs:224-270`), with internal callers only (document re-ingest `reader/mod.rs:314,601`, noise cleanup `activity/cleanup.rs:195`) — **no HTTP route and no UI** (verified: `routes/brain.rs:1327-1335` is GET plus two backfill POSTs; nothing under `components/brain/` mentions forget or delete) → a DELETE route, a button, an export. **The strongest differentiator per line of code in this document.** |
| 2 | **A spend dashboard and a hard cap** | #1 Grok Bot complaint; neither product has a cap; matches the cost-conscious routing principle | **Corrected 2026-09-21 (peer session permagent-runtime-46, verified):** this largely EXISTS — `ui/command-center/src/components/history/SpendPanel.tsx` renders the ledger rollup and sets session and per-task ceilings via `POST /api/governance/budget`; ceilings live in `cost_router/budget.rs` (task 1/3/6, session 5/12/25 USD). The read-only capability map missed it. What was actually broken was the **reservation** side — holds sized from a model's max output meant 63 reservations held $81.35 against $1.23 of real spend, so the gate refused work on money never spent (fixed in `11e5c913`) — and **pricing**: Sonnet 5 was stored at $3/$15 rather than $2/$10, and Fable/Opus 5 had no row and fell through to a $150/$600 worst case (fixed in `18c8de1e`). **Any dollar figure quoted from the ledger before those commits is unreliable.** Remaining gap: surfacing this as a first-run, product-level promise ("see what it costs, and cap it") rather than a History sub-panel, and a cap that covers background sweeps and monitors. |
| 3 | **User-defined monitors** — "tell me when this page changes / this price drops / mail from Y arrives" | background monitoring is table stakes; this is the no-API long tail people actually repeat | `skill_triggers` table (`spectral_schema.rs:394`) + `proactive.rs` candidate/judge/budget machinery + `rss.rs`. Verified: the table is written only with `trigger_type = 'repetition'` as a provenance record of *why a skill was proposed* (`skills.rs:122`); nothing ever updates `last_triggered_at` and **nothing evaluates a condition**. Copy auto-pause-when-ignored; the scheduler already pauses after three failures |
| 4 | **Notification budget** | fatigue is measured; Muse ships a dial | `proactive.rs:38-43` already has `MIN_GAP_HOURS 20`, quiet hours, 7-day same-subject suppression → expose as a user dial, cap 3–5/day across all sources |
| 5 | **Approvals on the watch + real push** | approvals are the most convergent pattern; phones are where they get answered | `WatchRelay.swift` and `POST /api/decisions/{id}/answer` exist, the watch renders neither; push is 40 lines of ntfy from an env var (`notification_router.rs:362-368`), **no APNs** |
| 6 | **Install parity** | self-hosted loses on setup, not capability | the DMG path spawns a child sidecar that dies with the app (`src-tauri/src/daemon.rs:17-21` names this); launchd needs a terminal; so do Telegram, Tailscale and push. **Goal: install → always-on agent with no terminal.** |
| 7 | **Turn on the draft-only sweeps** | email is the #1 permission users grant (36 %) | concierge, initiative, playbook, steward are default **off** and already draft-only by construction (`concierge.rs:1-22`) |
| 8 | **Skill import/export in Agent Plugins 1.0** | a six-vendor neutral spec; ours is already manifest + skills dir + MCP | deeplink encode/decode exists (`recipe_deeplink.rs`), no UI. **Sharing files, not a marketplace** — 12–20 % of ClawHub was malicious. Keep the OSV + sigstore check in the path (`extension_malware_check.rs:35-40`) |
| 9 | **Extension / MCP install panel** | MCP won its layer | catalogue, malware check and `manage_extensions` all exist; zero UI |
| 10 | **Fix the roster's honesty gaps** | "visible work" | Watcher is marked `sim` though `proactive.rs` is a real loop; Polybot is `static` and never emits `agent_state_changed` |

### Tier 2 — design work, adapted from Meta's security model

- **A credential broker with one egress authority.** Today: keychain + an egress *inspector* that extracts destinations
  by regex (`security/egress_inspector.rs:32`). Target: tools receive **surrogate tokens**; one daemon component injects
  the real secret at the boundary, and only after authorising. `sovereignty/` already fails closed at the provider
  factory — extend the same stance to connectors. This is also what makes **per-agent credential isolation** possible,
  the thing Grok Bot's shared computer cannot do.
- **Capability grants in the Decision Inbox.** Formalise "always allow" as a scoped object — connector + use-case +
  duration (once / session / task / time-bounded / perpetual) — with a **signed record per action**. That is the
  mandate-and-receipt shape the payment networks converged on, and it extends the earned-privilege ladder instead of
  replacing it.
- **Taint-based prompt reduction.** Track whether a session has read user data; routine untainted actions skip approval.
  We cannot use eBPF on macOS, but the daemon already mediates every tool call, so taint is a per-session flag set by
  memory, mail, file and document reads.
- **Dry run.** Called the single biggest gap in Grok Bot. We already gate every consequential tool call through
  `verification_approval::decide` and the `ToolInspector` chain — a mode that records the would-be action and stops is
  a short step.
- **An action audit trail with evidence.** The open-source clones all converged on **screenshots in-thread as
  evidence**. We have the activity journal and execution traces; add the evidence and a user-facing receipt
  (`receipts.rs` today is an eval counter contract, not a receipt).
- **Live view, takeover, pause-on-takeover** for the agent's browser. The webview and a11y bridge exist; there is no
  "watch it work" pane and no hand-back.
- **Undo, where the world allows it.** Neither product has it. Start with what is reversible by construction: files
  (the write jail knows every path), drafts, calendar entries, memory triples (`brain_handle.rs:1196` already undoes
  those).
- **Injection defence as a first-class subsystem.** Hostile-by-default framing of retrieved and browsed content;
  interactive-element-list browsing (ours is a DOM-derived list of interactive elements built by injected JS — **not** a native accessibility tree, and it runs in page context); no page-context JS; and remember that **encoding-layer attacks beat
  plaintext classifiers**, so the structural controls matter more than any detector — including the typed-decision
  detector proposed in the Jev analysis, which is itself injectable.

### Tier 3 — the structural bet

- **Act in the user's real, already-authenticated browser, at their own IP.** This is the one capability that is
  *better* local than cloud: no credential hand-off at all, passkeys work (a cloud agent structurally cannot satisfy
  WebAuthn user-presence), fewer bot blocks than a shared datacenter IP, no account bans for "new device in a
  datacenter", no VM bill — and the Ninth Circuit's reasoning favours an agent that is the user's browser. Today we
  have **none** of it. Route: a browser extension or CDP attach to a *dedicated profile* (not the user's main one —
  blast radius), a11y-tree snapshots, pause-on-takeover. **The security design must come first**; this is the largest
  new attack surface proposed here. Watch Web Bot Auth: an unaffiliated local agent is unverified by default.
- **Teach a task by demonstration → a draft skill.** Both vendors converged on the same shape: record once, produce a
  **draft the user reviews and edits**, then schedule it. We already turn repeated tool calls into skill proposals
  (`tasks/mod.rs:219`), gate execution (`skill_run.rs`), and track proven/stale. The missing piece is recording a
  browser session — which depends on the item above.
- **Several accounts per connector** (multiple Gmail, multiple workspaces). Small once a second connector exists;
  reviewers called it the day-one feature.
- **Connectors in the order users grant them:** email *write* (draft-only first), then messaging, files, calendar
  *write*. Finance is last (20 %) — and we already have more there than either product.
- **Payments: don't, yet.** 17 % trust an agent to buy; autonomous checkout is the category's clearest flop. If ever:
  single-use, amount-limited cards, and approval on every checkout — Meta's design, unchanged.
- **Apple surfaces.** SiriKit is gone; **App Intents is the only way into Siri.** Exposing Permagent's actions as App
  Intents puts Henry behind "Hey Siri" and Shortcuts on Mac, phone and watch — a structured action graph, which is
  exactly what did *not* fail in 2026.
- **Family / shared agents ("multiplayer").** The most-requested missing feature in Grok Bot power-user surveys; Muse
  is single-person by design. It depends on federation, which is slice 2 of 6 — real work, but nobody else has it.

### Do not copy

Big bot teams and a coordinator of 20 (hype; we already have a DAG coordinator that is real). A public skills
marketplace. More persona and avatar investment (nobody reviews for it; ours is already user-editable). One unbounded
thread per agent. A shared computer across agents. Default-on training. Autonomous checkout. "Only comes back for
approval" as a promise — the honest promise is **honest failure reporting**, and false "done" is the category's
vendor-admitted weakness. The completion-verification work in the harness is therefore product-facing, not just bench
hygiene.

---

## 6. Positioning

Not "private AI" — privacy ranks third and converts poorly. The claims the evidence supports:

1. **"It uses your logins, from your house — it never asks for your passwords."** 58 % won't give passwords to any agent.
2. **"Nobody can delete your agent's memory but you."** Manus is the proof; Meta's Confidential VM is an attempt to buy
   back what we have by construction.
3. **"You can see what it costs, and cap it."** Neither product can say this.
4. **"You can see everything it did, try it without consequences, and take it back."** Audit, dry run, undo — all three
   missing from both.
5. **"Bring any model."** Both refuse.

Audience: people already running Ollama, Home Assistant or a home server — and the operators with no-API chores who
turned out to be Grok Bot's real users.

---

## 7. Suggested order

1. **Tier 1 items 1, 2, 5, 6** — memory control, spend cap, watch approvals + push, install parity. Small, visible,
   and each one is a claim in §6.
2. **Tier 1 item 3** (monitors) with item 4 (notification budget) — the first new *capability*, and the one with the
   most existing scaffolding.
3. **Tier 2 credential broker + capability grants**, as one design — because it gates everything in Tier 3.
4. **Tier 3 real-browser agent**, security design first, then teach-a-task on top of it.

Fits the existing pattern: Fable plans and reviews, Opus/Codex workers implement, one worktree each.

---

## 8. Open questions for Jesse

1. **Is the real-browser agent in scope?** It is the biggest structural advantage and the biggest new attack surface.
   Everything in Tier 3 depends on the answer.
2. **Who is the user?** The evidence says prosumer/operator, not mainstream. That changes how much onboarding polish
   is worth versus capability.
3. **App Intents:** worth a separate spike — it is cheap to try and is Apple's only remaining door into Siri.
4. **Agent Plugins 1.0** as our skill format — adopt now, or wait to see whether it sticks?

---

## 9. What this research could not establish

All three web lanes ran out of search budget. Not covered or thin: Apple, Alexa+ and Perplexity head-to-heads with
Muse; r/LocalLLaMA and Reddit sentiment generally; voice and wearables as an agent surface; whether users prefer an
agent with its own computer or one using theirs (**no data exists either way**); any retention figure for any agent
product; independent task-success rates for either product; Muse's conflicting download figures; the scale of Grok
Bot's user base. Several widely repeated claims were traced to press releases and dropped.

---

## 10. Sources (load-bearing)

Grok Bot: [launch post](https://x.ai/news/introducing-grok-bot) · Cursor's Grok Bot security docs
(`cursor.com/docs/grok-bot/security`) · [HN thread, jjcm's comment](https://news.ycombinator.com/item?id=49261514)

Muse: [Meta newsroom](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/) ·
[product page](https://ai.meta.com/muse/) · Meta research, "How We Built Safety Into Muse" ·
[TechCrunch](https://techcrunch.com/2026/09/08/meta-debuts-its-muse-ai-agent-will-consumers-trust-it/) ·
[CNBC](https://www.cnbc.com/2026/09/08/meta-personal-ai-agents-public-reckoning-privacy-safety.html) ·
[Axios](https://www.axios.com/2026/09/08/meta-debuts-muse-personal-ai-agent)

Market: [Menlo Ventures, State of Consumer AI 2026](https://menlovc.com/perspective/2026-the-state-of-consumer-ai/)

Everything else — the Oppenheimer survey, the Verge and Gizmodo hands-on pieces, the Ninth Circuit ruling, Cloudflare
Web Bot Auth, the incident reports, and each named user quote — is cited with its URL in
`docs/research/agent-market-evidence/`.
