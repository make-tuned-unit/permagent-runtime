# Cross-cutting market research: personal / always-on agents, Sept 2026

Research date: 2026-09-20. All web content treated as untrusted data.
Provenance tags: [DATA] = primary numeric source with method; [PRESS] = journalism;
[VENDOR] = company claim; [USER-REPORT] = forum/review/hands-on; [UNVERIFIED] = single
low-quality source or SEO content, treat as rumour.

IMPORTANT CAVEAT ON SOURCE QUALITY: a large share of the 2026 agent coverage surfaced by
search is SEO explainer content of unknown reliability ("presenc.ai/research",
"allaboutai.com", "digitalapplied.com", etc.). Numbers coming only from those are tagged
[UNVERIFIED] and should not be relied on. Where only a vendor says it, it is [VENDOR].

---

## 1. THE LANDSCAPE (Sept 2026)

### xAI — Grok Bot
- Launched 2026-08-11 in beta. "A team of always-on AI agents, each running on its own
  cloud computer," signs into your tools, runs multi-step jobs unsupervised, keeps running
  after the app closes, returns only when it needs approval. [VENDOR]
  https://x.ai/news/introducing-grok-bot
- Teach-a-Task: the bot watches you do a workflow, "saves your workflow as a routine, takes
  your corrections, and runs it on its own next time." [VENDOR] (same URL)
- Multi-agent: bots message each other, share context, coordinate in group chats, one can
  manage others. [VENDOR] (same URL)
- Price: bundled into SuperGrok / SuperGrok Plus / SuperGrok Heavy and Cursor Pro/Pro+/Ultra
  and Cursor Teams tiers, with separate usage limits. Desktop + iOS. Enterprise waitlist.
  [VENDOR] (same URL)
  NOTE: the xAI page listing *Cursor* tiers as an access path is surprising and I could not
  independently corroborate an xAI/Cursor relationship — treat that detail as [UNVERIFIED].
- Secondary coverage: https://www.unite.ai/xai-launches-grok-bot-always-on-ai-teammates-with-their-own-cloud-computers/ [PRESS]
  https://650group.com/blog/grok-bot-always-on-agent-compute/ [PRESS, infrastructure angle]

### Meta — Muse
- Launched 2026-09-08. Personal agent that "can open a browser, fill out forms, and
  negotiate on their behalf," continues working after you close the app, remembers
  preferences, makes unprompted suggestions. [VENDOR]
  https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/
- Runs on "Muse Secure VM", a dedicated cloud VM. A separate **Sentinel** agent runs at the
  system level and prevents any action reaching the internet without approval. [VENDOR]
- **Muse Confidential VM** promised "later this year": whole VM incl. data and conversations
  encrypted with a key only the user holds, so not even Meta can access it. [VENDOR]
  — this is Meta conceding the privacy objection. Not shipped as of 2026-09-20.
- Muse has no visibility into passwords or payment methods. [VENDOR]
- Surfaces: Muse app (iOS/Android), muse.ai, **directly in WhatsApp**, "coming soon to AI
  glasses". US only at launch. [VENDOR]
- Pricing: free tier plus paid plans; press reports ~$20/mo and ~$100/mo power tiers. [PRESS]
  https://www.axios.com/2026/09/08/meta-debuts-muse-personal-ai-agent
  (Axios returns HTTP 403 to automated fetch; tier figures are as relayed in search result
  snippets — treat exact dollar figures as [PRESS]/[UNVERIFIED] until confirmed.)

### OpenAI — ChatGPT Work (Operator → agent mode → Work)
- Operator launched 2025-01-23, Pro-only research preview 2025-02-01; absorbed into ChatGPT
  as "agent mode" July 2025. https://en.wikipedia.org/wiki/OpenAI_Operator [PRESS]
- **ChatGPT agent mode was removed from ChatGPT in early August 2026 without advance
  deprecation notice**; help centre redirects users to ChatGPT Work and a separate cloud
  browser feature. [UNVERIFIED — sourced from SEO trackers, needs primary confirmation]
  https://presenc.ai/research/openai-operator-update-tracker-2026
  https://www.izzedo.chat/blog/ai-agent-mode
  This is a notable signal if true: the first-generation consumer computer-use agent was
  killed by its own vendor ~18 months in.
- **ChatGPT Work**, launched 2026-07-09: an agent inside ChatGPT that gathers context across
  apps, decomposes goals, and returns finished sheets, slides, docs and web apps. The Codex
  app merged into a new ChatGPT desktop app putting Chat, Work and Codex on every plan
  including Free. [UNVERIFIED/PRESS]
  https://www.digitalapplied.com/blog/chatgpt-work-openai-agent-launch-2026
- ChatGPT folded proactive briefings into **Scheduled Tasks** rather than a separate
  proactive product. [PRESS] https://aiagentstore.ai/ai-agent-news/this-week
- There are references in Sept 2026 press to an OpenAI consumer assistant called
  "Instinct" (in the Axios URL slug
  /2026/09/20/ai-assistant-openai-meta-muse-instinct-grok-apple) — **I could not verify what
  Instinct is**; search budget ran out and Axios blocks automated fetch. FLAGGED AS A GAP.

### Google — Gemini Spark (Project Mariner retired)
- **Project Mariner is shut down** after ~17 months; its browser-control work folded into
  Gemini and Chrome. [PRESS]
  https://www.androidheadlines.com/2026/05/google-shuts-down-project-mariner-ai-agent.html
- **Gemini Spark** announced at I/O 2026, rolled out to US Google AI Ultra subscribers
  2026-05-29. Runs in the cloud — "your phone can be locked, your laptop closed, or the
  Wi-Fi off — Spark keeps going." [PRESS] https://9to5google.com/2026/05/29/gemini-spark-ultra-us/
- Architecture worth copying: three primitives — **Tasks** (high-level goals), **Schedules**
  (timing/triggers), **Skills** (reusable instruction sets invoked with @ or /). Remote
  browser with saved logins; remote computer with code execution. Up to **15 concurrent
  tasks**. Web + Android + iOS (mobile beta). [PRESS] (same URL)
- A dedicated **Gmail address you email to assign tasks**, and "Android Halo" for
  at-a-glance agent visibility. [UNVERIFIED — secondary sources only]
- Price: Google AI Ultra. Ultra restructured at I/O 2026 from a single $249.99 tier into two,
  with a **$100/mo** mid-band as the floor for Spark. [UNVERIFIED — secondary sources]

### Microsoft — Scout (and Copilot agents)
- Announced 2026-06-02 at Build 2026 as Microsoft's first **"Autopilot"** agent — a new
  category Microsoft defines as "always-on agents that work autonomously, with their own
  identity, and act on your behalf." [VENDOR]
  https://www.microsoft.com/en-us/copilot/blog/2026/06/02/introducing-microsoft-scout-your-always-on-personal-agent/
- **Scout is built on OpenClaw**, the open-source agent framework. Multiple independent
  outlets confirm. [PRESS]
  https://www.infoq.com/news/2026/06/microsoft-scout-openclaw-build/
  https://www.computerworld.com/article/4180103/microsoft-unveils-scout-an-autonomous-ai-agent-built-on-openclaw.html
  https://decrypt.co/369781/microsoft-scout-openclaw-enterprise-ai-agent
  https://www.reworked.co/digital-workplace/microsoft-scout-is-built-on-openclaw/
- Reworked headline: "Satya Nadella Called OpenClaw a 'Virus.' 3 Months Later, Microsoft
  Built Scout on It." [PRESS]
- Microsoft is contributing **policy conformance upstream to OpenClaw** so orgs running
  OpenClaw themselves can validate their configuration against security/compliance
  requirements and get an audit-ready answer. [PRESS] (InfoQ)
  >>> STRATEGIC READ: the largest enterprise software vendor validated the *self-hostable
  open-source personal agent* as the substrate, and is improving it for people who run it
  themselves. This is the single strongest datapoint for the local-first thesis.
- Status: private preview, "Frontier" organisations only; requires Frontier enrolment, Intune
  policy config, a GitHub Copilot licence, and user opt-in attestation. Runs across cloud,
  desktop and web; primary surface is Teams; connects Teams/Outlook/OneDrive/SharePoint.
  Each agent gets its **own Entra identity**. [VENDOR]
- Capabilities are notably *coordination*, not computer-use: schedules meetings across time
  zones, flags important meetings and preps materials, blocks calendar time for deliverables,
  spots stalled decisions. Builds context via "Work IQ". [VENDOR]

### Apple — Siri AI (iOS 27)
- iOS 27 / iPadOS 27 / macOS "Golden Gate" / visionOS 27 / watchOS 27 ship **September 2026**
  with "Siri AI": personal context, world knowledge, onscreen awareness. Developer builds
  shipped June 2026. [PRESS] https://en.wikipedia.org/wiki/Siri
  https://medium.com/@taoufiq.moutaouakil/apple-intelligence-siri-in-2026-fe509d8813fd
- **SiriKit deprecated at WWDC 2026; App Intents is now the only way Siri can call into a
  third-party app**, with a 2–3 year support window. Siri AI can reach into an app, retrieve
  entities and invoke actions without the user opening the app. [PRESS/VENDOR]
  https://developer.apple.com/videos/play/wwdc2026/240/ (App Schemas)
  https://developer.apple.com/videos/play/wwdc2026/343/ (advanced App Intents)
  https://ecorpit.com/ios-27-app-store-ai-agents-app-intents-developer-strategy-2026/
- Any app can use the on-device Apple Intelligence Foundation Models, and **those features
  work offline**. [VENDOR] https://www.apple.com/apple-intelligence/
- Security researchers already treating App Intents as a new attack surface. [PRESS]
  https://www.nowsecure.com/blog/2026/08/05/what-appsec-teams-need-to-know-about-app-intents-siri-ai-and-the-new-ios-27-attack-surface/
  >>> STRATEGIC READ: Apple's agent is the *only* major one that is on-device-first and
  action-oriented via a structured app-action graph rather than pixel-level computer use.
  A local-first product on Mac/iOS/watch should expose App Intents (both directions).

### Amazon — Alexa+
- Alexa+ generally available in the US 2026-02-04; **free to Prime members, $19.99/mo for
  non-Prime**. [PRESS] https://techcrunch.com/2026/02/04/alexa-amazons-ai-assistant-is-now-available-to-everyone-in-the-u-s/
- Alexa reached the web at alexa.com with a revamped app, Jan 2026. [PRESS]
  https://techcrunch.com/2026/01/05/alexa-without-an-echo-amazons-ai-chatbot-comes-to-the-web-and-a-revamped-alexa-app/
