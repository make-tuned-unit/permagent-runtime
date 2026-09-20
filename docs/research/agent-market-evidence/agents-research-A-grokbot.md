# Grok Bot — feature teardown, adoption, trust, competitive context

Research date: 2026-09-20. Researcher: subagent A.
Provenance tags: [VENDOR] = xAI/Cursor/SpaceXAI own docs or blog. [PRESS] = journalism/analyst.
[USER-REPORT] = named user on HN / Cursor forum / Reddit / X. [INDEPENDENT-TEST] = someone
ran it and reported method + result. [UNVERIFIED] = single low-quality or SEO-farm source.

**Method note / caveat on sources.** The brief's suggested starting points include several
sites that are SEO-farm explainers paraphrasing the press release (layer3labs.io,
aitoolsreview.co.uk, kie.ai, netalith, ayautomate, coursiv, callmissed, atomicbot, zentor,
aipricing.guru, cellcog, aibuilderclub, 4geeks, basenor). I used them only to find primary
URLs, and I have tagged anything that rests on them [UNVERIFIED]. The load-bearing sources
here are the two primary doc sets — **docs.x.ai/grok-bot/** and **cursor.com/docs/grok-bot/**,
which are the same documentation served under both brands — the x.ai launch post, the
Hacker News launch thread, and the Cursor community forum.

**Web search budget for this session was exhausted (200/200) partway through**, so section 5
(competitive context) and parts of section 3 rest on fewer sources than I would like. Gaps
are marked explicitly rather than filled.

---

## 0. Corporate context (matters for the whole teardown)

- SpaceXAI agreed to acquire **Cursor (Anysphere)** in a deal valued at **$60 billion**;
  the transaction **closed 2026-08-14**, three days after the Grok Bot beta launch.
  Cursor became a wholly owned subsidiary. [PRESS] https://en.wikipedia.org/wiki/SpaceXAI
- Consequence: Grok Bot is **operationally a Cursor product**. The cloud computers are
  "Cursor-hosted", connector tokens live "on Cursor's backend", the compliance posture is
  Anysphere's ISO certificates, the privacy policy is cursor.com/privacy, and billing runs
  through Cursor plans. The x.ai branding is a veneer over Cursor infrastructure. [VENDOR]
  https://cursor.com/docs/grok-bot/security-faq
- Reported precursor: "SpaceX's April deal with Cursor, the AI-coding start-up whose
  technology is being used to power the agent's computer environment." [PRESS]
  https://www.reworked.co/collaboration-productivity/xai-launches-grok-bot-ai-agents-in-beta/
- Prior xAI agent product **Grok Build** (CLI coding agent, $30/mo SuperGrok, up to 8 parallel
  agents, 70.8% SWE-bench Verified as of May 2026) had a **July 2026 scandal**: researcher
  **Tinh Dang** found it uploading entire source repos — including API keys and secrets — to a
  Google Cloud Storage bucket **despite users disabling "Improve the model"**. xAI open-sourced
  it under Apache 2.0 in response. [PRESS] https://en.wikipedia.org/wiki/Grok_Build
  → This is the single most important piece of prior context for trust. It is also exactly
  what the top security comment on the HN launch thread referenced (see §3).
- xAI 2025 financials: $3.2B revenue, $6.4B operating loss. [PRESS] en.wikipedia.org/wiki/SpaceXAI

---

## 1. COMPLETE FEATURE INVENTORY

### 1.0 Launch timeline (all [VENDOR] unless noted)
| Date | Event |
|---|---|
| 2026-08-11 | Grok Bot beta. macOS + iOS. SuperGrok / SuperGrok Heavy / Cursor Pro / Pro+ / Ultra / Teams Standard + Premium. https://x.ai/news/introducing-grok-bot |
| 2026-08-14 | SpaceXAI–Cursor acquisition closes [PRESS] |
| 2026-08-21 | Access expanded to SuperGrok Plus, Cursor Pro+, Cursor Teams |
| 2026-08-26 | Templates / multi-account / network-exit-routing announced as in-flight [UNVERIFIED — aibase] https://news.aibase.com/news/30628 |
| 2026-08-28 | Marketplace launch (templates/bots) [PRESS-ish] https://www.explainx.ai/blog/spacexai-grok-bot-marketplace-launch-2026 |
| 2026-09-02 | Android app |
| 2026-09-03 | Enterprise launch; two-week free trial for Grok and Cursor enterprise customers |
| — | **No general-availability announcement. Still beta as of 2026-09-20.** explainx explicitly checked and rebutted "beta exit" framing. |

### 1.1 The cloud computer — THE BRIEF IS WRONG ON THIS POINT
The brief says "each with its own cloud computer." The documentation says the opposite, and
this is the most consequential architectural fact in the product.

- **One persistent cloud computer per user account, shared by every Bot that user owns.**
  "Every Bot on your account uses the same computer: browser sessions, files, and
  command-line credentials are shared." [VENDOR] https://cursor.com/docs/grok-bot/security
- Each Bot gets **its own screen** on that one machine, so several Bots can drive browser and
  desktop tools in parallel — "but the screens are work surfaces, **not security boundaries**."
  [VENDOR] ibid.
- Explicit vendor warning: **"Don't place a credential on the computer if another of your Bots
  shouldn't be able to use it."** / "Treat a login or file on the computer as available to
  every Bot." [VENDOR] https://cursor.com/docs/grok-bot/security-faq
- **Isolation is per-user, not per-bot**: a dedicated **Firecracker microVM** per user, with its
  own kernel, memory and virtual devices; one user cannot reach another user's computer.
  [VENDOR] cursor.com/docs/grok-bot/security
- OS: Linux VM. Contents: **a browser, a terminal and a filesystem**, on durable disk —
  "Local member files persist across sessions on durable disk." [VENDOR] docs.x.ai/grok-bot/security
- **Installable software**: yes, via **Team Setup manifests** — "manifests of install scripts
  that run on every team computer, so the same tooling is present everywhere." Individual users
  can also install through the terminal. Caveat from a user: **"Update/Reset rebuilds it"** —
  the machine can be reset and anything you installed outside a manifest is gone. [USER-REPORT]
  grossera, https://forum.cursor.com/t/grok-bot-can-i-send-it-a-message-from-outside/168199/15
- **No on-prem, no own-perimeter, no bring-your-own-image.** Verbatim: "Grok Bot runs only on
  Cursor-hosted cloud computers. On-premises deployment, deployment inside your own perimeter,
  and bring-your-own-image deployment are not supported today." [VENDOR]
  https://cursor.com/docs/grok-bot/security  and repeated as a Q&A in the security FAQ.

### 1.2 How Bots sign into user accounts
Two distinct paths, and the split is the interesting part:

1. **Connectors / plugins (OAuth)** — "Connector tokens stay on Cursor's backend. Bots invoke
   tools without receiving OAuth tokens, and **tokens are never stored on the computer**."
   [VENDOR] cursor.com/docs/grok-bot/security. So for OAuth-able apps the model never sees a token.
2. **Computer use (browser session)** — for everything else. "the Bot **hands you the computer**"
   for passwords, passkeys, two-factor codes, CAPTCHAs, payment or identity checks, and sites
   that require a human. The human types; the session then persists on the shared machine.
   [VENDOR] cursor.com/docs/grok-bot/get-started, /security
- There is **no credential vault the bot draws from**. There is a **"secure secret request"** for
  supported connections: masks the entered value, keeps it out of the transcript and away from
  the model. [VENDOR] docs.x.ai/grok-bot/approvals-security-and-privacy
- **Hardware security keys (YubiKey) require per-use approval** on supported platforms. [VENDOR] ibid.
- **2FA / CAPTCHA are explicitly NOT automated** — they are handoff points to the human.
  This is a real product limitation and also the honest choice.
- A Bot **"has no identity of its own and cannot hold more access than the signed-in member."**
  [VENDOR] security-faq. Everything stays attributable to a named member.

### 1.3 Approvals model
- **What triggers an approval** (verbatim list): sending messages or invitations; publishing
  content; purchases and financial transfers; deleting or overwriting data; changing
  permissions; production changes; accepting legal terms. [VENDOR]
  https://docs.x.ai/grok-bot/approvals-security-and-privacy
- **Where it surfaces**: desktop shows *Allow once / Deny / Always allow*. Mobile (iPhone,
  Android) shows an **Auto-review sheet** with *Allow / Deny / Always allow*, plus a
  local-command card with *Allow once / Deny*. [VENDOR] ibid.
- **Auto Review**: a model-based check of Bot actions against the member's request. Explicitly
  described as the *top* layer, with model-independent controls beneath it: "beneath it sit
  controls that don't depend on any model's judgment, including the network policy, per-action
  approvals, and per-user isolation." [VENDOR] cursor.com/docs/grok-bot/security
- **Policy**: team admins set team-wide *Ask first* / *Allow automatically* rules that apply to
  every member's Bots, with an enforcement switch that locks them. Personal rules layer on top
  but **can only increase strictness**; "*Ask first* wins when rules conflict."
  [VENDOR] https://docs.x.ai/grok-bot/teams-and-enterprises
- **SPEND LIMITS: none.** "there is no Grok Bot-specific spend cap yet" [UNVERIFIED — eesel,
  https://www.eesel.ai/blog/grok-bot-pricing]; corroborated by the absence of any spend control
  in the approvals doc, which lists purchases only as an approval category, not a budget.
  Given the token-burn complaints in §3 this is the single loudest gap.

### 1.4 Teach a Task (a.k.a. "show a Bot how it's done")
- Vendor framing: the Bot "watches the steps and remembers how you like the work done" and
  "saves your workflow as a routine." [VENDOR] https://x.ai/news/introducing-grok-bot
- Mechanics: demonstrate a **browser** workflow once, in a **one-to-one** conversation. It
  records **visible computer interaction for up to ten minutes**, **without microphone audio**,
  and converts it into a **draft skill**. [VENDOR] https://cursor.com/docs/grok-bot/work
- **Generalisation is not automatic.** The draft skill "requires refinement before scheduling" —
  you must add the decision rules by hand. A skill is defined as "a reusable set of instructions
  for how to do a task", capturing steps, decision rules, expected output and safety boundaries.
  [VENDOR] ibid. Independent review agrees: "Records up to 10 minutes of browser activity to
  generate draft skills, though you must add decision rules manually." [PRESS] eesel review.
- **Desktop only.** Teaching demonstrations and editing/testing schedules cannot be done from
  mobile. [VENDOR] cursor.com/docs/grok-bot/get-started
- Skills are **account-wide** (available to all your Bots) and invoked with `/` in the composer.
- This is the feature the community converged on fastest — see §3.

### 1.5 Routines / scheduling
- A routine runs on a schedule or after a supported event (e.g. a Slack message, a GitHub
  notification). Runs in the cloud with your laptop closed.
- **Hard limits: 50 routines per Bot; 20 most-recent run records retained per routine.**
  [VENDOR] cursor.com/docs/grok-bot/work
- 20 run records is a thin history for anything you want to audit.

### 1.6 Bot Teams / Chief of Staff — the brief's "15–25 bots" is NOT supported
- **Group chat limit is 2 to 6 Bots.** Verbatim: "Connecting two to six Bots in a group chat
  enables shared outcomes with visible handoffs." `@` addresses one teammate, `@everyone`
  broadcasts. **"Bot-to-group handoff messages are text-only."** [VENDOR] cursor.com/docs/grok-bot/work
- **Bot-to-bot messaging** is separate from group chat and is asynchronous: a Bot sends a message
  that **wakes the recipient Bot** to handle the request. Good when different Bots own different
  systems, or for long-running jobs. [VENDOR] ibid.
- **No documented cap on total Bots per account.** The 15–25 figure in the brief appears to come
  from usage anecdotes, not docs. The closest real data points: a SpaceXAI engineer's workshop
  showing **10–20 Bots under one Chief of Staff** [UNVERIFIED — secondary]; a user running
  **seven** [USER-REPORT, aimaker.substack.com]; two independent users running **six**
  [USER-REPORT — Ryan Perry; Eugeniu Ghelbur, via sidsaladi].
- **"Chief of Staff" is a prompt pattern, not a product feature.** There is no Chief-of-Staff
  object in the docs. It is a Bot whose description tells it to check whether another Bot already
  owns a task, delegate to that specialist, handle it directly only if nothing fits, then collect
  results and report back in one thread. Community prompt charters circulate for it.
  [USER-REPORT] https://x.com/i/article/2092216029816472002 (kocer);
  https://botdirectory.ai/bots/bot-chief-of-staff/
- **Shared vs private memory**: memory is per-Bot. The *computer* (browser sessions, files,
  terminal creds) is shared account-wide. So state is shared at the substrate level and
  private at the cognitive level — the inverse of what you'd want for least privilege.

### 1.7 Memory
- "A Bot retains stable preferences, important facts, and summaries from its work." Explicitly
  **not** a substitute for authoritative sources; corrections and safety boundaries are supposed
  to go in the **Bot description**, not memory. [VENDOR] cursor.com/docs/grok-bot/work
- **User-visible/editable memory is undocumented.** No documented way to inspect, correct or
  remove an individual memory. Flagged by a competitor comparison as a gap: "Limited transparency
  on how users inspect, correct, or remove individual memories." [UNVERIFIED — vellum.ai]
- Related and worse: **each Bot is one unbounded thread.** Michael Greenhalgh (co-founder,
  Health.AI), 2026-08-13 — Cursor staff confirmed **no fresh-session option, no manual
  compaction, no visible context meter**. [USER-REPORT via cellcog, forum.cursor.com]

### 1.8 Templates / marketplace / sharing
- **Share a Bot via public link.** The link exposes **configuration but not conversation history
  or logins**; the recipient adds a copy to their account. Teams/Enterprise admins control
  whether members may publish templates publicly. [VENDOR] cursor.com/docs/grok-bot/work
- A template packages identity, skills, connected tools and routines for one-click install.
- **Marketplace**: launched 2026-08-28. As of 2026-09-05 it carried **69 public bots from
  43 creators across 9–10 categories** — but this count comes from third-party trackers
  (blockchain.news, botmrr.io), **not** from xAI. [UNVERIFIED-count / PRESS-analysis]
  https://www.explainx.ai/blog/spacexai-grok-bot-marketplace-launch-2026
- Separately, the in-app **plugin** catalogue reportedly carries **219 plugins**
  [UNVERIFIED — roasbeast.com]. A community directory counts **31 integrations + 5 built-in
  skills + bring-your-own-MCP** on the Grok side. [UNVERIFIED — github.com/rdmgator12/awesome-grok-connectors]
- **No published revenue share, no paid-template mechanism, no published security-review process
  beyond editorial curation.** [PRESS-analysis] explainx.
- Roadmap signal: "Bots is what's coming: Third-party Grok Bots you can add straight to your
  team. Pick one, review the instructions (and memories, skills, routines, integrations…)"
  [USER-REPORT/leak] https://x.com/blankspeaker/status/2093399672803176787

### 1.9 Multi-account switching
- Two different things share this name:
  1. **App-level account switching** — add several Grok Bot accounts to the app and switch
     between them, to separate personal / work / org. [PRESS] testingcatalog.com;
     [UNVERIFIED] aibase.
  2. **Multi-account connectors** — connect **several Gmail addresses and several Slack
     workspaces to a single Bot**. This is the one users actually rave about: "most platforms
     assume each person has one Gmail account and one Slack workspace… Grok Bot lets her connect
     all of them to a single bot, which made it genuinely useful from day one."
     [PRESS] https://www.lennysnewsletter.com/p/how-i-ai-grok-bot-grok-46whats-great
- **This is the most under-rated shipped feature and the clearest thing to copy.**

### 1.10 Network exit routing — NOT residential IPs
- Default: hosted computers reach the internet through **shared static egress IP addresses**,
  and **"the ranges are shared across Grok Bot customers."** **Dedicated per-customer egress IPs
  are not available.** Treat the ranges as identifying Grok Bot traffic, not your team.
  [VENDOR] https://docs.x.ai/grok-bot/private-networks
- These are recognisable **datacenter** IPs carrying **shared reputation from other people's
  bots**, so sites sensitive to automation treat them aggressively.
  [INDEPENDENT-TEST] https://helenaai.substack.com/p/grok-bot-vms-ips-setup-prompts
  (tester confirmed all bots on one computer surfaced the same public egress IP, clearly datacenter)
- **Two escape hatches** — and note that both are "borrow a real network", not "buy residential
  proxies". xAI ships no proxy/geo product.
  1. **Route egress through this desktop** — a per-member setting that tunnels the cloud
     computer's web traffic back through the member's own desktop, using its network and IP.
     [VENDOR] docs.x.ai/grok-bot/private-networks; [UNVERIFIED-framing] aibase calls it an
     "exit tunnel" for when datacenter IPs are blocked.
  2. **Install a networking client on every hosted computer via Team Setup** — **Tailscale**
     (exit node inside your network) or **Cloudflare Tunnel** (`cloudflared`). Check scripts
     `command -v tailscale` / `command -v cloudflared`. [VENDOR] ibid.
- Limitations, verbatim: "Cursor does not install, operate, or monitor your networking client";
  ordinary internet traffic still uses Cursor's shared IPs unless routed; **no fleet-wide view of
  script deployment results**; **setup scripts cannot contain credentials or secrets.**
- **Strategic read for a local-first runtime:** the fact that xAI had to ship a feature whose
  entire purpose is "send the cloud agent's traffic back out through the user's own machine"
  is an admission that the cloud-hosted agent has a fundamental network-identity problem that
  a local agent does not have. This is the strongest single argument in the whole teardown.

### 1.11 Integrations / connectors
- Design principle, verbatim: **"connectors where available and computer use for everything
  else."** Some sites block automation or require a human. [VENDOR] docs.x.ai/grok-bot
- Named in docs: **Gmail, Notion, Slack**, GitHub (as a routine trigger). Plugins are
  **account-wide** and can be blocked by team admin policy. [VENDOR] cursor.com/docs/grok-bot/work
- **MCP is supported**, including custom/remote MCP servers and stdio setup; MCP authentication
  appears as an auditable control-plane event. [VENDOR audit-log list] + [UNVERIFIED — composio]
  https://composio.dev/content/how-to-add-mcp-servers-to-grok-bot
- Third-party plugin ecosystem exists (Customer.io publishes an official plugin doc for
  "Cursor and Grok bot": https://docs.customer.io/ai/plugins/cursor-grok-bot/ [VENDOR-3P]).

### 1.12 Messaging surfaces — thin, and a real weakness
- In-product: an **iMessage-style chat UI** on desktop + mobile. That is the primary surface.
- **There is no official inbound API. You cannot message a Bot from outside.** A user built a
  workaround: custom Slack app → **a Mac Mini running a LaunchAgent listener** → Cursor webhook.
  His warnings: "You cannot trust the webhook body, so a local listener that delivers the payload
  is a good alternative" and **"Do not put that listener on Grok Bot's cloud computer.
  Update/Reset rebuilds it, so the listener would be gone."**
  [USER-REPORT] grossera, https://forum.cursor.com/t/grok-bot-can-i-send-it-a-message-from-outside/168199/15
- Known Slack limitation: the Cursor Slack connector **posts as the logged-in human**, not as a
  bot identity. Hence the custom Slack app workaround. [USER-REPORT] ibid.
- **Email**: no native bot mailbox. Third parties fill the gap — AgentMail gives each bot a real
  owned address (`X@yourworkspace.agentmail.to`) with thread history and `reply_to_message`.
  [VENDOR-3P] https://www.agentmail.to/build/grokbot
- **No X DMs, no Telegram, no SMS, no voice, no web app** documented. A competitor roundup lists
  "Restricted to desktop apps and iOS; lacks email, Slack, Telegram, phone, or web app access"
  as a reason to leave. [UNVERIFIED — vellum.ai]
- **No voice interface at all.** Notable given xAI ships voice in the Grok consumer app.

### 1.13 Mobile & platforms
- Desktop: **macOS (Apple silicon + Intel), Windows (x64/Arm64), Linux (x64/Arm64)**, auto-updating.
- Mobile: **iPhone (iOS 18+), iPad, Android** (Android shipped 2026-09-02).
- Mobile can: message Bots, **watch and take over the computer**, approve actions, pause/resume
  routines. Mobile **cannot**: record a teach-a-task demonstration, edit schedules, test routines.
  [VENDOR] cursor.com/docs/grok-bot/get-started, docs.x.ai/grok-bot

### 1.14 Activity logs / replay / audit — weak, and the vendor says so
- **"An audit view of Bot actions is coming."** [VENDOR, quoted by PRESS] eesel review.
  As of now there is no per-response action log for individuals.
- **Action Recording**: **Enterprise only, off by default, 90-day retention.**
  [VENDOR] https://docs.x.ai/grok-bot/security; corroborated https://porteden.com/blog/muse-vs-grok-bot-vs-instinct/
- **Audit logs**: Enterprise only. Cover admin, security and authentication events plus Grok Bot
  **control-plane** events — Bot creation, member access changes, Team Setup manifests, MCP
  authentication, Slack account links, routines. Streamable to SIEM.
  **Note what this is not: it is not a log of what the Bot did in your apps.** [VENDOR] ibid.
- The docs advise you to ask the Bot for "source links and an action log" for important
  decisions — i.e. the audit trail is model-generated prose, not a system record. [VENDOR]
  docs.x.ai/grok-bot/approvals-security-and-privacy
- Routines keep only the 20 most recent run records.
- **No dry run.** Independent review calls this the biggest gap: "a test run performs real work,
  it can navigate websites, change files and call connected tools." You cannot rehearse on
  historical data. [PRESS/INDEPENDENT] https://www.eesel.ai/blog/grok-bot-review

### 1.15 API / SDK / MCP
- **MCP: yes** (client side — Grok Bot consumes MCP servers, incl. custom ones).
- **Public API / SDK to drive Grok Bot: no.** Confirmed by the absence of any in the docs and by
  the forum thread above, where the only route in is an undocumented Cursor webhook.
- **[UNVERIFIED]** whether that webhook is supported or will persist.

### 1.16 Models
- **Grok 4.6** powers it, with **Grok 4.7** imminent (3–4 weeks from late Aug). Grok 4.7 is
  described as trained on SpaceX engineering data for real-world problem solving.
  [PRESS] https://www.bensbites.com/p/grok-bot-is-not-what-you-think
- **The model is auto-selected per task. No manual override, no advanced mode at launch.**
  [UNVERIFIED — layer3labs/bleap tier, but consistent with Lenny's newsletter complaint that
  users have no ability to "select models"]
- **No BYO-model, no alternative providers.** "Users cannot select alternative model providers;
  the system ties workflows exclusively to xAI and Cursor products." [UNVERIFIED — vellum.ai]
- Note the odd phrasing in the Cursor doc set: one page says "The documentation does not specify
  which **Claude** models power Grok Bot" — an artefact of Cursor's docs, and a hint that the
  underlying agent harness is Cursor's, which is multi-provider internally. **[UNVERIFIED]** —
  do not repeat as fact.

---

## 2. PRICING, LIMITS, QUOTAS

### 2.1 Access matrix [VENDOR] https://cursor.com/help/grok-bot/plans
Grok Bot is **not sold standalone**. There is **no free tier**. It is an entitlement on a
Cursor or SuperGrok plan, and it has **its own usage meter, separate from your Grok and Cursor
plans** [VENDOR, x.ai/news/introducing-grok-bot].

Cursor plans (highest → lowest weekly usage):
- Cursor **Ultra** — highest weekly usage
- Cursor **Pro+** — generous, below Ultra
- Cursor **Pro** — below Pro+ (reported $20/mo)
- Cursor **Teams** (self-serve) — follows team plan allowance (reported $40/seat)
- Cursor **Enterprise** — admin-managed access

SuperGrok links (individual accounts only):
- **SuperGrok Heavy** — highest linked usage
- **SuperGrok Plus** — below Heavy
- **SuperGrok** — below Plus (reported $30/mo)
- **X Premium+** — below SuperGrok Plus

As of 2026-08-26 it is reportedly bundled into **all eight** Cursor + SuperGrok plans.
[UNVERIFIED — eesel/cellcog]

### 2.2 Prices
- Launch-era press: **$120/seat/month team, $200 individual** — "figures that read as enterprise
  software, not an app for the curious." [PRESS]
  https://www.reworked.co/collaboration-productivity/xai-launches-grok-bot-ai-agents-in-beta/
- Secondary sources place $120/seat on **Cursor Premium Teams** and say the practical individual
  floor is **Cursor Ultra at ~$200/mo** (or SuperGrok Heavy). [UNVERIFIED — eesel, cellcog]
- **Enterprise: contact sales, after a two-week free trial.**
- **Bundled at no extra charge if you already pay for SuperGrok Heavy or Cursor Ultra.** [UNVERIFIED]
- Numbers conflict across sources; xAI publishes no price table for Grok Bot itself. Treat the
  $120/$200 figures as [PRESS] not [VENDOR].

### 2.3 Quotas — deliberately opaque
- **Weekly included usage grant that resets weekly.** When exhausted, **on-demand usage**
  continues if enabled and counts toward the **on-demand monthly limit**. [VENDOR] cursor.com/help/grok-bot/plans
- **The docs publish no numbers**: "the documentation does not publish a universal Bot allowance,
  per-task charge, compute-hour rate, concurrency limit or overage tariff." [UNVERIFIED — eesel,
  but I confirmed the absence directly against the Cursor plans page.]
- Overage is **"billed from model and token cost"** — i.e. metered on tokens, not compute-hours.
  There are **no published compute-hour prices** because the VM is not metered separately.
- **Free trial is a usage credit, not a period** — drawn down by **agent steps and tokens**, with
  a 7-day window also applying. [VENDOR] cursor.com/help/grok-bot/plans
- **Concurrency**: no published cap. Practical limit is the shared machine — one computer, one
  screen per Bot, and a group chat caps at 6.
- **Hard limits that ARE published**: 50 routines/Bot, 20 run records/routine, 6 attachments per
  message, 10-minute teach-a-task recording, 2–6 Bots per group chat.

### 2.4 What the Cursor bundling actually means
1. **Distribution.** Grok Bot instantly inherited Cursor's paying developer base rather than
   starting from zero. That is why "thousands of organizations" is plausible so fast.
2. **Single meter, single bill, single identity.** You sign into Grok Bot with a **Cursor
   account**; SSO, SCIM and audit are Cursor's; the ISO certificates are Anysphere's.
3. **The floor is a developer-tool price.** Non-developers must buy Cursor Ultra to get the
   good tier — the most common launch complaint was about paywall exclusion [PRESS, eesel].
4. **Confusion cost.** Plans changed 2026-08-21 while the FAQ still showed 2026-08-11 tiers;
   Cursor billing staff conceded "The help page was behind. That is on us."
   [USER-REPORT via cellcog / forum.cursor.com]
5. **Strategic:** xAI bought a $60B agent harness and a distribution channel. The "Grok" in
   Grok Bot is the model; everything else is Cursor.

---

## 3. ADOPTION AND RECEPTION  ← the important one

### 3.1 Numbers — thin, and mostly not company-confirmed
- **"Thousands of organizations"** since launch, named customers **Legora, Supermicro,
  ServiceTitan**. [UNVERIFIED — surfaced via aggregators, I could not confirm against an xAI post]
- **"Millions of bots created in the past few weeks"** (September 2026 framing). [UNVERIFIED —
  same caveat. Treat as marketing-origin.]
- **Marketplace: 69 public bots from 43 creators** as of 2026-09-05, from third-party trackers
  (blockchain.news, botmrr.io), explicitly **not** an xAI milestone. [PRESS-analysis] explainx.
  → 69 public bots is a *small* number 3.5 weeks after a marketplace launch. That is the most
  honest adoption signal in this report, and it points at a narrow creator base.
- **Shipping velocity (vendor-sourced, via X)**: in one 5-day window, "~20% faster cold start on
  desktop, up to 35% more effective token usage." [VENDOR-via-X, UNVERIFIED]
- Context: the **Grok chatbot** had ~117M MAU as of March 2026 (tripled from 35M in Dec 2025)
  [UNVERIFIED — stats-farm sites]. Grok Bot is a tiny, expensive subset of that.
- **No waitlist size, no DAU, no retention figure has been published by xAI.** I could not find one.

### 3.2 What real users actually do with it (by observed frequency)
Best single corpus: a curated roundup of 35 documented cases
(https://sidsaladi.substack.com/p/grok-bot-use-cases-35-real-ways-people, partially paywalled)
plus the HN thread and X power-user writeups. The pattern is unambiguous:

**Dominant cluster — "many messy inbound messages → one actionable output, where no API exists."**
The common denominator across successful cases is that *the target system has no API*, so
computer-use is the only route in.

1. **Supplier sourcing & negotiation over messaging apps** — the flagship case. **jjcm** on HN
   (top comment, ~351 points): *"I had one of my bots reach out to ~40 fabric suppliers in
   vietnam, negotiate prices, lock one in, and get samples."* It messaged five mills, heard
   nothing, tried five more, then he pushed it to ~40 total. **The mills answer WhatsApp, not
   email — a logged-in messaging session was the only route in.** He paired it with a
   "prototyper" bot generating custom patterns. [USER-REPORT]
   https://news.ycombinator.com/item?id=49261514
2. **Permits, forms and inspections** — Price Foulger: filled city permit forms, booked an
   inspection *with a captcha*, emailed insurer notifications. [USER-REPORT via sidsaladi]
3. **Freight dispatch** — John Ferguson: 100+ driver texts, load tracking, filing delivery
   documents into a transport system. [USER-REPORT via sidsaladi]
4. **Refund / money recovery** — Darian Shirazi: scanned email for lost refunds and chased
   merchants; **reportedly exceeded the subscription cost**. One of the few positive-ROI claims.
   [USER-REPORT via sidsaladi]
5. **Ad-platform chores via teach-a-task** — Ryan Perry recorded a Meta Ads Manager workflow once
   and the bot replays it on schedule. [USER-REPORT via sidsaladi]
6. **Solo-operator "org charts"** — Eugeniu Ghelbur ran six named bots (CEO, CMO, product,
   engineering, ops, chief of staff); Perry ran six (analytics, creative, engineering,
   infrastructure…); Matt Shumer ran researcher + writer under a chief of staff; another user
   ran seven. [USER-REPORT]
7. **Personal chores** — calendar management, ordering lunch daily, **sniping IMAX 70mm tickets**
   (copied by the community within a day), finding Slack emojis, a teacher's assistant wired to
   a syllabus + digital textbook + Google Classroom posting homework weekly.
   [USER-REPORT via nateherk/mvanhorn X writeups]

**Observation that should shape your product:** the winners are **operators**, not knowledge
workers. Ops beats "thinking work" in every documented success. The moat is *browser session +
persistence*, not model intelligence.

### 3.3 Sticky vs demo-ware
**Sticky**
- **Teach-a-task by screen recording.** The community converged on it in ~48 hours: the advice
  that spread was *don't write a prompt describing the workflow — hit + in the chat and record
  yourself doing it in the browser.* [USER-REPORT] nateherk, "A Week of Grok Bot Lessons"
  https://x.com/nateherk/article/2089917020087210160
- **Persistent logged-in browser sessions on a durable VM.** Unanimously the real unlock.
- **Multi-account connectors** (several Gmails, several Slack workspaces into one bot) — the one
  thing Lenny's guest called "genuinely useful from day one." [PRESS] lennysnewsletter
- **Bot-to-bot handoffs.** An HN user who'd built his own version: *"I've already been doing
  something very similar… but I never managed to get bot-to-bot communication working."*
- **Give the bot its own email address** for account verification — the top community "quick win."

**Demo-ware / didn't stick**
- **Large bot teams.** Everyone documented converges on 5–7 bots; the group-chat cap is 6.
  The "20 bots under a chief of staff" workshop figure is a vendor demo, not observed practice.
- **The autonomy promise.** "Only come back when something needs your approval" collides with
  unbounded token burn and the no-dry-run problem.
- **The marketplace.** 69 bots / 43 creators in 3.5 weeks, no revenue share.
- **Firing bots is common.** One power user: *"Dana Dispatcher was the first bot they fired, as
  it was slow and would make mistakes."* [USER-REPORT via nateherk]

### 3.4 Complaints — ranked by frequency across Cursor forum, Reddit (r/grok, r/cursor,
r/GrokBuild) and HN. Sourcing: https://forum.cursor.com, compiled at
https://cellcog.ai/blog/grok-bot-problems/ (SEO site, but it names users and dates, and the
forum URLs check out).

1. **TOKEN BURN — the #1 complaint, and it came from the product's biggest fan.**
   **jjcm** (HN, top comment): *"Biggest downsides are token expenditure. I've used more tokens
   this month than not this month. That's not a typo - I've used less tokens in the last 5 years
   prior to this month than I have this month. Always on perpetual agents use a LOT of tokens."*
   He concludes it needs a future *"post-ASIC world"* of far cheaper tokens to be sustainable.
   Corroborating quota reports: a business user with six agents burned **42% of the weekly
   allowance on day one** (Reddit, 2026-08-14); another measured **~100 completions + one script
   = 5% of weekly burn**, calling it *"truly terrible"* (Reddit, 2026-08-22); a SuperGrok Heavy
   subscriber burned **roughly half the allowance on a single workload** (2026-08-23/24).
   **And there is no spend cap.**
2. **Reliability — the shared computer gets stuck and takes every Bot with it.**
   **Scott Rocha** (Cursor forum, 2026-08-20/21): *"None of my agents in grok bot work… nothing
   works."* **Daniel Seaton**: persistent "Bot failed to respond" through restarts.
   **Matthew Brown** (2026-08-21/23): black-window lockout. **Cursor staff confirmed the root
   cause: the shared computer entering a stuck state affects all Bots simultaneously.**
   This is the architecture biting, not a transient bug.
3. **Shared-login / no isolation.** A six-agent business tried giving each agent its own Microsoft
   login via separate Chrome profiles; **the profiles reset daily and Chrome crashed frequently.**
   The vendor's own line is the summary: *"do not use separate Bots as a security boundary."*
4. **Logins breaking / bans from third-party sites.** **Tom Zarebczan had a Gmail account disabled
   the same week he created it through Grok Bot; Matt Shumer reported the same a day later.**
   Google flags the activity as bot activity and disables the account without notice; automated
   sending patterns throttle deliverability first, suspend later. [USER-REPORT via
   https://www.agentmail.to/blog/grok-bot-gmail-vs-agentmail] — this is why "give the bot its own
   email" became community doctrine, and why third-party agent-mailbox vendors exist at all.
5. **Unbounded context.** Michael Greenhalgh (Health.AI, 2026-08-13): *"Each Bot is one unbounded
   thread."* Cursor staff confirmed: no fresh session, no manual compaction, no context meter.
6. **Billing/plan confusion.** Plans changed 2026-08-21, FAQ still showed 2026-08-11 tiers;
   Cursor billing: *"The help page was behind. That is on us."*
7. **Paywall.** Most launch complaints were about exclusion from the good tier. [PRESS] eesel.
8. **No dry run, no audit view.** See §1.14.
9. **Low customisation.** Lenny's guest: the platform *"has not given her much to wrestle with"* —
   no model selection, no personality tuning, less room than alternatives. [PRESS]

### 3.5 The societal / second-order critique (HN, unusually strong)
This thread is worth reading in full — it's the most substantive public discussion of the product.
- **pavel_lishin**: *"What happens when fifty people fire off a 15-second 'get me a shirt' prompt?
  When five hundred, five thousand, five million do?"*
- **raincole**: *"The 40 companies will have to use AI to filter the messages too."*
- **zeafoamrun**: *"whenever we put up a job ad it's filled with thousands of LLM generated fake
  candidates in a few days."*
- **solid_fuel**: *"The user without empathy has managed to save 30 minutes on a task they could
  have done themselves anyway. The only cost was wasting the time of at least 39 other people."*
- **dgellow**: *"agents running non stop with access to all your accounts? I would be so anxious
  all the time."*
- **stymaar**, on why claimed injection defences don't help: *"the actual figures says that
  Mythos 'only' falls for prompt injection 2.6% of the times"* — a 2.6% failure rate is
  catastrophic for anything security-critical.
- **thenbrent**, on the SaaS-vs-OSS axis: *"OpenClaw broke every update for a month, so I gave up
  and moved to Hermes"*; Grok offers *"the convenience of SaaS, while the others offer the freedom
  and ownership of open source."*
- **Gormo** invoked mainframe→PC as the precedent for eventual decentralisation.
- All: https://news.ycombinator.com/item?id=49261514

### 3.6 The second HN thread (the launch PR) got flagged
https://news.ycombinator.com/item?id=49261532 — mostly meta-argument about flagging. The one
substantive comment is the important one: **johnnyApplePRNG**: *"Does this SpaceXAI product
silently upload all of your code to a google cloud bucket when you hit enter as well?"* —
a direct reference to the Grok Build incident. **rvz**: *"Because it has an association with Elon
Musk, the audience on HN quickly flags it."* **nozzlegear**: *"I flag all press releases from all
major AI companies. It's all just marketing spam."* Moderator **tuesdaynight** clarified flagging
is automatic.
→ Takeaway: a meaningful slice of the technical audience will not evaluate this product on
merits because of the vendor. That is a distribution fact worth knowing.

---

## 4. TRUST, PRIVACY, SECURITY

### 4.1 Where credentials live
- OAuth connector tokens: **Cursor's backend**, never on the VM, never handed to the model. Good.
- Everything else: **a logged-in browser session on a shared VM**, established by the human
  during a hand-the-computer-over moment. **Every Bot on the account can use it.**
- Net: the blast radius of a single compromised/injected Bot is **every session on the machine**.

### 4.2 Retention & training [VENDOR] docs.x.ai/grok-bot/security, cursor.com/docs/grok-bot/security
- **Privacy Mode**: "customer data is not used for training."
- Training opt-out otherwise "follows the applicable Cursor account and privacy settings" —
  i.e. it is a Cursor setting, not a Grok Bot setting.
- **Zero Data Retention** follows existing provider agreements; "Model providers do not keep
  prompts or outputs."
- **DPA**: data deleted or returned **within 30 days** of written direction after service ends.
- **No per-organization retention policy. No point-in-time restore.**
- Local member files persist indefinitely on durable disk until the machine is reset.
- **Action Recording**: Enterprise only, **off by default**, **90-day retention**.

### 4.3 Compliance
- **Anysphere holds ISO/IEC 27001 and ISO/IEC 42001 (issuer: Schellman), and Grok Bot is in the
  current ISO scope.** [VENDOR] cursor.com/docs/grok-bot/security-faq
- But an independent review of the Grok Bot doc set specifically found **"No SOC 2, ISO 27001,
  GDPR or HIPAA claim anywhere in the doc set"** and scored regulated organisations **3/10**.
  [INDEPENDENT] eesel. These are reconcilable — the ISO claim is in the FAQ, not the main docs —
  but the discrepancy itself tells you the compliance story is immature and inconsistently
  documented.

### 4.4 Prompt injection
- **The docs contain no injection-specific defence.** The stack is: Auto Review (a model judging
  a model) on top, and beneath it *"controls that don't depend on any model's judgment,
  including the network policy, per-action approvals, and per-user isolation."*
  That's honest, and it's also thin.
- **Network policy** is the strongest real control: four modes — No Policy (Allow All),
  Allow All Network Access, **Defaults + Team Allowlist**, **Team Allowlist Only** — covering
  domains and IP ranges with ports, no cap on entries, per-directory-group on Enterprise.
  **Enterprise-gated.** [VENDOR] docs.x.ai/grok-bot/security
- Comparative: a Muse/Grok Bot/Instinct analysis found **no mention of prompt-injection defences**
  for Grok Bot at all, versus Meta's classifiers + human-in-the-loop.
  [PRESS] https://forkast.news/same-blueprint-different-locks-grokbot-and-muse-ship-the-persistent-agent-vm/

### 4.5 Incidents
**Directly attributable to Grok Bot:**
- **Gmail account terminations.** Tom Zarebczan, Matt Shumer — accounts disabled within days of
  being created/driven through Grok Bot. [USER-REPORT]
- **Shared-computer stuck states** taking down all of a user's Bots. [USER-REPORT + staff-confirmed]

**Attributable to xAI's agent line, and relevant to whether you'd trust the vendor:**
- **Grok Build silently uploaded whole repos incl. API keys and secrets to a GCS bucket despite
  "Improve the model" being off** (July 2026, researcher Tinh Dang). Open-sourced under Apache 2.0
  in response. [PRESS] en.wikipedia.org/wiki/Grok_Build
- **Morse-code prompt injection drains ~$150–200k** (May 2026): an attacker sent a Morse-code
  prompt via X that tricked Grok + Bankrbot into transferring 3B DRB tokens from a verified wallet
  on Base. Classified as OWASP LLM01 (prompt injection via encoding) + LLM06 (excessive agency).
  [PRESS/INDEPENDENT] https://oecd.ai/en/incidents/2026-05-04-4a73 ;
  https://neuraltrust.ai/blog/grok-morse-code ; https://www.giskard.ai/knowledge/how-grok-got-prompt-injected-an-x-user-drained-150-000-from-an-ai-wallet
- **"Cryptographic Context Injection"** (Aug 2026, Adversa AI): a web page carries an *encrypted*
  JSON payload plus key material plus a decrypt instruction; asking Grok web chat to summarise the
  page turns into silent exfiltration of name, approximate location, subscription tier and active
  conversation history. **Zero-click.** [PRESS/INDEPENDENT]
  https://www.theregister.com/ai-and-ml/2026/08/20/grok-chat-duped-into-swallowing-injected-instructions/5290019 ;
  https://gbhackers.com/zero-click-grok-attack-prompt-injection/
  → Encoding-layer injection defeats plaintext classifiers. This is the attack class that a
  browser-driving, always-on, all-your-sessions-on-one-box agent is worst positioned against.
- Model-level reputational incidents (MechaHitler antisemitic spree July 2025; "white genocide"
  injection May 2025; sexual deepfakes from Dec 2025 → lawsuits and regulator threats; regional
  blocks by **Turkey, Indonesia, Malaysia**, Philippines child-safety action). [PRESS]
  These are not Grok Bot, but they are the vendor's risk record and they drive the HN flagging.
- **Off-label harm anecdote** cited in a review: "A farmer killed nearly 25 acres of his own crop
  after listening to an AI app." [PRESS, second-hand] bensbites — not Grok Bot specifically;
  included only as the reviewer's framing of blind-reliance risk. **[UNVERIFIED as Grok Bot.]**

### 4.6 Enterprise controls (all Enterprise-tier)
SSO via Cursor account; **SCIM 2.0** provisioning/deprovisioning; team-wide Auto Review rules with
an enforcement lock (personal rules may only tighten); plugin blocking via Team Marketplace
connector policy (shows "Disabled by team admin"); **Team Setup manifests** (install scripts on
every team computer); network policy allowlists; audit logs streamable to SIEM; ability to restrict
local-computer execution; Action Recording; Privacy Mode.
**Not available at any tier:** on-prem, own perimeter, BYO image, dedicated egress IPs,
per-org retention policy, point-in-time restore, spend caps, per-bot credential isolation, dry run.

---

## 5. COMPETITIVE CONTEXT
*(weakest section — search budget ran out. Treat as directional.)*

**The category converged on one blueprint within a month: the persistent agent VM.**
Reworked's framing: *"It's the same basic model as Anthropic's Claude Cowork, OpenAI's ChatGPT
Work and Microsoft's Copilot Cowork: you give a bot a job, hand it access to your apps and it
runs until it needs your sign-off."* [PRESS] reworked.co

**Named competitors appearing in comparison sets** (from an alternatives roundup [UNVERIFIED —
vellum.ai]): ChatGPT Work, Claude Cowork, **Gemini Spark**, **Perplexity Computer**, Manus,
OpenClaw, TrustClaw, Martin, Cora, Vellum. Meta's **Muse** launched 2026-09-08. A third product,
**Instinct**, appears in three-way comparisons.

**Grok Bot vs Meta Muse** — the best-documented comparison, and it is not flattering:
[PRESS] https://forkast.news/same-blueprint-different-locks-grokbot-and-muse-ship-the-persistent-agent-vm/
and https://porteden.com/blog/muse-vs-grok-bot-vs-instinct/

| | Grok Bot | Muse |
|---|---|---|
| VM granularity | one shared computer **per account**, all bots share it | **dedicated isolated Linux VM per user**, systemd-nspawn containers |
| Isolation boundary | **account-level** (Firecracker between accounts only) | **action-level**, via "Sentinel", "the sole permission authority for all connector actions and network egress" |
| Credentials | browser sessions shared across bots; vendor warns not to place a credential another bot shouldn't use | secure storage **outside the agent runtime**, just-in-time insertion at the network boundary — **"the agent never sees real tokens"** |
| Permission scoping | connectors are **account-wide** | can narrow grants **below OAuth scopes** (e.g. strip Gmail settings from a read grant) |
| Injection defence | none documented | classifiers + human-in-the-loop |
| Activity log | Action Recording, **Enterprise-only, off by default**, 90-day | in-product views, no documented export |
| Surfaces | desktop + iOS/iPad/Android | iOS, Android, web (muse.ai), **inside WhatsApp** |
| Positioning | persistent named teammates, business apps, shared cloud computer | personal tasks — email, forms, travel booking |

Forkast's verdict: *"Muse is the most architecturally careful of the three major agents, with Meta
building a real permission boundary and publishing a detailed engineering write-up about it,"*
while *"The GrokBot model treats the agent as a user-proxy, prioritizing ease of coordination"* —
and a security-first approach *"is likely the standard for enterprise environments."*

**Where Grok Bot wins:** coordination fluidity and shared-state workflows (handoffs need no
re-auth); teach-by-demonstration (neither Muse nor the open-source projects have it); multi-account
connectors; zero-setup polish; sheer distribution via Cursor.
**Where it loses:** every security axis; cost transparency; auditability; channel breadth;
customisation; model choice; and it has no free tier when Muse is free and US-wide.

**Versus open source.** The HN framing is the honest one — Grok offers *"the convenience of SaaS,
while the others offer the freedom and ownership of open source"*, and the counterweight complaint
is *"OpenClaw broke every update for a month."* **Reliability and polish, not capability, is what
the open-source side is losing on.**

---

## 6. WHAT IS MISSING — the local-first / cost-control shopping list

This is the part with the most direct product implication. Everything below is a *demonstrated*
gap, with the source that shows it.

1. **A spend cap.** No Grok Bot spend limit exists. The loudest complaint in the entire corpus is
   token burn, from the product's own top advocate. Nobody can bound the cost of an always-on agent.
2. **Per-bot credential isolation.** Users tried to build it themselves with separate Chrome
   profiles and failed (profiles reset daily, Chrome crashed). The vendor tells you not to try:
   *"do not use separate Bots as a security boundary."*
3. **Local / self-hosted deployment.** Refused outright in the docs. The demand is visible in the
   **eight-plus open-source clones that appeared within weeks**, all pitching the same three
   words — your hardware, your credentials, your roof:
   - **OpenGrokBot** — "always-on AI teammates, each with its own computer — running on your
     hardware, with your credentials, under your roof." One gateway process + **one Docker
     container per bot**, any OpenAI-compatible model (BYOK). Replicates threads, persistent
     memory, **approval workflows (outbound actions held for review)**, bot-to-bot handoffs,
     per-bot browser/filesystem/terminal, **screenshots embedded in threads as evidence**.
     Admits it lacks "zero-setup polish, iOS app, and teach-by-demonstration."
     **24 stars, 8 forks** — i.e. the demand is loud but the supply is embryonic.
     https://github.com/wolfqing/OpenGrokBot
   - **OpenBot** — "free, open-source, local-first… turns a task into bounded, auditable work on
     the operator's machine. No account, paid service, GPU, or hosted agent."
     https://github.com/pmwasim/OpenBot
   - **Rakazo** (elie222) — "Choose your own model and sandbox." https://github.com/elie222/rakazo
   - **OpenMausBot** — bots each with "their own personality, memory, model, computer, and apps",
     running on the **claude, codex and grok CLIs installed on your own machine**.
     https://github.com/milind-soni/OpenMausBot
   - **OpenClaw** — local-first personal agent: channels, memory files, heartbeats, cron jobs,
     browser tools, deep configuration.
   - Directories: https://openalternative.co/alternatives/grok-bot ,
     https://www.opensourcealternatives.to/alternative-to/grok-bot
   → **Note the convergent feature set of the clones: per-bot container, BYO model, approvals on
   outbound actions, screenshot evidence in-thread. That is the market telling you what a
   local-first agent runtime is supposed to be.**
4. **Bring-your-own-model / model choice.** None. Auto-selected, no override, no advanced mode.
   Called out by Lenny's guest as the main constraint and by every alternatives roundup.
5. **A dry run / rehearsal mode.** "A test run performs real work." Cited as the single biggest
   functional gap by the most rigorous review I found.
6. **A real action audit trail.** Vendor: *"An audit view of Bot actions is coming."* Today the
   audit log covers control-plane events only, Action Recording is Enterprise/off/90-day, and
   routines keep 20 runs. For an agent acting in your bank and your inbox this is not enough.
7. **User-visible, editable memory.** Undocumented. Combined with "each Bot is one unbounded
   thread" (no fresh session, no compaction, no context meter), memory is a black box that also
   costs money.
8. **Inbound messaging / an API.** No supported way to message a Bot from outside. A user had to
   run **a Mac Mini with a LaunchAgent** to bridge Slack in — an unintentionally perfect
   advertisement for a local daemon.
9. **"Multiplayer."** Per a power-user survey, **the most-requested missing feature is multiplayer
   — shared groceries and social plans with a partner.** [USER-REPORT] nateherk/mvanhorn.
10. **Voice.** Absent entirely.
11. **Network identity.** Shared datacenter egress IPs with shared reputation, no dedicated IPs.
    The vendor's own fix is to route through the user's desktop.
12. **Not getting your accounts banned.** Gmail terminations are a recurring, named, dated
    user-reported failure mode. A cloud datacenter IP driving your Gmail is the proximate cause.

**The through-line:** almost every item on this list is *structurally* solved by running the agent
on the user's own machine — network identity, credential locality, spend visibility, audit,
isolation, account-ban risk, and inbound messaging. Grok Bot's residual advantages are polish,
teach-by-demonstration, multi-account connectors, and 24/7 uptime while the laptop is shut.
**Uptime is the only one that genuinely requires someone else's computer — and a Mac mini that is
always on neutralises it.**

---

## 7. ROADMAP SIGNALS

- **Grok 4.7** — 3–4 weeks out as of late Aug 2026, "trained on SpaceX engineering data for
  real-world problem-solving." [PRESS] bensbites. Trending-topic corroboration:
  "Grok 4.7 Nears Release with Major Bot Platform Upgrades" [UNVERIFIED — X trend]
- **Marketplace phase 2 — third-party Bots, not just plugins.** Verbatim leak/announcement:
  *"Grok Bot is getting a Marketplace. Your one stop shop for Plugins and Bots. Plugins is the
  catalog we already have. Bots is what's coming: Third-party Grok Bots you can add straight to
  your team. Pick one, review the instructions (and memories, skills, routines, integrations if
  it has…"* [USER-REPORT/leak] https://x.com/blankspeaker/status/2093399672803176787
- **"An audit view of Bot actions is coming."** [VENDOR, quoted in eesel review]
- **Enterprise waitlist** from launch → Enterprise shipped 2026-09-03. Largely delivered.
- Shipped-fast cadence, vendor-sourced: ~20% faster desktop cold start, up to 35% more effective
  token usage in a 5-day window. [VENDOR-via-X, UNVERIFIED]
- **Not announced, despite demand:** spend caps, dry run, per-bot isolation, model selection,
  local/on-prem, voice, multiplayer, an inbound API. The docs say on-prem is "not supported
  **today**", which is the only hedge.
- Broader xAI roadmap (not Grok Bot): AI-generated video game before end of 2026, humanoid AI
  "Macrohard", Grokipedia. [PRESS] en.wikipedia.org/wiki/SpaceXAI

---

## APPENDIX — source URLs used

**Primary / vendor**
- https://x.ai/news/introducing-grok-bot
- https://x.ai/bot (403 to automated fetch — note the irony)
- https://x.ai/bot/marketplace
- https://docs.x.ai/grok-bot
- https://docs.x.ai/grok-bot/approvals-security-and-privacy
- https://docs.x.ai/grok-bot/security
- https://docs.x.ai/grok-bot/security-faq
- https://docs.x.ai/grok-bot/private-networks
- https://docs.x.ai/grok-bot/teams-and-enterprises
- https://cursor.com/docs/grok-bot
- https://cursor.com/docs/grok-bot/get-started
- https://cursor.com/docs/grok-bot/work
- https://cursor.com/docs/grok-bot/security
- https://cursor.com/docs/grok-bot/security-faq
- https://cursor.com/docs/grok-bot/identity
- https://cursor.com/docs/grok-bot/teams
- https://cursor.com/docs/grok-bot/private-networks
- https://cursor.com/help/grok-bot/plans
- https://docs.customer.io/ai/plugins/cursor-grok-bot/ (third-party vendor plugin doc)

**User threads**
- https://news.ycombinator.com/item?id=49261514 (main launch thread, jjcm top comment)
- https://news.ycombinator.com/item?id=49261532 (flagged PR thread)
- https://forum.cursor.com/t/grok-bot-can-i-send-it-a-message-from-outside/168199/15
- https://x.com/nateherk/article/2089917020087210160
- https://x.com/mvanhorn/article/2087941850456076696
- https://x.com/i/article/2092216029816472002 (kocer, chief-of-staff charter)
- https://x.com/johnwhereareu/article/2092952573506654427
- https://x.com/blankspeaker/status/2093399672803176787
- https://sidsaladi.substack.com/p/grok-bot-use-cases-35-real-ways-people
- https://aimaker.substack.com/p/grok-bot-ai-agent-team
- https://helenaai.substack.com/p/grok-bot-vms-ips-setup-prompts

**Press / analysis**
- https://www.reworked.co/collaboration-productivity/xai-launches-grok-bot-ai-agents-in-beta/
- https://www.vktr.com/collaboration-productivity/xai-launches-grok-bot-ai-agents-in-beta/
- https://www.bensbites.com/p/grok-bot-is-not-what-you-think
- https://www.lennysnewsletter.com/p/how-i-ai-grok-bot-grok-46whats-great
- https://www.eesel.ai/blog/grok-bot-review
- https://www.eesel.ai/blog/grok-bot-pricing
- https://forkast.news/same-blueprint-different-locks-grokbot-and-muse-ship-the-persistent-agent-vm/
- https://porteden.com/blog/muse-vs-grok-bot-vs-instinct/
- https://www.explainx.ai/blog/spacexai-grok-bot-marketplace-launch-2026
- https://www.testingcatalog.com/grok-bot-to-get-templates-and-multi-account-support-soon/
- https://en.wikipedia.org/wiki/SpaceXAI
- https://en.wikipedia.org/wiki/Grok_Build
- https://releasebot.io/updates/xai

**Security**
- https://oecd.ai/en/incidents/2026-05-04-4a73
- https://neuraltrust.ai/blog/grok-morse-code
- https://www.giskard.ai/knowledge/how-grok-got-prompt-injected-an-x-user-drained-150-000-from-an-ai-wallet
- https://www.theregister.com/ai-and-ml/2026/08/20/grok-chat-duped-into-swallowing-injected-instructions/5290019
- https://gbhackers.com/zero-click-grok-attack-prompt-injection/
- https://www.agentmail.to/blog/grok-bot-gmail-vs-agentmail

**Open-source alternatives**
- https://github.com/wolfqing/OpenGrokBot
- https://github.com/pmwasim/OpenBot
- https://github.com/elie222/rakazo
- https://github.com/milind-soni/OpenMausBot
- https://openalternative.co/alternatives/grok-bot
- https://www.opensourcealternatives.to/alternative-to/grok-bot
- https://www.vellum.ai/blog/best-grok-bot-alternatives
- https://github.com/rdmgator12/awesome-grok-connectors
- https://github.com/ZeroPointRepo/awesome-grok-bot

**Aggregators used only to find primaries (treat as [UNVERIFIED])**
cellcog.ai/blog/grok-bot-problems/ (names users + dates, forum URLs check out),
news.aibase.com/news/30628, layer3labs.io, aitoolsreview.co.uk, kie.ai, netalith.com,
ayautomate.com, coursiv.io, callmissed.com, atomicbot.ai, zentor.ai, aipricing.guru,
aibuilderclub.com, 4geeks.com, basenor.com, mindstudio.ai, composio.dev, roasbeast.com,
botdirectory.ai, bot.store/academy.
