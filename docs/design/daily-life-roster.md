# The daily-life agent roster — Inbox, Day-planner, Researcher

**Epic:** #640 · **Status:** Design-first (no code) · **Author:** design agent · **Date:** 2026-07-22

> Permagent has world-class agent *infrastructure* but a builder's *roster*. Every
> "personal-agent OS" the landscape reviews name leads with life-management roles —
> email/inbox, plan-my-day, research/monitoring — and Permagent has none of them.
> This doc grounds those three roles in the code that already exists, recommends the
> lightest architecture, sequences the build, and surfaces the decisions for Jesse.

**Discipline note.** Every claim below is tagged **[verified]** (read at the cited
`file:line`) or **[assumed]** (design inference, not yet in code). Citations are to
the worktree at time of writing; line numbers drift — treat them as anchors, not
contracts.

---

## 1. What exists — the scaffolding a roster agent reuses

Adding these agents is **adding characters + tool wiring, not new architecture.**
The runtime already has three distinct "agent" shapes, a delivery pipe, and an
approval surface. Here is each, with what a roster agent reuses.

### 1a. Two things called "worker" — don't conflate them

**[verified]** There are two separate concepts:

- **Dispatchable workers** — goal-execution *delegates* the orchestrator (Henry)
  hands a task card to. Defined as `WorkerPersona` in `agent.yaml`, seeded from
  `default_roster()` (`crates/goose/src/config/agent_identity.rs:365`). Today's
  roster is four: `claude_code` (`:368`, `ExternalCli`), `codex` (`:397`),
  `librarian` (`:415`, `Pending` — registered but no engine wired), `reviewer`
  (`:428`, `InternalSubagent`, adversarial review). A worker runs via a
  `WorkerEngineKind` (`agent_identity.rs:207`): `InternalSubagent` (default,
  in-process), `ExternalCli` (Claude Code / Codex in an isolated worktree),
  `SupervisedCli` (S1 visible-terminal Claude Code), or `Pending`. The
  orchestrator selects + dispatches via `dispatch_goal_fn`
  (`crates/goose/src/agents/platform_extensions/orchestrator.rs:720`) and renders
  the roster into Henry's self-brief via `dispatchable_workers_from_config`
  (`crates/goose/src/agents/self_knowledge/mod.rs:250`).

- **Background workers / surfaces** — the always-on characters (Scheduler,
  Librarian, Steward, Initiative, Echo/Watcher) and user-facing surfaces (Reader,
  Decision Inbox, Inbox, Timeline, …). These are **not** goal-dispatch targets;
  they are `FeatureDescriptor` entries in the self-knowledge registry
  (`self_knowledge/mod.rs:144` `WORKER_DESCRIPTORS`, `:180` `SURFACE_DESCRIPTORS`).

**The daily-life roles are the second kind** — proactive characters that run on
their own and surface things — **not** orchestrator delegates. This is the single
most important architecture call in the doc (see §3).

### 1b. The proactive character pattern — Echo / the Watcher (#672)

This is the precedent the roster should copy. **[verified]** It is split in two:

- **Identity** lives in the `permagent`/goose lib: `crates/goose/src/echo.rs` —
  `WATCHER_NAME` (`:12`) and a `SELF_KNOWLEDGE_FEATURE` descriptor (`:16`,
  `id:"watcher"`, `category: Worker`) so Henry knows the Watcher "as one of his own"
  and can describe it. Registered into the roster at `self_knowledge/mod.rs:149`.
- **Behavior** lives in the daemon: `crates/goose-server/src/proactive.rs`. A
  long-lived `tokio` interval loop, `spawn(state)` (`:86`), ticks every 3h
  (`TICK`, `:34`) after a startup delay (`:35`). It computes candidates from
  sources — `compute_news` (RSS/Google-News, `:327`) and `compute_dormant`
  (a dormant Brain thread, `:394`) — hands them to the model for **taste**
  (`reason`, `:188`; provider `resolve_provider`, `:255`), and emits **at most one
  nudge a day** (budget `:100`, quiet hours 08:00–22:00 `:106`). It **only ever
  surfaces; it never acts.** Started at `crates/goose-server/src/commands/agent.rs:169`.

The Watcher is, in effect, **the Scout with two hardcoded sources.** The Scout is a
generalization of it (see §2c).

Sibling characters, same registry:
- **Librarian** — `crates/goose/src/agents/platform_extensions/librarian.rs:27`
  (`id:"librarian"`), memory-curation.