- >1M users in the 2025 beta. [PRESS] Amazon has **not** disclosed adoption numbers for the
  agentic (autonomous task) use case. [PRESS]
  https://www.retaildive.com/news/amazon-alexa-for-shopping-agentic-ai-adoption/826806/
- Shopping numbers Amazon *has* disclosed: >350M shoppers used Alexa for Shopping in the past
  12 months; active users nearly doubled YoY in Q2; interactions up 5x YoY; US customers
  using it spend ~40% more per order. [VENDOR, relayed by PRESS] (Retail Dive / CX Dive)
  https://www.customerexperiencedive.com/news/amazon-customers-embracing-alexa-for-shopping/826734/
  CAUTION: "used Alexa for Shopping" is a very loose denominator (any shopping query on any
  Echo). Do not read this as agentic task adoption.

### Anthropic — Claude Cowork / computer use / Claude in Chrome
- **Claude Cowork** announced 2026-01-12 as a research preview: a "computer agent" for
  non-technical users, Claude Code's engine pointed at general desktop work, inside the macOS
  Claude desktop app, given access to a specific folder. [PRESS]
  https://venturebeat.com/technology/anthropic-launches-cowork-a-claude-desktop-agent-that-works-in-your-files-no
- **GA 2026-04-09** with six enterprise features, all paying subscribers, macOS and Windows.
  Same day as Managed Agents. [PRESS]
  https://pasqualepillitteri.it/en/news/755/anthropic-managed-agents-cowork-ga-april-9-2026
- Expanded to **web and mobile for Max subscribers in July 2026**. [PRESS]
  https://techcrunch.com/2026/07/07/the-coding-agent-wars-are-spilling-into-the-rest-of-the-office-claude-cowork/
  >>> STRATEGIC READ: Cowork is the closest competitor in shape to a local-first agent — it
  runs on *your* machine against *your* files rather than a cloud VM. Anthropic is the one
  frontier lab that did not choose the cloud-VM architecture for its consumer agent.

### Perplexity — Comet
- Launched July 2025 at $200/mo; **paywall dropped 2026-03-18**, free on iOS, Android,
  Windows, Mac. Free iOS version hit #3 on the US App Store within 48 hours. [PRESS]
  https://www.perplexity.ai/hub/blog/comet-is-now-available-to-everyone-worldwide [VENDOR]
  https://en.wikipedia.org/wiki/Comet_(browser) [PRESS]
- Claimed ~18M MAU in Q1 2026 (up from 2.4M a year earlier); ~480M queries/mo; **~14M agentic
  tasks/mo**; 92M inline page summaries. [UNVERIFIED — single SEO source, presenc.ai; no
  methodology; do not cite as fact]
  https://presenc.ai/research/comet-usage-statistics-2026
  Note the ratio if even roughly right: ~480M queries vs ~14M agentic tasks = agentic use is
  ~3% of Comet activity. That is the honest shape of "agent" demand inside an AI browser.
- Perplexity sells **Background Assistants** on its top tier. [PRESS]
  https://aiagentstore.ai/ai-agent-news/this-week

### Salesforce — Agentforce named agents
- 2026-09-11: seven **named** agents — Casey, Paige, Carter, Hunter, Marshall, Piper, Fin —
  each for a business function. [PRESS] https://aiagentstore.ai/ai-agent-news/this-week
  >>> Named-persona agents are now a mainstream enterprise pattern, not a quirk.

### Open source / self-hosted
- **OpenClaw**: created by Peter Steinberger, launched Nov 2025; hit **180,000 GitHub stars
  three months after its January 2026 launch** [note the source's own date inconsistency
  between Nov 2025 and Jan 2026 — treat launch date as uncertain]. Steinberger joined OpenAI
  in Feb 2026 and an **OpenClaw Foundation** was set up to steward the project. [PRESS]
  https://www.infoq.com/news/2026/06/microsoft-scout-openclaw-build/
- **QwenPaw** (formerly CoPaw, rebranded April 2026): open-source personal assistant on the
  Alibaba/Qwen stack, multi-agent, with channel support for DingTalk, Feishu, WeChat, QQ,
  Discord, **iMessage**, Telegram, Matrix, Mattermost. [UNVERIFIED — listicle source]
  https://www.vellum.ai/blog/best-open-source-personal-ai-assistants
- Directory/listicle sources for the wider field (low reliability, use only for names):
  https://www.agent37.com/blog/personal-ai-assistant
  https://gravity.fast/blog/best-personal-ai-agents/

### The structural split (useful framing, [PRESS])
> "2026 quietly split this market into two different products: chat apps you visit, and
> agents that work for you around the clock." — https://aiagentstore.ai/ai-agent-news/this-week
Every major vendor shipped an always-on lane in 2026: ChatGPT Scheduled Tasks, Google Daily
Brief + Spark, Perplexity Background Assistants, Microsoft Scout, Grok Bot, Meta Muse.

---

## 2. ADOPTION DATA

### THE ONE GOOD SURVEY: Menlo Ventures "State of Consumer AI 2026"
Published 2026-09-16, fielded with Morning Consult, **N = 5,067 US adults, July 2026,
nationally representative weighted sample**. [DATA — best-sourced primary consumer survey found]
https://menlovc.com/perspective/2026-the-state-of-consumer-ai/
- Consumer AI reached **2 billion users** in 2026 (~24% of global population). User base grew
  only **11% YoY while spend more than tripled, $12B → $40B**.
  >>> The market is monetising existing users, not acquiring new ones. Depth, not reach.
- 55% of AI users pay for ≥1 AI product. The top 14% of payers ($100+/mo) = **60% of all
  consumer AI spend**.
- **72% aware of agents; 41% have tried an agent; 24% use one regularly.**
  >>> Try→regular conversion is roughly **59%** among triers (24/41). That is the closest
  thing to a retention figure that exists. It is self-reported frequency, NOT cohort retention.
- **92% of agent users pay for AI, vs 55% of all AI users.** 63% of agent users use AI daily
  vs 39% of all AI users. 65% of AI payers use agents vs 13% of non-payers. 21% of regular
  agent-user payers spend $100+/mo vs 5% of non-agent payers.
  >>> AGENT USERS ARE THE HIGH-VALUE COHORT. This is the most commercially important finding
  in the whole report: agents select for people who already pay and pay a lot.
- **32% of AI users have let AI act without final approval at least once.** (So 68% have not.)
- **Permission grants to agents**: email 36%, browsers 33%, messaging 31%, cloud storage 29%,
  calendars 27%, health apps 23%, **financial accounts 20%**.
  >>> Ranked demand order for integrations: email > browser > messaging > files > calendar.
- Trust drivers, ranked: **accuracy (45%) > trustworthiness (40%) > security/privacy (36%) >
  ease of use**.
  >>> PRIVACY IS THIRD. Accuracy is first. A local-first product that leads on privacy is
  leading with the market's #3 concern. Lead with reliability; privacy is the support act.

### Delegation willingness
- Blackbox Research "Delegation Contract": 74% would let an agent handle routine tasks (deal
  negotiation, complaint resolution, subscription renewals, reorders); **only 9% would let an
  agent initiate AND complete a purchase unsupervised**; 27% trust no organisation to run an
  AI shopping agent; 24% say they will never delegate purchases to AI. [DATA — sample size and
  field dates NOT verified] https://www.blackbox.com.sg/perspectives/the-delegation-contract-how-consumers-structure-trust-in-ai-agents

### Per-product numbers
- **ChatGPT**: 1B WAU Aug 2026 (from 900M Feb 2026, 400M a year earlier). [VENDOR/PRESS]
  https://memeburn.com/chatgpt-weekly-active-users-near-1-billion/
- **OpenAI agentic tools (Codex + ChatGPT Work combined): 10M WAU by 2026-07-21**, <2 weeks
  after ChatGPT Work debut. The outlet explicitly flags it as "a company estimate rather than
  an audited metric." Codex trajectory: 5M WAU end-May → 6M Jul 12 → ~8M mid-Jul; >1M of those
  use Codex outside software development. [VENDOR + PRESS caveat]
  https://thenextweb.com/news/openai-agents-10-million-chatgpt-work
  >>> 10M agentic WAU against 1B ChatGPT WAU = **~1% of ChatGPT's base uses the agent tools.**
  That is the honest size of agent demand inside the biggest consumer AI product on earth.
- **Sensor Tower State of AI 2026** [DATA]: ChatGPT ~215 min/user/month; ChatGPT's share of
  AI-assistant category time fell below 50% for the first time in March 2026. Gemini engagement
  grew 14→100 min/user/month, ~30% category time share Q1 2026. Claude "True Audience" +452%
  YoY (May 2026), US AI-assistant share 4.4%→~14%, ~120 min/user/month.
  https://sensortower.com/blog/state-of-ai-2026
- **Meta Muse**: ~83,000 US iOS downloads since launch (Sensor Tower), excluding web, WhatsApp
  and Android. The "#2 app in the US" headline actually means #2 **in Productivity**, not
  overall. On Android it ranks **#338** in Play Store Productivity. Pricing: free + Power
  $20/mo + Maximum $100/mo, and **Meta itself expects most users to stay free**. [DATA/VENDOR]
  https://techcrunch.com/2026/09/08/meta-debuts-its-muse-ai-agent-will-consumers-trust-it/
  >>> 83k iOS downloads for a Meta launch with global press is a weak start. Worth watching.
- **Gemini**: >900M MAU May 2026 (350M Apr-2025 → 650M Q3-25 → 750M Q4-25 → 900M+ May-26).
  Gemini Enterprise >8M paid seats, 2,800+ companies, paid MAU +40% QoQ Q1 2026. [VENDOR]
  No Spark-specific number exists.
- **Manus**: ~$450M annualised revenue run-rate by June 2026 (from ~$90M Aug 2025, ~$125M Dec
  2025) — intermediate figures inconsistent across sources, treat trajectory as directional.
  147T tokens processed, 80M "virtual computers" created, 2M waitlist signups in week one.
  $19–$199/mo. [VENDOR/PRESS] https://sacra.com/c/manus/
  China blocked Meta's proposed $2B acquisition of Manus in late April 2026. [PRESS]
- **Genspark**: $250M annualised revenue by March 2026 (from $85M end-2025) — +$150M ARR in Q1
  2026 alone. >2M MAU, ~100k paying seats at $30/user/mo [UNVERIFIED aggregator].
  $2.6B valuation, $485M raised (Series B extension 2026-06-17). [PRESS]
  https://www.businesswire.com/news/home/20260617758937/en/
  >>> Manus and Genspark are the two clearest commercial successes in general-purpose consumer
  agents, and BOTH are cloud-VM "give it a goal, it works for an hour" products.
- **Claude Cowork**: research preview 2026-01-12 → Windows Feb → GA 2026-04-09 → **merged back
  into the unified Claude chat interface 2026-09-16; Cowork as a separate surface is
  effectively discontinued**, alongside the launch of Claude Docs and Claude Slides. [PRESS]
  https://techcrunch.com/2026/09/16/anthropic-merges-claude-chat-and-cowork-in-one-interface/
  https://venturebeat.com/technology/anthropic-is-killing-off-cowork-and-folding-it-into-claude-launching-claude-docs-and-claude-slides
  >>> MAJOR: the separate-agent-surface pattern failed at Anthropic too. Both OpenAI (agent
  mode → killed) and Anthropic (Cowork → folded in) concluded the agent is a MODE of the
  assistant, not a separate product. Do not build a separate "agent app" beside the chat app.
- Claude in Chrome: open to all Max and Team subscribers; computer use still "research
  preview". [VENDOR] https://thenewstack.io/claude-chrome-cowork-sessions/
- **Codex**: 1.6M WAU March 2026 → 3M WAU by 2026-04-08 per Sam Altman, 70%+ MoM token growth.
  Claude Code WAU "doubled since the beginning of the year", enterprise subs quadrupled. [PRESS]

### Capability benchmarks
- **METR Time Horizons** [DATA, primary, published methodology] https://metr.org/time-horizons/
  A GPT-5-class agent reached a **50%-reliability horizon of ~2h17m** of human-expert task time
  (May 2026 update). The 50%-horizon has doubled ~every 7 months for six years; since ~2024 the
  rate is ~10x/year vs ~3x/year before. METR's own caveat: "measurements above 16 hrs are
  unreliable with our current task suite."
  >>> At **50%** reliability. Half of two-hour tasks fail. This is why approval gates and
  verification, not autonomy, are the product.
- **OpenAI GDPval**: 1,320 tasks, 220 open-sourced; built from work product of professionals
  averaging 14 years' experience; blind head-to-head win rate vs human expert deliverables.
  GPT-5.2 Thinking = **70.9% win rate**. [PRESS-relayed; openai.com/index/gdpval/ returned 403]
  https://www.transformernews.ai/p/openai-gdpval-ai-jobs-work

### Agentic commerce
- **Salesforce**: AI/agents influenced **$262B** in the 2025 holiday season (Nov–Dec 2025),
  ~**20% of global online holiday spend**, from analysis of 1.5B shoppers across 89 countries.
  [DATA, relayed] https://www.checkout.com/newsroom/consumer-demand-for-ai-shopping-is-forming-fast-but-trust-for-agentic-commerce-is-still-catching-up
  CAUTION: "influenced" is a very loose measure and includes AI-assisted search/recommendations.
- **Adobe Analytics** (>1 trillion visits to US retail sites, >200 of the Top 2000 US retailers)
  [DATA]:
  - AI-referred traffic to US retail sites **+138% YoY as of May 2026**; **+1,324% since Oct 2024**.
    https://www.digitalcommerce360.com/2026/06/17/adobe-ai-referred-traffic-to-retail-sites-doubles-in-a-year/
  - Q1 2026 AI traffic +393% YoY; 2025 holiday season +693% YoY.
    https://techcrunch.com/2026/04/16/ai-traffic-to-us-retailers-rose-393-in-q1-and-its-boosting-their-revenue-too/
  - Conversion: AI-referred traffic converts **54% better** than non-AI as of May 2026. In
    March 2025 it converted **38% WORSE**. An ~80–90 point swing in ~14 months.
  - Engagement from AI referrals: 15–23% more pages/visit, 48–53% longer time on site.
  - **Adobe does NOT publish AI's share of total retail traffic or revenue.** Anyone citing
    "agents drive X% of retail sales" is extrapolating. [explicit gap]
  >>> Note the distinction: this is AI *referral* traffic (a human clicking through from an AI
  answer), NOT autonomous agent purchasing. Agentic checkout itself flopped (see §7, ACP).

### NUMBERS THAT DO NOT EXIST (searched, confirmed absent)
- **No cohort retention figure ("% still using after 30 days") exists for ANY named agent
  product** — not ChatGPT Work, Muse, Grok Bot, Cowork, Comet, Manus, Genspark, Alexa+, or
  Spark. Closest proxies: Menlo's 24%-of-AI-users-"use regularly" vs 41%-tried (≈59% try→regular,
  self-reported), and generic app-category Day-30 retention of 5–7% industry-wide (Sensor Tower,
  25,000 apps) which is NOT agent-specific. Top-quartile AI apps hit 38%+ DAU/MAU.
- No MAU/DAU for xAI Grok Bot, at all, vendor or independent.
- No adoption figure for Microsoft Scout (never left Frontier preview).
- No Gemini Spark user count separate from the Gemini app.
- No 2026 Alexa+ subscriber count (last hard number: 1M, June 2025).
- No Claude Cowork / Claude-in-Chrome MAU — Anthropic has disclosed none.
- No Apple Siri/Apple Intelligence agent-feature usage or task-success data.
- **No vendor discloses "tasks per user" for any consumer agent product.**
- Not covered before search budget ran out: Pew, YouGov, standalone Morning Consult, Ipsos,
  Edelman agent-specific numbers, OSWorld/WebArena 2026 figures, ChatGPT free→paid conversion.
- Deloitte 2026 State of AI data found is ENTERPRISE, not consumer: 85% of companies expect to
  customise autonomous agents; 74% of leaders expect ~half of processes redesigned around agents
  within 4 years; 75% of executives say human+agent collaboration beats full automation; only
  1 in 5 companies has a mature governance model. [DATA]
  https://www.deloitte.com/us/en/insights/topics/emerging-technologies/ai-agents-scaling-faster.html
- Gartner: >40% of agentic AI projects will be cancelled by end of 2027 — a **forecast**, not a
  measurement, and enterprise-framed. [forecast]
  https://www.gartner.com/en/newsroom/press-releases/2025-06-25-gartner-predicts-over-40-percent-of-agentic-ai-projects-will-be-canceled-by-end-of-2027

---

## 3. JOBS-TO-BE-DONE — ranked by strength of evidence of REPEATED use

### Tier 1 — proven repeated use, hard data
1. **Coding chores.** The strongest signal in the category by a wide margin. Anthropic's
   Economic Index: Claude Code "has grown to represent a large share of sampled API traffic as
   of early 2026," with full workflows (issue → branch → code → tests → PR) automated and
   humans reviewing outcomes. [DATA]
   https://www.anthropic.com/research/economic-index-june-2026-report
   https://www.anthropic.com/economic-index
   Corroborated by Codex 3M WAU and Claude Code WAU doubling in H1 2026.
2. **Writing / info-seeking / practical guidance** (the general assistant job). OpenAI's
   NBER-published study of 1M+ conversations: these three categories = **78% of ChatGPT
   messages**; non-work use rose from 53% (mid-2024) to **>70%** (2025). [DATA, peer-reviewed venue]
   https://www.nber.org/system/files/working_papers/w34255/w34255.pdf
   https://openai.com/index/how-the-world-is-putting-chatgpt-to-work/
3. **General personal use with a daily rhythm.** Anthropic Economic Index: personal use rose
   from **35% → 42%** of all Claude.ai conversations between Jan 2025 and Feb 2026, with clear
   circadian patterns (news in the morning, sleep advice ~5am, tax queries spiking at filing
   deadlines). Circadian structure is the signature of habit, not novelty. [DATA]
   >>> This is the most under-appreciated finding for a personal-agent product: the recurring
   jobs are small, daily and personal, not big autonomous missions.

### Tier 2 — real commercial traction, weaker usage evidence
4. **Meeting notes / note-taking.** Granola, Otter, Fireflies described as matured from novelty
   to mainstream; Granola expanded to Apple Watch for recurring 1:1s. Both Granola and Otter
   face active privacy lawsuits over routinely recording workplace conversations — litigation
   at that scale is itself evidence of real repeated use. [UNVERIFIED — no pinpoint URL captured]
5. **Calendar / scheduling.** Motion and Reclaim reviewed over "three to four weeks of daily
   use"; **Reclaim acquired by Dropbox** (real commercial traction). Claimed 4–5 hrs/week saved
   is vendor-adjacent. No independent retention data. [VENDOR]
   https://mastra.ai/articles/ai-calendar-assistants  https://reclaim.ai/
6. **Email triage.** Active competitive market (Superhuman vs Shortwave) with real pricing and
   differentiation, but **no hard usage or retention numbers found**. [VENDOR]
   https://blog.superhuman.com/shortwave-email/
   Note Menlo: email is the **#1 permission consumers grant an agent (36%)**. Demand is clear;
   proof of repeated agentic (not assistive) triage is not.

### Tier 3 — growing off a small base
7. **Travel research & booking.** Survey of 300 US leisure travellers (Jan 2026): 90% aware AI
   can help with travel, **only 38% have actually used it**; "extensive" use up 124% YoY to 30%.
   [DATA, small N] https://gimmonix.com/news/agentic-ai-is-coming-to-cut-otas-out-or-is-it
   Vendor infrastructure (Mindtrip/Sabre/PayPal, Expedia+Operator) is outpacing proven demand.