- **Steward** — `crates/goose/src/steward/mod.rs:404` (`id:"git_steward"`); it
  **proposes, never executes in v1** (`steward/mod.rs:236`), routing High/Critical
  git ops to human approval (`classify_risk` `:129`, `route` `:142`).
- **Initiative** — `crates/goose/src/initiative/mod.rs:44` (`id:"initiative"`),
  ambient origination, **default OFF**. Its driver
  (`crates/goose/src/initiative/driver.rs:1`) is a **native Rust interval loop, not
  a scheduler recipe** — a ruling (W6/W7) made deliberately because a scheduler
  recipe would spend agent tokens on every tick. **This ruling is load-bearing for
  the roster: proactive ticking is a native loop, not a recipe.**

### 1c. The delivery pipe — the Notification Router (#66, just merged cb0ad463)

**[verified]** `crates/goose-server/src/notification_router.rs` — the daemon-side
single policy boundary for turning events into notifications.
- `Severity { Info, Warning, Critical }` (`:20`); `Channel { Push, InApp, Digest }`
  (`:38`).
- `classify(event)` (`:91`) already handles `ProactiveNudge` and `TaskCompleted`
  (`:107`), `DecisionCreated` by tier (`:96`), `GoalStateChanged` (`:101`).
- Priority routing with per-user thresholds (`:60`, `:156`; NULL disables a
  channel), a 60s digest tick (`deliver_due_digests`, `:208`), ntfy-style push
  (`:269`). Spawned at `commands/agent.rs:173`.

**The roster reuses this verbatim:** a roster agent emits a `ProactiveNudge` (or a
digest-worthy event) and the router decides in-app vs OS-push vs daily-digest by the
user's thresholds. No new delivery code.

### 1d. The action / approval surface — the Decision Inbox (+ #760)