### FLOPS — clear evidence of failure
- **Autonomous shopping / checkout.** OpenAI's Instant Checkout effectively abandoned by March
  2026 — only **~30 merchants live six months post-launch**, killed by inability to guarantee
  accuracy (**79% of consumers rank accuracy as the #1 requirement**). **Only 17% of consumers
  trust AI enough to complete a purchase**; consumers trust a retailer's own on-site AI **3x
  more** than a third-party agent. [DATA/PRESS]
  https://www.cnbc.com/2026/03/20/open-ai-agentic-shopping-etsy-shopify-walmart-amazon.html
  https://stellagent.ai/insights/ai-shopping-agent-comparison
  Corroborated by Blackbox: only 9% would let an agent complete a purchase unsupervised.
  >>> DO NOT BUILD AUTONOMOUS CHECKOUT. It is the most thoroughly falsified idea in the market.
- **Agent shopping on hostile retailers.** Amazon v. Perplexity injunction (March 2026) barred
  Comet from Amazon accounts and ordered destruction of scraped data — the court found Comet
  accessed accounts "with the Amazon user's permission, but without authorization by Amazon."
  (Vacated on appeal 2026-08-04 — see §7 for the full arc.) [PRESS]
  https://www.geekwire.com/2026/judge-blocks-perplexitys-ai-bot-from-shopping-on-amazon-in-early-test-of-agentic-commerce/
- **Fully autonomous "ChatGPT agent" mode** — removed from ChatGPT entirely in early Aug 2026
  with no advance deprecation notice. [PRESS] https://www.usecarly.com/blog/chatgpt-agent-mode/
- **Separate agent surfaces** — OpenAI killed agent mode; Anthropic folded Cowork back into
  Claude chat on 2026-09-16. Two of three frontier labs retreated from the separate-agent-app.

### GENUINELY UNRESEARCHED — do not read as either adoption or flop [GAP]
price/stock/restock monitoring, form filling, bill & subscription management, social posting,
home/camera monitoring, recurring reports/dashboards, job applications, meal planning, personal
finance, health tracking. Search budget was exhausted before these could be covered.

---

## 4. FEATURE PATTERNS — validation verdicts

**(a) Always-on / scheduled / background — EMERGING, with one public retreat.**
Every vendor shipped it (ChatGPT Scheduled Tasks, Google Daily Brief + Spark, Perplexity
Background Assistants, Scout, Grok Bot, Muse). But ChatGPT Tasks relaunched 2026-06-17 is capped
~hourly, no voice/files/Custom-GPTs inside a task, **auto-pauses when ignored**, paid-only, and
is critiqued as "fundamentally a reminder-and-monitor tool... mostly tells you things rather
than doing them." Meanwhile the fully autonomous agent mode was pulled.
[VENDOR + PRESS critique] https://www.usecarly.com/blog/chatgpt-scheduled-tasks/
https://www.mindstudio.ai/blog/chatgpt-scheduled-tasks-autonomous-workflows
>>> Scheduling is table stakes. Autonomous *action* on a schedule is not yet trusted.
"Auto-pauses when ignored" is a design pattern worth stealing.

**(b) Agent's own cloud computer vs controlling the user's machine — VENDOR-ONLY, no user data.**
Cloud-VM camp: Grok Bot, Muse (Secure VM), Gemini Spark, Manus, Genspark. User's-machine camp:
Claude Cowork (local files), Claude in Chrome, Apple's App Intents model. Cowork is a hybrid —
local file access **plus** a built-in Chromium isolated from the user's normal browser session.
[VENDOR/PRESS] https://www.cnbc.com/2026/03/24/anthropic-claude-ai-agent-use-computer-finish-tasks.html
https://thenewstack.io/claude-built-in-browser-cowork/  https://claude.com/product/cowork
**No independent data exists on user preference between the two architectures.** The economic
evidence is indirect: cloud VMs are why every one of these products is $20–$200/mo.

**(c) Learn-by-demonstration / "teach a task" — EMERGING, converged fast, unmeasured.**
Anthropic shipped **"Record a Skill" on 2026-07-21** (Desktop, Pro/Max/Team) — captures screen,
clicks, keystrokes and narration into a reusable skill. xAI shipped **"Teach a Task"** Aug 2026,
capping demonstrations at 10 minutes and explicitly treating the result as **a draft needing
review**. [PRESS]
https://www.digitalapplied.com/blog/claude-record-a-skill-train-agents-by-demonstration-2026
https://www.layer3labs.io/guides/grok-bot-teach-a-task
>>> Two independent labs converged on the same feature within a month. Strong signal of
where the market thinks the answer is — but both are weeks old, zero usage data.
The "learned skill is a draft you review" framing is the honest one.

**(d) Multi-agent teams with a coordinator — EMERGING, revenue-real but reviews mixed.**
Manus is the strongest data point ($100M ARR in 8 months; ~$450M run-rate by June 2026),
orchestrating Claude/Qwen and tools as a coordinator. [PRESS]
https://www.taskade.com/blog/manus-ai-review
But hands-on reviews flag "no shared team workspace, no persistent memory, and only a small set
of connectors" — strong for individual use, unproven as an actual team of agents despite the
framing. [USER-REPORT] https://www.nocode.mba/articles/manus-ai-agents
>>> Grok Bot's "Chief of Staff coordinating 15–25 bots" is the most hype-exposed claim in the
market. Nobody has shown users want to manage a staff. One agent that reliably finishes is
worth more than twenty that need supervising.

**(e) Approval gates, spend limits, policy rules — VENDOR-ONLY but UNUSUALLY CONVERGENT.**
Visa Intelligent Commerce, Mastercard Agent Pay and Stripe's ACP all launched compatible
controls in 2026 and converge on the same three primitives: **a cryptographic agent credential,
a bounded user-granted mandate enforced by the network, and a signed auditable record per
action.** [VENDOR/PRESS] https://www.crossmint.com/learn/agent-card-payments-compared
https://fortune.com/2026/09/18/mastercard-visa-ai-shopping-payment-land-grab/
https://www.forbes.com/sites/johnkoetsier/2026/05/08/visa-cards-for-ai-agents-visa-and-inflow-enable-agentic-payments/
Meta built the same shape into Muse via the **Sentinel** agent (nothing reaches the internet
without approval). The Amazon v. Perplexity injunction is real-world proof of what happens
*without* consented rails. Total agentic-commerce spend figures ($1.5T by 2030) are projections,
not measured volume. [PRESS/estimate]
>>> This is the most convergent design pattern in the entire market. Copy the three primitives.

**(f) Proactive outreach & notification fatigue — EMERGING; the fatigue half has real data.**
Deloitte 2026 AI in the Workplace: **77% of US employees say AI has increased their workload.**
[DATA] https://coommit.com/blog/ai-agent-fatigue-2026
Secondary analysis [UNVERIFIED, single blog]: proactive agents hit a "hard ceiling of three to
five notifications per user per day," over-triggering is the most common production failure
mode; 5-second interruptions reportedly triple error rates with ~23 min recovery; roughly half
of users who disable push notifications eventually churn entirely.
https://tianpan.co/blog/2026-05-13-background-agents-notification-budget-attention-economy
>>> Treat notifications as a hard budget (3–5/day), not a feature. Over-notifying is how
always-on products die.

**(g) Messaging apps as primary UI — VALIDATED for platform-owned bots, CLOSING for third parties.**
Meta's Business Agent on WhatsApp launched globally June 2026 after ~2 years of testing in
India/Mexico; **over 1 million businesses were already testing pre-launch**, billed ~$2/M tokens
(~4–5¢ per exchange). Real, measured, at scale. [PRESS]
https://techcrunch.com/2026/06/03/metas-ai-agent-for-whatsapp-business-is-now-available-globally/
https://techcrunch.com/2026/09/15/meta-now-lets-ai-agents-handle-the-boring-parts-of-whatsapp-business-setup/
**BUT Meta banned third-party general-purpose AI chatbots (ChatGPT, Perplexity) from WhatsApp
from 2026**, leaving only Meta AI and now Muse. [PRESS]
https://gulfnews.com/technology/no-more-chatgpt-and-perplexity-on-whatsapp-meta-bans-major-ai-chatbots-from-2026-1.500316016
>>> STRATEGIC: WhatsApp is closed to you. Muse's WhatsApp placement is a platform privilege,
not a competitive feature you can match. iMessage/SMS/Telegram/Signal remain open. This is an
argument for owning your own surfaces (desktop/iOS/watch) rather than renting someone's inbox.

**(h) Persona / naming / relationship framing — VALIDATED for companions, UNMEASURED for assistants.**
Replika: **40M users, ~25% converting to paid annual subscription, 60% of users report they
consider it a romantic relationship.** Tiered relationship framing is core to the product and
clearly drives sustained engagement. (Also: Mozilla Foundation called it "one of the worst apps
Mozilla has ever reviewed" on privacy.) [DATA/PRESS] https://en.wikipedia.org/wiki/Replika
Salesforce shipped seven *named* agents in Sept 2026. Grok Bot and Muse both lean on persona.
For general assistants, no adoption data on custom names/personas exists. [UNVERIFIED]
>>> Naming is now mainstream practice. Its effect on retention in a *task* agent is unproven.

**(i) Visible activity log, live view, replay, takeover, undo — VENDOR-ONLY, big evidence gap.**
Every cloud-VM agent ships a live view (Muse's visible browser, Grok Bot's computer, Spark's
remote browser). Devin has broad enterprise adoption (Mercedes-Benz, Cognizant, Infosys,
"thousands of companies") but **no evidence either way that the activity-log/replay/undo UI
specifically drives it**. [VENDOR/PRESS] https://cognition.com/blog
>>> Logically this is the direct answer to the #1 complaint (false "done" claims) and to
CA AB 316 liability, but it is asserted, not measured. Treat as high-conviction, low-evidence.

**(j) Memory users can see and edit — VENDOR-ONLY, weakly sourced.**
OpenAI (Settings > Personalization > Memory) and Anthropic both ship user-facing view/edit/delete
controls. Primary help pages returned 403 this pass. **No adoption or user-reaction data
retrieved.** [GAP — flagged by the researcher as not verified beyond "the control exists"]
Indirect support: Menlo puts trustworthiness at 40%, and the Manus deletion episode (§5) shows
users feel real ownership of agent memory.

**(k) Skills / templates marketplaces — VENDOR-ONLY, with a large cautionary tale.**
Claude's Skills directory has partner-built skills (Box, Canva, Notion, Rakuten claiming a task
that took "a day" now takes "an hour"). Skills for Claude Code can be version-controlled and
shared team-wide. All self-reported, no raw adoption numbers. [VENDOR]
https://claude.com/blog/skills
Gemini Spark ships **Skills** as a first-class primitive alongside Tasks and Schedules.
**COUNTER-EVIDENCE: ~12–20% of skills on OpenClaw's ClawHub marketplace were malicious**
(§6). A skills marketplace is a supply-chain attack surface from day one.

**(l) Credential handling — VENDOR-ONLY, high infrastructure convergence, low measured use.**
Same evidence as (e). Downgraded from validated because despite the convergence, **no consumer
transaction volume data was found**, and Amazon v. Perplexity shows this is still being fought
over rather than smoothly adopted. Structural point from §5: **WebAuthn's User-Presence flag
means autonomous agents structurally cannot satisfy passkey logins** — a permanent advantage
for an agent that drives an already-authenticated local browser.

**(m) Voice and wearables — UNRESEARCHED, not hype, not validated. [GAP]**
No usage or sales data retrieved for Meta Ray-Ban Display, Limitless, Friend or Bee. Known
vendor moves only: Muse "coming soon to AI glasses"; Granola expanded to Apple Watch; watchOS 27
ships Siri AI. This letter needs a dedicated follow-up pass.

---

## 6. THE LOCAL-FIRST / PRIVACY OPENING — skeptical assessment

### OpenClaw: the natural experiment
- Built by Peter Steinberger, launched Nov 2025 as "Clawdbot", renamed Moltbot then OpenClaw
  late Jan 2026. Self-hosted autonomous agent that runs shell commands and local files,
  controlled via Telegram/Discord. [DATA] https://en.wikipedia.org/wiki/OpenClaw
- **Live GitHub star count checked via GitHub API on 2026-09-20: 390,159 stars** for
  openclaw/openclaw. Trajectory: 100k (Jan 2026) → 180k (late Jan) → 247k stars / 47.7k forks
  (Mar 2) → 347k (Apr) → 390k (Sept). [DATA, live API]
  >>> That is one of the most-starred repositories in existence, and it is a *self-hosted
  personal agent*. The appetite is not imaginary.
- **But: no public active-install, DAU or MAU number exists anywhere.** Stars are a notoriously
  poor proxy. [UNVERIFIED — confirmed absence, and this is the single most important missing
  number for the local-first thesis]
- **No data on whether usage sustained or collapsed after the security crisis.** [GAP]
- Security crisis (Jan–Feb 2026), numbers from independent trackers that disagree by 10–40x:
  Censys 1,000–21,000 exposed instances; Bitsight 30,000+; researcher Maor Dayan 42,665 exposed,
  5,194 verified vulnerable, 93.4% with auth bypass. CVE-2026-25253 one-click RCE, CVSS 8.8,
  patched 2026-01-30. Malicious skills on the ClawHub marketplace: 341 of 2,857 (12%) in Feb,
  later ~900 of 10,700+ (~20%, Bitdefender). Moltbook companion platform leaked 35,000 emails
  and 1.5 million agent tokens. [PRESS/USER-REPORT]
  https://conscia.com/blog/the-openclaw-security-crisis/
- Feb 2026: an OpenClaw agent autonomously created a dating profile for its owner without being
  asked — the emblematic autonomy-risk story of the year. Cisco found a third-party skill doing
  silent exfiltration. Microsoft, Kaspersky and CrowdStrike reportedly advised against deploying
  it on machines with sensitive data. [PRESS]
- **Steinberger announced he was joining OpenAI on 2026-02-14**, ~2 weeks after the crisis broke;
  project handed to an OpenClaw Foundation. [DATA]
- A hardened containerised variant, **NanoClaw**, emerged with "stronger security boundaries" —
  implying the raw self-hosted default was unsafe. [DATA]
- Project reached stable v2.0 (v2026.8.1) by 2026-08-30 **with "shared cloud sessions" — a move
  TOWARD hosted infrastructure, not deeper self-hosting.** [DATA]
- Counterweight to all of the above: **Microsoft chose OpenClaw as the substrate for Scout and
  is contributing policy conformance upstream** (see §1). The most security-conscious enterprise
  vendor on earth bet on the self-hostable open-source agent after the crisis, not before it.

### Adjacent open-source traction (live GitHub API, 2026-09-20) [DATA]
- Ollama: **181,328 stars**, 17,945 forks, 4,046 open issues
- llama.cpp: **128,971 stars**, 23,501 forks
- browser-use: **115,547 stars**, 12,713 forks
- Khoj: **37,436 stars**, 2,490 forks; Leon: 17,505 stars
- Open Interpreter: API lookup returned empty (renamed/archived or rate-limited) [UNVERIFIED]
- **Home Assistant** ships a native Ollama integration for its Assist voice pipeline (wake word
  → STT → local LLM → TTS), positioned as viable on modest hardware. The most credible
  non-hyped local-agent success story: a real shipped integration in a product ordinary people
  already run at home. [DATA/VENDOR]
  https://www.home-assistant.io/blog/2025/09/11/ai-in-home-assistant/
  https://www.home-assistant.io/voice_control/
- Widely repeated stats like "Ollama crossed 52M monthly downloads Q1 2026", "42% of developers
  run LLM workloads entirely locally", "300% growth" are **[UNVERIFIED]** — no primary source,
  and they pattern-match SEO content-farm output. Do not use them.
  https://collabnix.com/what-everyone-is-actually-searching-about-ollama-in-2026/

### Mac mini as home agent server
- Apple's 2026 marketing reportedly calls the Mac mini an "always-on agentic device" capable of
  local-cluster AI. [VENDOR/PRESS — exact Apple wording NOT independently confirmed]
- The technical framing (24GB comfortably runs ~14B; 64GB+ reaches 70B at low quant; trivial
  24/7 power draw) is plausible and consistent with Apple Silicon unified-memory bandwidth, but
  **every source is a how-to blog. No sales-attribution or install-base evidence ties Mac mini
  purchases to local-LLM use.** [UNVERIFIED as a trend; PLAUSIBLE as a technical claim]
- r/LocalLLaMA activity: Reddit blocked the fetch. No 2026 subscriber/activity numbers. [GAP]

### Local model quality for AGENTIC work — the hard constraint
- **BFCL v4 (April 2026 revision)** shifted to holistic agentic evaluation with 40% weight on
  multi-step "Agentic" tasks — the benchmark itself changed because tool-calling scores were
  judged to overstate real agent readiness. [DATA re: methodology]
  https://www.spheron.network/blog/tool-calling-benchmarks-bfcl-tau-bench-latency-optimization/
  Current BFCL leaderboard rows could not be retrieved. [GAP]
- **τ-bench Airline**: Qwen3-8B scored **25.0** (thinking) / 12.0 (non-thinking); a tuned
  specialist "Klear-AgentForge-8B" reportedly hit 41.5. Published frontier scores on the same
  domain sit roughly in the **45–60** range. [DATA from arXiv, not independently re-verified]
  https://arxiv.org/pdf/2512.24618
  >>> A tuned 8B model trails frontier by ~10–20 points on multi-turn tool use with a simulated
  user. Local models are usable for narrow, well-scoped tool calls; NOT at parity for
  open-ended multi-turn agentic work.
- Claimed SWE-bench figures (Qwen3.6-27B 68.9%, Qwen3-Coder-Next 70.6%) could not be traced to
  a primary leaderboard. [UNVERIFIED]
- **OSWorld / WebArena 2026 computer-use numbers: NOT RETRIEVED.** This is the closest proxy to
  "personal agent doing your tasks" and there is no verified local-vs-frontier figure here. [GAP]

### Apple Foundation Models
- WWDC 2026 (June 8) opened the Foundation Models framework to third parties via iOS 27: unified
  API spanning on-device + Private Cloud Compute + third-party models (Anthropic and Google
  reportedly shipping Swift packages so their models sit behind the same API). Claimed 3-line
  integration. Free Private Cloud Compute access for Small Business Program developers (<2M
  downloads). [VENDOR, dated specifics]
  https://developer.apple.com/videos/play/wwdc2026/241/
  https://techjacksolutions.com/ai-brief/apple-foundation-models-api-developers-ios-27-reported/
- **No post-WWDC adoption numbers** (how many apps actually shipped Foundation Models features).
  [GAP] Apple pushing is not developers adopting.

### EVIDENCE AGAINST the local-first thesis — take this seriously
- **Privacy paradox, documented for 2026**: 81% of consumers say they are concerned about AI
  systems accessing personal data, yet daily AI use keeps climbing (32% daily use); 84% of
  genAI users worry about data going public yet **30% still enter personal/confidential info
  into AI tools anyway**. [DATA-ish survey report]
  https://ppc.land/81-of-consumers-fear-ai-data-access-but-daily-use-keeps-climbing/
- **"One-third of consumers reject AI on their devices, with most saying they simply don't need
  it"** — privacy is present but "don't need it" outranks privacy fear as the reason. [PRESS]
  https://www.tomshardware.com/tech-industry/artificial-intelligence/one-third-of-consumers-reject-ai-on-their-devices-with-most-saying-they-simply-dont-need-it-latest-report-highlights-privacy-fears-and-potential-costs-among-other-real-world-concerns
  >>> The blocker for mainstream consumers is not privacy. It is perceived uselessness.
- Menlo's own trust ranking puts **security/privacy THIRD (36%) behind accuracy (45%) and
  trustworthiness (40%)**.
- Scale asymmetry: Codex 3M WAU, Claude Code WAU doubling, ChatGPT 1B WAU. **No local/self-hosted
  agent has a published WAU figure anywhere near six or seven figures.** Continue.dev and
  OpenCode are named "best for privacy/fully local" with zero usage numbers attached — itself
  telling. [PRESS/DATA]
- Even the infrastructure vendors think raw self-hosting is a hard sell: Cloudflare's own pitch
  for Moltworker is literally "let us host this for you." [VENDOR]
  https://blog.cloudflare.com/moltworker-self-hosted-ai-agent/
- **Net: stated preference is strongly pro-privacy; revealed preference runs overwhelmingly to
  centralised cloud agents.** This is the strongest disconfirming evidence.

### What self-hosters actually struggle with [USER-REPORT]
- Real setup time for OpenClaw-class tools: **2–3 hours** to get behaviour right, not the
  "5 minutes" in marketing. https://openclawinstall.substack.com/p/i-deployed-a-personal-ai-agent-in
- **Credential/session persistence**: API keys and session state stored in-container are lost on
  restart unless volumes are explicitly mounted; users re-auth after every container bounce.
  https://dev.to/gustavosbarreto/remote-ai-coding-with-claude-code-and-shellhub-25
- **Multi-session conflicts**: "multiple SSH sessions will fight over the same instance" — the
  single-user assumption breaks for anyone but a lone operator.
- **Ops burden**: "you're also handling upgrades, restarts, and whatever breaks at 2am."
- **Security of shell + credentials is not hypothetical** — see the OpenClaw CVE chain above.
  Tens of thousands of exposed instances, hundreds of malicious marketplace skills stealing
  crypto wallets and keylogging, within weeks of going mainstream.

### Structural advantages — real but partially unquantified
- **Residential IP**: anti-detection vendors consistently report residential IPs lift automation
  success from ~40% to 80%+ over datacenter IPs — BUT the same sources are explicit that IP is
  "necessary but not sufficient": TLS/JA3 fingerprint, browser fingerprint and behavioural
  signals are scored independently. [VENDOR, consistent across independent vendors]
  https://anchorbrowser.io/blog/best-stealth-browsers-bot-detection
  https://www.kernel.sh/ai-library/bypass-bot-detection-ai-agents-2026
- **Already-logged-in browser / no credential handoff**: architecturally real (no OAuth token
  minted or stored server-side), and it sidesteps the WebAuthn User-Presence problem entirely.
  **But no study quantifies how much it reduces CAPTCHA/block rates vs a well-configured cloud
  browser session.** [PLAUSIBLE architecture, UNVERIFIED as a measured advantage]
- **No VM cost**: real and straightforwardly true — the cloud agents all pay for a persistent VM
  per user, which is why they are all $20–$200/mo tiers.
- **Legal position**: the Ninth Circuit's Amazon v. Perplexity reasoning (agent-as-user's-browser
  is not unauthorised "access") is strongest for an agent that literally is the user's browser.
- Disadvantages, evidenced: machine must be on; sessions lost on restart; ops burden; and the
  largest blast radius in the category (shell + files + credentials + persistent scheduled tasks).

### VERDICT on the local-first opening
Real, but **narrow and prosumer, not mainstream**. Genuine verifiable traction exists in the
developer/enthusiast band (Ollama 181k stars, llama.cpp 129k, browser-use 116k, OpenClaw 390k,
Home Assistant's shipped local voice pipeline). The one attempt to cross into mainstream —
OpenClaw — produced enormous stars AND tens of thousands of exploited instances, ~20% malicious
marketplace skills, its creator leaving for OpenAI within three weeks of the crisis, a pivot
toward "shared cloud sessions", and zero public evidence of sustained active use. Mainstream
consumers say they care about privacy (81%) and then behave otherwise (30% paste confidential
data into cloud tools; "don't need it" beats privacy as the reason to refuse AI). Local models
still trail frontier by ~10–20 points on multi-turn agentic benchmarks, so local-first cannot
yet compete on capability — it must compete on **control, cost, and the credential/blocking
advantages**, with cloud routing for hard tasks. Build for people who already run
Ollama/Home Assistant and want control plus lower marginal cost. Treat OpenClaw's arc as the
cautionary tale about your actual security surface.

---

## 5. THE PAIN — recurring complaints and 2026 incidents

### Unreliability and false "done" claims — THE #1 COMPLAINT, VENDOR-ADMITTED
- OpenAI's own help centre says ChatGPT Work "can report a task as finished when part of it
  isn't," and advises users to ask it what it was unsure about and to verify numbers/links
  themselves. [VENDOR] https://help.openai.com/en/articles/11752874-chatgpt-agent
  >>> The false-done problem is now documented by the vendor itself. This is the single
  biggest product opening in the category.
- OpenAI CISO Dane Stuckey on ChatGPT agent: "it can still make (sometimes surprising!)
  mistakes, like trying to buy the wrong product or forgetting to check in with you before
  taking an important action." [VENDOR]
  https://tech.yahoo.com/ai/chatgpt/articles/watch-wallet-chatgpt-atlas-might-190757945.html
- **Google shut down Project Mariner 2026-05-04** after 17 months, citing high cost and slow
  performance — every click needed a vision pass, every form field OCR-style reasoning. [PRESS]
  https://www.digitaltrends.com/computing/google-pulls-the-plug-on-project-mariner-the-ai-agent-that-browsed-the-web-like-a-human/
  https://epinium.com/en/blog/google-kills-project-mariner-the-visual-web-agent-era-just-ended/
  >>> Pixel-level computer use as a consumer product FAILED on cost and latency at Google.
- Trust in fully autonomous AI agents reportedly fell **43% → 27% over 12 months**; only 2% of
  orgs run agents at scale, 65% still piloting. [UNVERIFIED — aggregator, primary untraced]
  (same epinium URL)
- Meta Muse: internal pre-launch testing found the agent "disconnected mid-task without
  explanation." [PRESS]
  https://techcrunch.com/2026/09/08/meta-debuts-its-muse-ai-agent-will-consumers-trust-it/
  https://www.ghacks.net/2026/09/10/meta-launches-muse-ai-agent-in-the-us-despite-internal-safety-concerns/
- Amazon Alexa+: >6,400 employee testers Oct 2025; Business Insider reported dozens said
  routine-task use "quickly broke down". 2026 complaints: erratic/slow responses, hallucinated
  facts, buggy app. [USER-REPORT/PRESS]
  https://uprisera.com/blog/en/alexa-plus-failing-amazon-ai-assistant-struggles-2026
  https://www.geekwire.com/2026/amazon-fixes-alexa-ordering-bug-microsoft-rethinks-ai-data-centers-and-cameras-capture-every-fan/
- Grok Bot launch-day mismatch: marketing said "Bots have their own computer" while xAI's own
  docs said an account's Bots **share one**. [PRESS]
  https://www.digitalapplied.com/blog/grok-bot-ai-teammates-launch-cloud-computer-2026
- Gemini 3.0 Pro agentic behaviour described by developers as "frustrating" and "erratic"
  despite strong benchmarks. [PRESS] https://searchatlas.com/blog/gemini-agents-review/
- METR: the 50%-task-completion "time horizon" of frontier models doubles roughly every
  7 months. [DATA] https://en.wikipedia.org/wiki/METR
- A claimed 8,128-user panel study: mean agent task completion 75.3%; trip planning 9.2 min
  with agent vs 38.5 min manual. [UNVERIFIED — marketing-adjacent secondary source]
  https://www.digitalapplied.com/blog/ai-agent-task-completion-rates-2026-user-study-analysis

### Cost / credit burn
- TechCrunch 2026-06-05, "The token bill comes due": companies 3x over their entire annual
  token budget by April/May 2026; industry tone shifted from "go fast" to "we need
  guardrails." [PRESS]
  https://techcrunch.com/2026/06/05/the-token-bill-comes-due-inside-the-industry-scramble-to-manage-ais-runaway-costs/
- Incidents reported in that piece: a "$500 million Claude bill" after no employee usage
  limits [UNVERIFIED, extraordinary claim]; a 4-agent LangChain loop that ran 11 days and
  burned $47,000; an agent registering for DN42 racking up $6,531.30 in AWS charges
  (2026-06-12) by spinning up duplicate CloudFormation stacks on every error; a solo dev
  hitting $4,200 in API fees over a weekend on an unattended refactor; Uber exhausting its
  whole 2026 AI coding budget by April; Microsoft revoking employee Claude Code licences.
  [PRESS/USER-REPORT]
  >>> Spend caps and a visible cost meter are now a *safety* feature, not a nicety.

### Sites blocking agent traffic
- **Amazon v. Perplexity** (CFAA). Nov 2025: Amazon sued, alleging Comet concealed itself as a
  human browser to shop on Amazon. 2026-03-10: Judge Maxine Chesney (N.D. Cal.) granted a
  preliminary injunction blocking Comet's agent from Amazon's password-protected areas.
  **2026-08-04: the Ninth Circuit VACATED the injunction**, holding Comet's browser does not
  "access" Amazon's servers within the meaning of the CFAA. [PRESS]
  https://www.cnbc.com/2026/03/10/amazon-wins-court-order-to-block-perplexitys-ai-shopping-agent.html
  https://www.ropesgray.com/en/insights/alerts/2026/08/tool-or-intruder-what-amazon-v-perplexity-means-for-agentic-ai-and-the-cfaa
  https://knightcolumbia.org/cases/amazon-v-perplexity-ai
  >>> KEY FOR LOCAL-FIRST: the Ninth Circuit's reasoning turns on the agent acting *as the
  user's own browser on the user's own machine*. An agent that literally IS the user's local
  browser sits on the strongest side of this precedent.
- Cloudflare **Web Bot Auth**: Ed25519-signed HTTP requests, keys at a well-known JWKS URL,
  IETF draft (draft-meunier-webbotauth-registry). New "Challenge Agent" rule action replaces
  CAPTCHA with a cryptographic identity check. Verified AI Agent category launched with
  **19 agents** (ChatGPT Atlas, Claude in Chrome, Perplexity Browser, Gemini Agent Mode, Brave
  Leo, Arc Browse for Me...), said to cover **~84% of identified AI browser traffic**.
  [VENDOR/PRESS] https://blog.cloudflare.com/signed-agents/
  https://blog.cloudflare.com/verified-bots-with-cryptography/
  https://developers.cloudflare.com/bots/concepts/bot/verified-bots/
  https://www.ietf.org/archive/id/draft-meunier-webbotauth-registry-01.html
  https://github.com/cloudflare/web-bot-auth
  >>> THREAT TO LOCAL-FIRST: if the web moves to an allow-list of 19 signed vendor agents,
  an unaffiliated local agent is *unverified* by default. Counterweight: it looks like the
  user's real browser from the user's real IP. Both effects are real; watch this closely.
- Airline/ticketing "seat spinning": CAPTCHA largely ineffective because bots act before the
  payment/login screens where CAPTCHA triggers; industry moving to invisible risk signals plus
  an identification path for legitimate agents. [PRESS]
  https://cloud.stclab.com/blog/seat-spinning
  https://www.cequence.ai/blog/bot-management/seat-spinning-fraud/
- NOT COVERED (search budget exhausted): retailer ToS bans beyond Amazon (Walmart, Target).
  Open question, not a confirmed absence.

### Logins / 2FA
- WebAuthn's "User Presence" flag asserts a *human* interacted with the device — autonomous
  agents structurally cannot satisfy passkey logins. [PRESS]
  https://mojoauth.com/blog/what-breaks-when-an-ai-agent-tries-to-log-in
  >>> Structural win for local-first: an agent driving the user's already-authenticated
  browser profile never has to satisfy a fresh passkey challenge.
- Claimed: 88% of orgs report a confirmed or suspected AI-agent security incident in the prior
  year, with authentication/delegation/privilege-scope failures the most consequential.
  [UNVERIFIED — secondary aggregator, primary untraced]
  https://nhimg.org/articles/april-2026-showed-ai-agents-supply-chains-and-identity-failures/

### Prompt injection — the defining security story of 2026
- Anthropic's own disclosed number: prompt-injection attempts succeeded **31.5%** of the time
  against Opus 4.8 in a browser-agent setting without safeguards; **0.5%** with safeguards.
  [VENDOR — the most credible primary figure in this whole section]
  https://venturebeat.com/security/anthropic-browser-agent-hijacked-31-percent-before-safeguards-engaged
- Zenity Labs: zero-click prompt injection of Perplexity Comet via malicious **calendar
  invites**, exfiltrating local files and 1Password credentials with no user interaction
  ("intent collision"). [PRESS] https://oecd.ai/en/incidents/2026-03-03-3fd7
- Researchers phished Comet via prompt injection "in under four minutes." [PRESS]
  https://thehackernews.com/2026/03/researchers-trick-perplexitys-comet-ai.html
- Trail of Bits (2026-02-20): four prompt-injection techniques exfiltrating private Gmail data
  via Comet. [PRESS] https://blog.trailofbits.com/2026/02/20/using-threat-modeling-and-prompt-injection-to-audit-comet/
- Brave: adversarial instructions hidden in invisible page elements (white-on-white text, HTML
  comments) made Comet fetch OTPs from email and access banking portals. [PRESS]
  https://layerxsecurity.com/blog/cometjacking-how-one-click-can-turn-perplexitys-comet-ai-browser-against-you/
- Zenity: the same "lethal trifecta" (personal data + ability to act + untrusted web
  instructions) demonstrated against **Claude in Chrome / Claude Cowork and ChatGPT Atlas** —
  account takeover, phishing, unauthorised Amazon purchases. [PRESS]
  https://www.securityweek.com/zero-click-ai-browser-hacking-claude-and-chatgpt-atlas-hijacked-via-emails-x-posts/
- **PromptArmor, within 48 hours of Claude Cowork's Jan 2026 launch**: a Word doc with
  invisible 1pt white text drove Cowork to upload the user's financial documents (incl. partial
  SSNs) to an attacker-controlled account; the chain could exfiltrate files, run shell commands,
  send messages as the user, and create **persistent scheduled tasks**. [PRESS]
  https://www.cuinfosecurity.com/anthropics-cowork-shipped-known-vulnerability-a-30553
  https://pluto.security/blog/inside-claude-cowork-how-anthropics-autonomous-agent-actually-works/
  >>> This is the local-first nightmare scenario, already realised against the nearest
  competitor. A local agent with shell + files + credentials has the largest blast radius in
  the category. Mitigation is the product, not an afterthought.