**[verified]** `crates/goose/src/decisions.rs` — "the daemon's single channel for
human/policy decisions." Answering mints a non-forgeable `DecisionProof` the
goal-transition guard requires; every step is hash-chained in `decision_audit`.
- `create_decision` (`:609`), `answer_decision` (`:1096`, answers
  approve/reject/choice/input/**edit** `:30`), tiered by `tier_for_action_class`
  (`:508`).
- **Tool-confirmation bridge (#760):** an answered `tool_approval` decision
  (`routes/decisions.rs:171`) is delivered back to the parked agent turn
  (`deliver_tool_confirmation` `:832`) — the same channel that used to hang on a
  modal. This is exactly the "propose an action → human approves → it executes"
  loop the Concierge's send and the Planner's booking need.

### 1e. The tool integrations each role needs — bundled vs deferred

| Integration | State | Evidence |
|---|---|---|
| **Gmail MCP** | **[verified] bundled + registered** | `extensions/gmail_mcp/server.py` exposes `gmail__search`, `gmail__read`, `gmail__list_labels`, `gmail__list_threads`, `gmail__send`. Registered as a Stdio extension (`routes/integrations.rs:355`, key `"gmail"`, cmd `permagent-gmail-mcp`), OAuth token injected via keyring. **OAuth scope is `gmail.readonly`** (`integrations.rs:14`). There is **no `create_draft` tool — only `send`** (`gmail_client.py:104`, "requires gmail.send scope"). |
| **Google Calendar MCP** | **[verified] NOT bundled** | No `extensions/*calendar*`, no MCP, no OAuth route. Only a **macOS AppleScript** read of the local Calendar app, surfaced as a read-only dashboard card (`routes/dashboard_cards.rs:391` `CALENDAR_APPLESCRIPT`, card `:136`). Calendar is the least-built dependency. |
| **web-search** (#357) | **[verified] available/deferred** | Not bundled; arrives via a connected Brave or Tavily MCP the user adds a key for (`agent_identity.rs:97` `WEB_SEARCH_FEATURE`; setup skill `builtin_skills/skills/web_search_setup.md`). Tools appear only once connected. |
| **Activity Journal** (#619) | **[verified] durable, present** | `crates/goose/src/activity_journal.rs`, append-only table, `record_event`/`page`; consumer wired at `state.rs:623`; `GET /api/activity` (`routes/activity.rs:98`). |
| **Telegram gateway** | **[verified] present** | `crates/goose/src/gateway/telegram.rs:16`; `create_gateway` (`gateway/mod.rs:97`); managed by `GatewayManager` (`state.rs:88`). Only registered gateway type today. |

### 1f. The cost of adding a character — self-knowledge & snapshots

**[verified]** A new character is a `SELF_KNOWLEDGE_FEATURE` descriptor
(`FeatureDescriptor`, `self_knowledge/mod.rs:128`) added to `WORKER_DESCRIPTORS`
(`:144`) **and** its id to `KNOWN_WORKER_IDS` (`:551`) — a test asserts the two sets
match exactly (`:593`), so a missing entry fails CI. Rendering it changes Henry's
brief, which **rewrites four insta snapshots**: `…prompt_manager__tests__basic.snap`,
`…typical_setup.snap`, `…one_extension.snap`, `…all_platform_extensions.snap` (the
worker block is baked at `basic.snap:42-43` today).

**Escape hatch [verified]:** the Playbook feature ships its descriptor **flag-gated
and hidden** (`worker_descriptor_visible` `:163`, `self_knowledge/mod.rs:157-162`)
precisely so the canonical `.snap` files stay **byte-for-byte unchanged** until the
flag is on. **Recommendation: build each roster character behind a flag** so the
snapshot churn lands once, at GA, not on every slice.

---

## 2. The three roles

Each is a **new character** (identity descriptor + a daemon-side behavior) plus a
**tool it mostly already has**, gated by the risk tiers already built.

### 2a. The Concierge — Inbox / communications  *(highest demand)*

**What it does.** Triage the inbox: read new mail, flag the urgent (family,
financial), archive/label the noise (newsletters), and **draft** replies — never
send without approval. The exact "semi-autonomous with approval" pattern the
research demands.

**Data/tools.**
- *Existing* **[verified]:** `gmail__search` / `gmail__read` / `gmail__list_threads`
  / `gmail__list_labels` cover read + triage fully. The `gmail.readonly` scope means
  **the Concierge is read-only by construction today** — it *cannot* send even if it
  tried. That is a safety feature, not a bug, for a draft-only launch.
- *Net-new* **[assumed]:** (1) a triage loop (clone `proactive.rs`); (2) drafting.
  Because there is **no `create_draft` tool and send needs a scope widening**,
  draft-only-first means composing the reply *in-app* (a Decision-Inbox card with an
  `edit`-able body) rather than writing a Gmail draft. Writing a real Gmail draft, or
  sending, is a **later slice** needing a `gmail__create_draft` / `gmail__send` path
  and the `gmail.send` OAuth scope.

**How it surfaces.** A daily/again-on-arrival **triage digest** via the notification
router (in-app + optional push), and per-reply **draft cards in the Decision Inbox**
(`edit` → approve → send, once the send path exists). Optionally a dedicated
"Communications" surface, but folding into the existing Inbox + Decision Inbox is
lighter (see §5).

**Autonomy ladder.** read-only triage/labeling → in-app draft cards → Gmail drafts →
send-with-approval → (never) auto-send. Each rung is a scope/tool addition, not a
rearchitecture.

### 2b. The Planner — Calendar / day  *(most gated)*

**What it does.** "Plan my day": read the calendar + active goals + open decisions,
propose a focus-time-blocked day, and surface what needs action. Read + propose
first; never book.

**Data/tools.**
- *Existing* **[verified]:** active goals (orchestrator goal cards), open decisions
  (`decisions.rs`), and — macOS only — today's events via the AppleScript dashboard
  read (`dashboard_cards.rs:391`).
- *Net-new* **[assumed]:** a real **Google Calendar MCP** (OAuth + read tools) is
  **not built** — it must be authored to make the Planner cross-platform and to read
  more than "today." Until then the Planner is **macOS-AppleScript-only and
  read-only.** This is why the Planner is sequenced last.

**How it surfaces.** A **morning digest** ("here's your day") via the notification
router, and action items as Decision-Inbox cards. Cadence is once-daily, not
continuous — so the Planner is the one role that fits a **scheduler recipe**
(`crates/goose/src/scheduler.rs:24`, cron + escalate-to-Decision-Inbox) rather than
a native loop.

### 2c. The Scout — research / monitor  *(lowest effort — the Watcher generalized)*

**What it does.** Proactive information curation: watch subscribed topics/projects
and brief "what matters before you search." The Watcher **already does this** for two
fixed sources (project news + dormant Brain threads); the Scout adds
**user-chosen topic subscriptions.**

**Data/tools.**
- *Existing* **[verified]:** the whole `proactive.rs` loop (candidate → taste →
  one nudge/day), the RSS/news source (`compute_news`), the activity journal (#619),
  the notification router.
- *Net-new* **[assumed]:** a small **topic-subscription store** (what to watch) and a
  brief-composition step. web-search (#357) deepens briefs but is
  connect-your-own-key (Brave/Tavily), so the Scout degrades gracefully to
  RSS-only when no search provider is connected.

**How it surfaces.** Identical to the Watcher — a rare, gentle nudge/digest via the
notification router; a "briefs" view is optional polish.

---

## 3. Architecture — recommendation

**Build all three as new *character-agents* on the Echo/Watcher pattern — a native
daemon behavior loop + a lib-side identity descriptor — delivered through the
Notification Router and gated by the Decision Inbox for any action. Not orchestrator
worker personas; scheduler recipes only for the Planner's daily cadence.**

Why, mapped to what exists:

- **Not orchestrator worker personas.** `WorkerPersona`/`default_roster` delegates
  are *goal executors* Henry dispatches a task card to and awaits a result
  (`orchestrator.rs:720`). The daily-life roles are *proactive and standing* — they
  watch and surface on their own, indefinitely, "even when you are idle." That is the
  **background-worker** shape (§1b), not the dispatch shape. Forcing them into
  `WorkerPersona` would mean Henry re-dispatching a goal on a timer — exactly the
  token-wasteful pattern the Initiative W6/W7 ruling rejected (`initiative/driver.rs`
  is a native loop *because* recipes cost tokens).

- **Character = identity (lib) + behavior (daemon), copied from Echo.** Each role
  adds a `SELF_KNOWLEDGE_FEATURE` in its own module (like `echo.rs`) registered in
  `WORKER_DESCRIPTORS` + `KNOWN_WORKER_IDS`, and a `spawn(state)` interval loop in
  `goose-server` (like `proactive.rs`) started in `commands/agent.rs`. The Scout can
  literally extend `proactive.rs`; the Concierge is a second loop over Gmail; the
  Planner is a daily recipe (its cadence suits the scheduler, its output the router).

- **Delivery and approval are already solved.** Emit `ProactiveNudge` → the router
  (§1c) handles channel/threshold/quiet-hours. Any *action* (send a reply, book an
  event) becomes a Decision-Inbox card with `edit`/approve (§1d), which is the
  supervised-tier philosophy Permagent already stands on. **Nothing new in the
  delivery or governance layers.**

- **Ties to the local-first thesis.** These characters are the payoff of a permanent,
  local, Brain-backed agent: it can watch your inbox/calendar/topics *because it
  lives on your machine and remembers.* The Watcher already reasons with
  `complete_fast` (a cheap/local-tier model, `proactive.rs:253`), so triage/curation
  can run on the local tier and only escalate to a frontier model for drafting — which
  matters because inbox and calendar are the most sensitive data in the product
  (see §5 privacy).

**Snapshot cost (§1f):** each character is a 4-`.snap` regeneration. **Recommendation:
ship each behind a feature flag using the Playbook hidden-descriptor pattern** so the
snapshot churn is a single deliberate GA step, and CI stays green through the slices.
*(Snapshots/self-knowledge are out of scope for this doc's edits — the build PRs own
that change; this doc only names the cost.)*

**One-line shape per role:**

| Role | Character shape | Behavior host | Primary tool | Delivery | Action gate |
|---|---|---|---|---|---|
| Concierge | new character | native loop (`proactive.rs` clone) | Gmail MCP (bundled, readonly) | router digest + draft cards | Decision Inbox (edit→send) |
| Scout | extend the Watcher | `proactive.rs` (+ topic store) | RSS + web-search (deferred) | router nudge/digest | none (surfaces only) |
| Planner | new character | scheduler recipe (daily) | Calendar (AppleScript now; MCP later) | router morning digest | Decision Inbox (propose→book) |

---

## 4. Phased path

**Recommended first: the Concierge (draft-only, read-only triage).** Rationale — it
is the #1/#2 most-demanded role, the Gmail MCP is the *only fully bundled*
integration of the three, and the `gmail.readonly` scope makes a triage launch
**safe by construction** (it *cannot* send). Shipping it moves Permagent from "a
developer's agent" to "the personal agent that also builds." **Honest caveat:** the
**Scout is the cheapest build** (it is the Watcher generalized — proven pattern, all
deps present, zero OAuth change), so if the goal is the fastest proof-of-pattern
rather than the highest-value role, build the Scout first. This is a genuine
value-vs-effort fork — see decision D2.

Buildable-now vs gated:

- **Slice 1 — Concierge, read-only triage [buildable now].** New character
  (descriptor, flag-gated) + a `proactive.rs`-style loop that searches/reads Gmail,
  classifies urgency, and emits a triage **digest** via the router. No new scope, no
  send. Fold surface into the existing Inbox + a "triage" digest.
- **Slice 2 — Concierge, in-app draft cards [buildable now].** Compose a suggested
  reply into a Decision-Inbox card with an `edit`-able body (no Gmail write yet). The
  human edits/approves; approval is a no-op-to-Gmail placeholder until Slice 3.
- **Slice 3 — Concierge, real draft/send [gated: OAuth scope + tool].** Add
  `gmail__create_draft` and/or wire `gmail__send`, widen OAuth to `gmail.send`, gate
  send behind the Decision-Inbox `tool_approval` bridge (#760). *Gate: scope change +
  a new tool + product decision on send autonomy (D3).*
- **Slice 4 — the Scout [buildable now].** Generalize `proactive.rs`: add a
  topic-subscription store and a topic-watch source alongside news/dormant. Briefs
  degrade to RSS-only without a connected web-search provider. Ships its descriptor +
  World-View avatar (the #388 pattern).
- **Slice 5 — the Planner, macOS read+propose [partially buildable now].** A daily
  scheduler recipe reading goals + decisions + the AppleScript calendar, emitting a
  morning digest. *Gated to macOS* until Slice 6.
- **Slice 6 — the Planner, real Calendar MCP [gated: net-new integration].** Author a
  Google Calendar MCP (OAuth + read; later propose/book via Decision Inbox).
  *Gate: build a new MCP + OAuth, the largest net-new lift in the epic.*
- **GA — flip the flags.** Regenerate the four `.snap` files once, un-hide the
  descriptors, add World-View roster avatars.

**Cross-cutting dependency:** any Spectral schema addition (a topic-subscription
table for the Scout, a triage-state or draft table for the Concierge) is
**mini-gated** — schema/eval work validates on the mac mini, not this machine, per
standing memory. Keep new tables additive and flag-gated.

---

## 5. Open decisions for Jesse

- **D1 — Character-agent vs worker-persona vs recipe.** This doc recommends
  **character-agents** (Echo pattern) for Concierge + Scout and a **scheduler recipe**
  for the Planner's daily cadence. Confirm, or steer toward orchestrator
  worker-personas if you want these to be Henry-dispatchable delegates instead of
  standing characters.

- **D2 — Which agent first: Concierge (value) or Scout (effort)?** Concierge is
  highest-demand + safe-by-construction but needs a new triage loop + surface; Scout
  is the cheapest (Watcher generalized) and proves the pattern fastest. The doc
  recommends **Concierge**; this is the main scope fork.

- **D3 — Concierge autonomy ceiling.** Where does the ladder stop? read-only triage →
  in-app drafts → Gmail drafts → **send-with-approval** → (never) auto-send. Sending
  needs a deliberate `gmail.send` OAuth scope widening + a send/draft tool + a
  Decision-Inbox gate. How far, and how much can it act without a click?

- **D4 — Planner data source.** Ship the Planner **macOS-AppleScript-only** first, or
  block it on authoring a real **Google Calendar MCP** (OAuth, cross-platform)? And
  its autonomy: propose-only vs propose-and-book (booking → Decision-Inbox gate).

- **D5 — Surface.** New dedicated **"Communications" tab** for the Concierge (and a
  Scout "briefs" view), or **fold everything into the existing Inbox + Decision Inbox
  + daily digest**? The doc leans fold-in (lighter, reuses the router).

- **D6 — Privacy / sovereignty *(the load-bearing one)*.** Inbox and calendar are the
  most sensitive data in the product, and cloud inference **does** send prompt data
  out (per the sovereignty-router direction). Should Concierge/Planner reasoning over
  email + calendar content be **pinned to the local tier** (the Watcher already runs
  on `complete_fast`), escalating to a frontier model only for drafting with an
  explicit egress log? This ties directly to the sovereignty-router work and should
  be settled *before* the Concierge reads real mail.

- **D7 — Naming.** Confirm Concierge / Planner / Scout as the character names + World
  View avatars (the #388 roster pattern), matching the existing Watcher / Librarian /
  Steward voice.

---

*Scope: this is a design doc only — no code, and it deliberately does not touch
`platform_extensions/mod.rs`, the `prompt_manager` snapshots, `spectral_schema.rs`,
or `self_knowledge`. Those changes belong to the build PRs the slices above describe.*