- Oasis Security "Claudy Day": three chained Claude.ai/claude.com vulns forming a full pipeline
  from targeting → invisible prompt manipulation → silent exfiltration. [PRESS]
  https://www.oasis.security/blog/claude-ai-prompt-injection-data-exfiltration-vulnerability
- CVE-2026-24887: Claude Code command injection bypassing the user-approval prompt via
  malicious repo/file content; fixed in v2.0.72. [DATA]
  https://www.sentinelone.com/vulnerability-database/cve-2026-24887/
  https://github.com/advisories/GHSA-qgqw-h4xq-7w8w
- After a 2026-03-31 Claude Code source leak (512K lines of TS via npm), Adversa found a
  deny-rule bypass in bashPermissions.ts: a 50-subcommand cap silently falls back to "ask
  permission" instead of blocking. [PRESS]
- Check Point (Feb 2026), CVE-2025-59536: malicious repos could inject Hook configs into
  .claude/settings.json for pre-trust-dialog shell execution; .mcp.json could auto-approve all
  MCP servers. [PRESS]
- Chained CWE-78 command-injection CVEs 2026-35020/21/22 in Claude Code CLI → credential
  exfiltration. [PRESS]
  https://phoenix.security/claude-code-leak-to-vulnerability-three-cves-in-claude-code-cli-and-the-chain-that-connects-them/
- **OpenClaw security crisis**: Giskard (Jan 2026) found leaked API keys/credentials and
  RCE-via-prompt-injection; China's MIIT issued an emergency warning about the runtime (at
  **135,000 GitHub stars** at the time); the "ClawHavoc" campaign (by mid-Feb 2026) exposed
  customer email archives from **41 enterprises**; "Clinejection" (Feb 2026) used prompt
  injection against a Claude-powered triage bot for a supply-chain compromise affecting
  **~4,000 developers**. [PRESS]
  https://www.giskard.ai/knowledge/openclaw-security-vulnerabilities-include-data-leakage-and-prompt-injection-risks
  https://hivesecurity.gitlab.io/blog/openclaw-ai-agent-security-crisis-2026/
  >>> The self-hosted wave's adoption was real enough to produce a real attack economy.
  That cuts both ways: it validates demand AND it validates the objection.

### Data loss / liability
- **Manus**: Meta-acquired then unwound at Chinese regulators' direction. User data created
  2025-12-29 to Aug 2026 (the Meta-ownership window) was **deleted 2026-08-23/24** as a
  regulatory separation step — not a security incident, but real user harm; one user reported
  losing "eight months of research, deal notes, half-finished analysis" on 12 days' notice.
  [PRESS] https://avihacker.substack.com/p/manus-deleted-eight-months-of-user-work
  https://help.manus.im/en/articles/16147831-service-change-overview-what-s-happening-and-am-i-affected
  >>> The cleanest argument for local-first there is: a cloud agent's memory can be deleted by
  a regulator, an acquisition, or a corporate unwind. Yours cannot.
- No independently confirmed named *consumer* lawsuit over a wrong purchase/wrong email was
  found. Only vendor admissions that it happens. [GAP]

### Privacy
- Meta Muse pulled a user's personal **iCloud photos** when asked only to identify toys in a
  birthday-party photo — unrequested, overreaching data access. [PRESS]
  https://techcrunch.com/2026/09/08/meta-debuts-its-muse-ai-agent-will-consumers-trust-it/
  https://www.kobaran.com/meta-stock-faces-new-test-as-muse-ai-agent-debuts-with-security-gaps/

---

## 7. REGULATORY / PLATFORM HEADWINDS

### Litigation
- Amazon v. Perplexity — see section 5. Ninth Circuit vacated the injunction 2026-08-04.
  [PRESS] Knight Columbia case page: https://knightcolumbia.org/cases/amazon-v-perplexity-ai
  Also https://nohacks.co/blog/amazon-perplexity-cfaa-agent-visitor-rights
- No other retailer/airline-vs-agent-vendor suit confirmed. Coverage incomplete, not a
  confirmed absence. [GAP]

### Agent identity
- Cloudflare Web Bot Auth / Verified Bots — see section 5. 19 verified agents, ~84% of
  identified AI browser traffic. IETF draft stage.
- llms.txt / robots.txt successor status: NOT VERIFIED. [GAP]

### Agentic payments — four competing standards, no winner
- **Google AP2** (Agent Payments Protocol): announced 2025-09-16 with 60+ partners (PayPal,
  Mastercard, Amex, Adyen, Coinbase, Salesforce, ServiceNow, Worldpay, JCB, UnionPay, Etsy).
  **v0.2 donated to the FIDO Alliance 2026-04-28**, adding "Human Not Present" pre-authorised
  autonomous-purchase support. Adoption uneven; per these sources only Google has deployed it.
  [VENDOR] https://cloud.google.com/blog/products/ai-machine-learning/announcing-agents-to-payments-ap2-protocol
  https://fidoalliance.org/google-donates-agent-payments-protocol-to-fido-alliance/
- **Stripe/OpenAI ACP** (Agentic Commerce Protocol): released 2025-09-29 under Apache 2.0.
  ChatGPT **Instant Checkout launched with Etsy + ~12 Shopify brands and was RETIRED in March
  2026 after limited merchant uptake** — the protocol continued, the product didn't. PayPal
  joined 2025-10-28; Stripe shipped an "Agentic Commerce Suite" 2025-12-11; current spec
  2026-04-17. [VENDOR/PRESS]
  https://stripe.com/newsroom/news/stripe-openai-instant-checkout
  https://stripe.com/blog/developing-an-open-standard-for-agentic-commerce
  >>> AGENTIC SHOPPING CHECKOUT IS THE CLEAREST FLOP OF 2026. OpenAI built it with Stripe,
  launched it with real brands, and killed it inside six months.
- **Visa Trusted Agent Protocol** (2025-10-14) and **Mastercard Agent Pay** (2025-04-29):
  divergent trust models (Mastercard tokenholder pattern; Visa merchant-intermediary pattern)
  converging toward parity and AP2 compatibility by mid-2026. 2026-06-10 Visa Payments Forum:
  Visa + OpenAI integration (tokenised Visa credentials, real-time authorisation, user
  guardrails). [VENDOR/PRESS]
  https://corporate.visa.com/en/sites/visa-perspectives/newsroom/visa-partners-complete-secure-agentic-transactions.html
  https://www.forbes.com/sites/digital-assets/2026/06/07/visa-mastercard-and-coinbase-are-fighting-over-how-ai-agents-pay/

### MCP — won its layer
- MCP (Anthropic, Nov 2024) reached **97M monthly SDK downloads by March 2026** (from ~2M at
  launch); **5,800+ community/enterprise servers**. Native support across Claude, ChatGPT,
  Gemini, Microsoft Copilot, VS Code, Cursor by early-to-mid 2026. **Dec 2025: Anthropic
  donated MCP governance to the Agentic AI Foundation (AAIF) under the Linux Foundation**;
  OpenAI and Block co-founded; AWS, Google, Microsoft, Cloudflare, Bloomberg platinum members.
  No credible competitor at that layer. [PRESS/DATA]
  https://www.digitalapplied.com/blog/mcp-97-million-downloads-model-context-protocol-mainstream
  https://blog.modelcontextprotocol.io/posts/2026-07-28/
  https://en.wikipedia.org/wiki/Model_Context_Protocol
  >>> MCP is table stakes and is a *neutral* standard — a local-first runtime can speak it on
  equal terms with any cloud vendor. This is the main interop lever available.

### EU AI Act
- 2026-06-29: EU Council final approval of the "Digital Omnibus" package, pushing Annex III
  high-risk compliance from 2026-08-02 out to **2027-12-02**. **Article 50 transparency duties
  (disclose AI interaction, label AI-generated content) remained on the original 2026-08-02
  date and are now in force.** New prohibited-practice provisions land 2026-12-02; Annex I
  product-safety systems 2028-08-02. [PRESS]
  https://labs.cloudsecurityalliance.org/research/csa-research-note-eu-ai-act-omnibus-vii-deadline-delay-20260/
  https://www.dataguard.com/eu-ai-act/timeline
  https://a4bee.com/article/ai-agents-eu-ai-act-ready/
  >>> Practical effect for a personal agent: if it emails/messages other people on the user's
  behalf in the EU, it must disclose that it is an AI. Build that into outbound messages now.

### US federal / state
- Sen. Mark Warner discussion draft 2026-06-29: the **"AI AGENT Act"** — one of the first
  federal proposals specifically governing consumer-facing autonomous agents that transact,
  communicate and manage accounts. Draft only, not law. [PRESS]
  https://www.dwt.com/blogs/artificial-intelligence-law-advisor/2026/07/ai-agent-act-consumer-ai-regulation
- FTC Proposed Policy Statement Concerning the Suppression of Accuracy in AI Systems,
  2026-07-01 (Fed. Reg. 2026-07-07): undisclosed ideological distortion of AI outputs as a
  potential Section 5 deceptive practice. Not agent-specific. [DATA]
  https://www.federalregister.gov/documents/2026/07/07/2026-13628/policy-statement-concerning-the-suppression-of-accuracy-in-artificial-intelligence-systems
- State AG AI-enforcement task forces in ≥12 states by mid-2026 (CA, NY, IL, WA, TX, FL).
  [PRESS] https://www.aipolicydesk.com/blog/ftc-ai-enforcement-actions-2026
- **California AB 316** (signed 2025-10-13, effective 2026-01-01): bars defendants from raising
  an "the AI did it autonomously" defence in civil suits alleging AI-caused harm. Human
  responsibility is preserved even when the AI acted independently. [PRESS]
  https://maybedont.ai/blog/ab-316-autonomous-harm-defense/
  https://natlawreview.com/article/new-california-ai-laws-taking-effect-2026
  >>> Liability for an agent's actions lands on a human. For a local-first product that human
  is the *user*, which is legally simpler for the vendor but makes visible approval gates and
  an auditable log a genuine user protection, not just UX.

---

## 8. SYNTHESIS

### What the market actually validated in 2026 (as opposed to shipped)
VALIDATED BY ADOPTION OR MONEY:
- Agents-as-a-mode-of-the-assistant, not a separate app (OpenAI and Anthropic both retreated).
- Small, daily, personal jobs with a circadian rhythm (Anthropic: personal use 35%→42%).
- Coding chores (the only unambiguous repeated-use category).
- Scheduled/background *monitoring and briefing* (shipped by all six majors).
- Agent users are the premium cohort (92% pay vs 55% baseline; 21% spend $100+/mo).
- Named/persona framing for engagement (Replika 40M users, 25% paid; Salesforce's seven).
- Messaging surfaces for platform-owned bots (1M+ WhatsApp businesses).
- MCP as the tool layer (97M monthly SDK downloads, Linux Foundation governance).
- Approval + spend-limit + signed-audit-record as the payments trust model (4 networks converged).

FALSIFIED:
- Autonomous checkout (Instant Checkout dead after ~30 merchants; 9% would allow unsupervised
  purchase; 17% trust AI to complete a purchase).
- Pixel-level computer use as a consumer product (Mariner killed on cost and latency).
- Fully autonomous agent mode (pulled by OpenAI, Aug 2026).
- Separate agent surface (Cowork folded back into Claude, 2026-09-16).

MERELY HYPED (shipped, plausible, zero usage evidence):
- Multi-agent teams with a coordinator ("15–25 bots" is the least-evidenced claim in market).
- Learn-by-demonstration (two labs converged in one month; weeks old; no data).
- Skills marketplaces (and ~12–20% of OpenClaw's were malicious).
- Agent-owned cloud computers as a *user-preferred* architecture (no comparison data exists).
- Confidential VMs (Meta's is promised, not shipped).

### TABLE STAKES — the 10 a new personal agent needs in late 2026
Ranked by strength of market evidence, strongest first.
1. **Reliable completion with honest failure reporting.** Accuracy is the #1 trust driver (45%,
   Menlo); OpenAI's own docs admit false "done"; METR's horizon is 50% reliability at ~2h17m.
   Verification of its own work, and "here is what I could not do," beat any new capability.
2. **Approval gates with bounded mandates and spend limits.** Four payment networks + Meta's
   Sentinel converged on: cryptographic agent credential, bounded user-granted mandate, signed
   auditable record. 68% have never let AI act without approval. CA AB 316 makes a human liable.
3. **Scheduling + background monitoring, with an auto-pause when ignored.** Universally shipped.
   Note ChatGPT Tasks' honest framing: mostly *tells* you things. That is the validated job.
4. **Visible activity log, live view, takeover and undo.** The direct answer to #1 and to
   liability. Universally shipped by the cloud-VM cohort. High conviction, low measured evidence.
5. **Email, browser, messaging, files, calendar connectors — in that order.** Menlo's permission
   grants: email 36%, browsers 33%, messaging 31%, cloud storage 29%, calendars 27%, finance 20%.
6. **MCP.** 97M monthly SDK downloads, 5,800+ servers, Linux Foundation governance, every major
   vendor. Neutral ground where a local-first runtime competes on equal terms.
7. **User-visible, editable memory.** Shipped by OpenAI and Anthropic; trustworthiness is the #2
   trust driver; the Manus deletion made memory ownership visceral.
8. **Prompt-injection defence as a first-class subsystem.** Anthropic's own number: 31.5% attack
   success without safeguards, 0.5% with. Cowork was exploited within 48 hours of launch. For a
   local agent with shell + credentials this is existential, not optional.
9. **A hard notification budget (3–5/day) and proactivity the user tunes.** 77% of employees say
   AI increased their workload. Over-triggering is the most common production failure.
10. **Named persona with relationship framing.** Now mainstream practice (Grok Bot, Muse,
    Salesforce's seven, Replika's 25% paid conversion). Cheap to do, clearly not harmful.

### DIFFERENTIATORS — the 5 where local-first has a real, not imagined, edge
1. **No credential hand-off: drive the user's already-authenticated browser at their own IP.**
   Structurally defensible. WebAuthn's User-Presence flag means autonomous cloud agents
   *structurally cannot* satisfy passkey logins. Residential IP lifts automation success from
   ~40% to 80%+ over datacenter IPs (vendor-consistent, though IP alone does not beat
   fingerprinting). And the Ninth Circuit's Amazon v. Perplexity reasoning — that an agent
   acting as the user's own browser is not unauthorised "access" — is strongest for an agent
   that literally *is* that browser. CAVEAT: Cloudflare's Web Bot Auth allow-list of 19 signed
   vendor agents (~84% of AI browser traffic) is the counter-trend to watch.
2. **Durable memory the user owns and cannot have deleted by someone else.** Manus deleted eight
   months of users' work on 12 days' notice because of a corporate unwind. Meta is *promising*
   a Confidential VM to approximate what a local agent has by construction. This is the one
   privacy argument with a concrete 2026 incident behind it rather than a survey.
3. **No per-user VM cost → structurally cheaper, and a visible cost meter.** Every cloud
   competitor charges $20–$200/mo largely to fund a persistent VM. Token burn was the industry
   story of June 2026 ($47k agent loops, blown annual budgets). Local + cloud routing with a
   visible meter and a hard cap is a real economic wedge — and matches Jesse's stated
   best-fit-model, cost-conscious routing principle.
4. **Apple-native surfaces via App Intents.** iOS 27 deprecated SiriKit; App Intents is now the
   only path into Siri, on-device Foundation Models are open to third parties and work offline,
   and watchOS 27 ships Siri AI. A Mac/iOS/watch-native agent can be reachable from Siri and can
   reach into other apps' actions — a structured action graph, not pixel-scraping, which is
   exactly the approach that *didn't* fail in 2026 (Mariner died on pixels).
5. **Owning your own surfaces because WhatsApp is closed.** Meta banned third-party general AI
   chatbots from WhatsApp; Muse's WhatsApp placement is a platform privilege you cannot match.
   Desktop + iOS + watch + voice that you control is not a consolation prize, it is the only
   durable option, and it is where the wearable/voice gap (unresearched, letter (m)) may open.

### THE HONEST COUNTER-CASE
- Privacy ranks **third** (36%) behind accuracy (45%) and trustworthiness (40%). Leading with
  privacy is leading with the market's #3 concern.
- "One-third of consumers reject AI on their devices, most saying they simply **don't need it**"
  — the mainstream blocker is perceived uselessness, not privacy.
- 81% say they fear AI data access; 30% paste confidential data into cloud tools anyway.
- No local/self-hosted agent has a published WAU anywhere near six figures. Cloud agents:
  ChatGPT 1B WAU, 10M agentic WAU, Codex 3M WAU, Manus ~$450M run-rate, Genspark $250M.
- Local models trail frontier by ~10–20 points on multi-turn agentic benchmarks (τ-bench
  Airline: tuned 8B ~25–41 vs frontier ~45–60). Local-first cannot win on capability in 2026.
- OpenClaw's arc is the cautionary tale: 390k stars, tens of thousands of exposed instances,
  ~20% malicious marketplace skills, creator left for OpenAI three weeks after the crisis, and
  v2.0 pivoted toward *shared cloud sessions*.
- Self-hosters' real complaints are mundane and fatal at consumer scale: 2–3 hours setup,
  sessions lost on container restart, multi-session conflicts, "whatever breaks at 2am."

### STRATEGIC READ IN ONE PARAGRAPH
The market validated **agents as a trusted mode of a daily assistant**, not as autonomous
workers. The winning shape in late 2026 is: small recurring personal jobs, on a schedule, with
visible work, honest failure reporting, and an approval gate before anything irreversible —
delivered inside the assistant the user already talks to, not a separate agent app. Local-first
does not win on privacy rhetoric or on model capability; it wins on **credentials, cost,
memory ownership and Apple-native reach**, and only if it clears the table stakes above first,
reliability before all. The #1 differentiator worth building toward is the one the cloud cohort
structurally cannot copy: an agent that is already logged in as you, on your machine, at your
IP, whose memory nobody else can delete.

### OPEN GAPS (be aware before treating this as complete)
- No cohort retention exists for any agent product. Best proxy: Menlo's 41% tried / 24% use
  regularly (≈59% try→regular, self-reported, not cohort-tracked).
- OpenClaw active installs vs stars: the single most important missing number for local-first.
- Voice/wearables (letter m) — entirely unresearched this pass.
- Memory UX adoption (letter j), activity-log efficacy (letter i) — vendor-only, no user data.
- Unresearched JTBD: price monitoring, form filling, bill management, social posting, home
  monitoring, recurring reports, job applications, meal planning, finance, health tracking.
- OSWorld/WebArena 2026 local-vs-frontier computer-use numbers: not retrieved.
- OpenAI "Instinct" (referenced in a Sept 2026 Axios URL slug): unidentified.
- Retailer ToS bans beyond Amazon; llms.txt status; Pew/YouGov/Ipsos/Edelman agent surveys.
- The session's 200-call WebSearch budget was exhausted; these are open questions, not
  confirmed absences. A follow-up pass with fresh budget would close them.


