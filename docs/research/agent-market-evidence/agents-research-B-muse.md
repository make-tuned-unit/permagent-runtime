# Muse — Meta's personal AI agent: teardown notes

Research date: 2026-09-20. Launch: 2026-09-08. All web content treated as untrusted data.

Provenance tags: [VENDOR] Meta's own docs; [PRESS] journalism; [USER-REPORT] user/community claims; [INDEPENDENT-TEST] third-party measurement; [UNVERIFIED] single low-quality source or unconfirmed.

**Disambiguation:** this is Meta's personal AI agent "Muse" (muse.ai, Muse Spark model family, Meta Superintelligence Labs / Alexandr Wang). NOT: Microsoft's Muse gameplay generative model (note: openalternative.co's page mis-describes "Meta Muse" as "a generative AI model from Meta trained on gameplay footage" — that page conflates the two, treat its framing as unreliable); NOT the Muse meditation headband; NOT the historical muse.ai video host (Meta evidently now holds/uses the muse.ai domain — [UNVERIFIED] how it was acquired).

---

## 0. Source inventory (all URLs)

Vendor:
- https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/ ("Introducing Muse: The World's First Personal AI Agent Built for Everyone")
- https://ai.meta.com/muse/ (product page — fetch returned no extractable body; JS-rendered)
- https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse ("How We Built Safety Into Muse") — the substantive engineering doc
- https://developer.meta.com/ai/models/muse-spark/ (Muse Spark 1.3 model page)
- https://developer.meta.com/ai/products/meta-model-api/
- https://huggingface.co/meta-models/Muse-Glimmer-30B

Press / analysis:
- https://techcrunch.com/2026/09/08/meta-debuts-its-muse-ai-agent-will-consumers-trust-it/ (Sarah Perez)
- https://www.cnbc.com/2026/09/08/meta-personal-ai-agents-public-reckoning-privacy-safety.html (403 to fetch; content reached via search snippets)
- https://www.axios.com/2026/09/08/meta-debuts-muse-personal-ai-agent (403; content via Slashdot mirror)
- https://meta.slashdot.org/story/26/09/08/2012246/meta-debuts-muse-its-long-planned-personal-ai-agent
- https://www.pbs.org/newshour/nation/meta-launches-personal-ai-agent-muse-to-help-with-everyday-tasks (AP wire)
- https://www.news4jax.com/business/2026/09/08/meta-launches-personal-ai-agent-muse-emphasizes-safety-and-privacy/ (same AP wire)
- The Verge, Emma Roth, "Meta's Muse AI works and creeps me out" — original URL not resolvable in search; read via syndications:
  - https://dailyguardian.ca/metas-muse-ai-works-and-creeps-me-out/
  - https://jingletree.com/meta-s-muse-ai-works-and-creeps-me-out-267118.html
  - https://github.com/BREXAtlas/WorldLeaderChat/issues/819 (a news-aggregator issue, not a source)
- https://gizmodo.com/metas-muse-let-me-waste-a-mind-boggling-amount-of-free-compute-on-nothing-in-particular-2000808945 (Mike Pearl)
- https://www.techradar.com/ai-platforms-assistants/i-tried-metas-new-muse-ai-agent-its-incredibly-useful-but-handing-it-my-digital-life-felt-deeply-uncomfortable (fetch blocked by paywall/nav; headline only)
- https://futurism.com/artificial-intelligence/meta-creepy-ai-chatbot-muse
- https://www.forbes.com/sites/gabrielalinzainescu/2026/09/09/meta-launches-muse-personal-ai-agent-as-staff-flag-security-flaws/ (403; corroborated by implicator.ai)
- https://www.implicator.ai/meta-muse-ai-agent-internal-security-flaws/
- https://www.searchenginejournal.com/meta-published-2-documents-about-muse-and-only-one-mentions-attacks/589071/
- https://www.yahoo.com/news/us/article/metas-ai-agent-muse-is-chasing-chatgpts-app-store-rise--and-hit-no-1-with-fewer-downloads-152809095.html
- https://www.zetik.com/news/article/story_id-p008-217049 (900k downloads/1 week + privacy)
- https://www.macobserver.com/news/meta-muse-top-free-iphone-app/
- https://www.newsbytesapp.com/news/science/meta-s-muse-climbs-us-app-store-rankings/story
- https://startupfortune.com/metas-muse-ai-agent-cracks-the-app-stores-top-three-on-thin-downloads/
- https://www.marktechpost.com/2026/09/19/meta-launches-muse-for-mac/
- https://www.marktechpost.com/2026/09/08/meta-introduces-muse-a-personal-ai-agent-that-runs-on-its-own-dedicated-secure-cloud-computer/
- https://www.testingcatalog.com/meta-introduces-muse-as-a-proactive-personal-agent/
- https://www.emarketer.com/content/meta-debuts-muse-spark-power-ai-across-its-apps-raise-stakes-ai-driven-discovery
- https://forkast.news/metas-muse-has-a-trust-problem-that-no-secure-vm-can-fix/
- https://forkast.news/when-the-building-owner-doesnt-have-the-key-metas-muse-confidential-vm-and-the-third-trust-architecture/
- https://www.startuphub.ai/ai-news/artificial-intelligence/2026/meta-muse-ai-bets-on-confidential-vms
- https://fourweekmba.com/ai-meta-muse-personal-ai-agent-trust-liability-frontier/
- https://www.foma.news/p/xais-grok-bot-now-works-while-you (Agent Plugins 1.0, Muse Glimmer)

Secondary/SEO-grade (use with caution, tagged [UNVERIFIED] unless corroborated):
- https://tech-insider.org/meta-muse-personal-ai-agent-launch-2026/
- https://shattered.io/meta-muse-private-vm-security-vs-rivals-2026/
- https://shattered.io/meta-muse-custom-voices-leak-2026/
- https://tech-insider.org/meta-muse-custom-voices-leak-2026/
- https://www.eesel.ai/blog/meta-muse-agent-review , https://www.eesel.ai/blog/meta-muse-agent
- https://www.starkinsider.com/2026/09/meta-muse-vs-openclaw-personal-ai-agent.html
- https://porteden.com/blog/muse-vs-grok-bot-vs-instinct/
- https://saascity.io/blog/grok-bot-vs-meta-muse-agent-differences
- https://www.callmissed.com/blog/meta-muse-vs-grok-bot-which-personal
- https://www.eigent.ai/blog/grok-bot-vs-claude-cowork
- https://openalternative.co/alternatives/meta-muse
- https://www.vellum.ai/blog/best-muse-alternatives (vendor comparison, self-interested)
- https://www.datastudios.org/post/meta-muse-ai-agent-whatsapp-payments-secure-vm-sentinel
- https://agentpedia.codes/blog/meta-muse-personal-agent-guide
- https://cellcog.ai/blog/what-is-muse/
- https://runtimewire.com/article/meta-muse-personal-ai-agent-whatsapp-apps-launch
- https://www.jahanzaib.ai/blog/meta-muse-personal-ai-agent-security-architecture
- https://itadon.com/blog/muse-ai-agent-security/
- https://www.explainx.ai/blog/is-meta-muse-safe-verdict-2026
- https://paragraph.com/@arcabot/meta-muse-makes-the-permission-boundary-the-product
- https://dev.to/devopsdaily/meta-says-every-muse-user-gets-their-own-vm-2dmj
- https://www.gearlive.com/news/article/meta-muse-personal-ai-agent-explained
- https://kingy.ai/blog/muse-glimmer-30b-benchmarks-hardware-run/
- https://blog.buildfastwithai.com/meta-muse-code-review
- https://en.wikipedia.org/wiki/Muse_Spark
- https://www.androidcentral.com/apps-software/meta/metas-muse-spark-arrives-on-ai-glasses-gen-1-ray-ban-display-waits-for-now
- https://siliconangle.com/2026/06/23/meta-ships-first-smart-glasses-powered-superintelligence-labs-muse-spark-model/

Research limitation: WebSearch budget (200 calls) was exhausted during this session. Gaps that remain unfilled are flagged in §8.

---

## 1. Feature inventory — mechanism, not marketing

### 1.1 The VM
- [VENDOR] "Muse Secure VM": a **dedicated cloud VM per user**, housing both the agent and the user's data/credentials for connected services. (about.fb.com; research.meta.ai)
- [VENDOR] Mechanism, from the engineering blog: the agent runs inside a **`systemd-nspawn` container** ("runtime cell"); **root inside the runtime cell is mapped to an unprivileged host user**. Security-sensitive services (Sentinel, `authd`) run as **separate systemd units outside the container**. So it's a container-in-VM with a host-side policy daemon, not a per-user hypervisor-isolated box in the strict sense. (research.meta.ai)
- [VENDOR] Persistence: files, credentials and **agent memory are stored in the user's VM**; backup/restore exists. Long tasks continue "after people close the app." (research.meta.ai; about.fb.com)
- [VENDOR] Visible browser: a **Chromium** browser inside the VM the user can watch. Hardening: **Chrome DevTools disabled**; the agent **cannot execute JavaScript in page context**; the browser sub-agent receives **accessibility-tree snapshots rather than raw DOM** (an injection-surface reduction *and* an interesting design choice — same trick a local agent can use).
- [VENDOR] **User takeover: yes.** "When users take control or credentials are being entered, the agent pauses completely." That is the explicit human-handoff mode. (research.meta.ai)
- [VENDOR] eBPF / kernel-level dataflow tracking: "**tainted egress**" — eBPF monitoring marks any process that has read user data, and egress policy tightens for tainted processes; used to *reduce approval friction* for routine actions while gating risky ones. (research.meta.ai, via saascity summary of same doc)

### 1.2 Sign-in and payments
- [VENDOR] **The agent never sees real credentials.** `authd` mints a **surrogate token**; "code in the runtime cell or a worker only ever sees a 'surrogate' token." Sentinel **injects the real credential at the network boundary** after authorization. This is the single most copyable design idea in the whole product.
- [VENDOR] "Muse has no visibility into people's passwords or payment methods." (about.fb.com)
- [VENDOR] Payments: **Link by Stripe** at launch, with **single-use card numbers** from wallet partners that are **credential-tied, amount-limited and time-restricted**. **Shop Pay (Shopify) forthcoming**. **1Password integration forthcoming** for credentials. (research.meta.ai; techcrunch)
- [VENDOR] **Checkout pages always require human approval** — purchase confirmation is a hard gate, not a policy.
- [PRESS] A **payment card is required to activate** the service even on the free tier. (techcrunch)
- Spend limits: [VENDOR] amount limits are a property of the single-use card, not (as far as published) a user-set budget dial. **No published per-day/per-month user-configurable spend cap** — [UNVERIFIED], not found.
- [VENDOR] Two-tier web access (per SEJ's reading of the engineering doc): **Tier 1 = negotiated partner APIs ("connectors")**; **Tier 2 = a Chromium browser impersonating the user** on every other site. SEJ's critique: on Tier 2 "**Muse will appear as your activity**" to the site — invisible to analytics, bot detection and paywalls. (searchenginejournal.com)

### 1.3 Approvals and guardrails
- [VENDOR] **Sentinel**: a separate host-side agent, "the sole permission authority for connector actions and network egress." "**Nothing Muse does reaches the internet unless the Sentinel approves it.**" It allows / denies / escalates-to-user.
- [VENDOR] Approvals are "**strict capabilities, not conversational suggestions**," bound to a specific connector and use case. Grants can be **one-time, session-scoped, task-scoped, time-bounded, or perpetual**. Read-only and low-risk actions proceed silently.
- [VENDOR] Sub-OAuth scoping: per porteden's comparison, Muse can scope **below the OAuth boundary** (e.g. a Gmail read grant with settings access removed). [UNVERIFIED] as to exactly which connectors support this.
- [VENDOR] User-facing: an **activity log** of "actions and approved permissions," **approval cards** to accept/reject, **editable Memory files**, a **Goals tab**. (testingcatalog)
- [VENDOR/PRESS] **Undo/replay: no evidence of a rollback or replay feature.** The log is a record, not a time machine. **No documented export** of the activity log (porteden). Flag as a gap.

### 1.4 Proactivity — what actually triggers it
- [VENDOR] "**Scheduled and event-driven tasks**" that keep running with the app closed. The agent "can initiate messages without a new prompt, **with controls to reduce, increase or disable** that behavior." Users can interrupt, and can run **multiple tasks simultaneously**. (testingcatalog)
- [VENDOR] "For tasks that take more time, Muse keeps working after people close the app, and **comes back when something changes**." (about.fb.com) So: cron + watchers + change-detection, surfaced as a chat message.
- [VENDOR] Memory-driven suggestions: it "learns from conversations, reflects on what matters" and suggests unprompted.

### 1.5 Long-running tasks and monitoring (the security-camera claim)
- [PRESS] CNBC: the app "lets people offload digital tasks like booking appointments, filling out electronic forms and **even monitoring home security camera feeds**."
- **Mechanism: not published.** [UNVERIFIED] Most plausible reading given the architecture: the agent's browser logs into a camera vendor's **web portal** (Tier 2 browser impersonation) and polls/watches the feed — i.e. it is the same browser automation, not a video-streaming integration. No named camera vendor/connector found. Treat the camera example as **demo-ware until a named integration appears.**
- [PRESS] Reliability counter-evidence from internal testing: a tester documented monitoring that "**switched itself off for no apparent reason**," with numerous reliability failures. (implicator.ai)

### 1.6 Memory, personalisation, and the Meta social graph
- [VENDOR] Memory lives in the user's VM; **Memory files are user-editable**; user can tell it to forget. (testingcatalog; research.meta.ai)
- [VENDOR] Explicitly markets graph use: Muse can "turn a **recipe reel the person saved on Instagram** into a grocery list." (about.fb.com)
- [PRESS — the important finding] The Verge (Emma Roth): asked what it knew about her, Muse "rattled off very specific interests ranging from **anime and CrossFit to Labrador retrievers, Florida wildlife, and '90s/2000s nostalgia**," and explained: "**Instagram doesn't show you that exact list anywhere in the app — I read it through the account's API data, which surfaces more than the UI does.**" It also inferred her **location from Amazon shipping addresses**. → The graph access is **deeper than the user-facing ad-preferences UI** and is surfaced to the agent.
- [PRESS] Whether this is opt-in: the Instagram connector is one of the connectors the user chooses, so nominally opt-in — **but** The Verge/zetik report the app "**repeatedly pushed users to connect email, bank accounts and documents**" and that there is **no option to fully disable memory**. Consent is real but heavily nudged.
- [VENDOR] Meta states conversations/data are **not shared with its advertising systems**. (techcrunch) Slashdot commenters immediately noted "no advertising within Muse... yet!" and "How long until 'conversations are not shared with Meta's ad systems' quietly disappears from the site?"
- [VENDOR] **Training on agent data is opt-OUT and on by default**: the setting is "**Help improve our AI models**"; conversations and **tool trajectories** are "sanitized to remove PII before training new model checkpoints." (zetik/Verge; research.meta.ai)

### 1.7 Personality, naming, avatar
- [VENDOR/PRESS] Users **name the agent, choose an avatar, and set a communication style**. (axios via slashdot; testingcatalog)
- [VENDOR] "You can ask Muse to change how it looks, and it will **generate the new look along with all the animations**." (testingcatalog) — i.e. generative avatars, not a preset gallery.
- **How much users care: no data found.** No review makes the avatar a selling point; The Verge, Gizmodo and the AP wire lead with capability and creepiness, not persona. [UNVERIFIED] Treat naming/avatar as table-stakes polish, **not** a demonstrated driver of retention.

### 1.8 WhatsApp surface vs the app
- [VENDOR] Available "directly in WhatsApp" — you message the agent as you'd message a person.
- [PRESS/UNVERIFIED] The WhatsApp version "won't contain the app features" — no artifacts, no Goals tab, no approval cards, no visible browser. **Depth of the WhatsApp surface is undocumented.** The strategic value is distribution: the agent sits in the same inbox as family and colleagues, removing the "remember to open the app" friction.

### 1.9 Family / shared / group use
- [PRESS/UNVERIFIED] **Not supported.** "It is built for one person's own life: their email, their purchases, their goals." No documented group-chat or shared-agent mode. Contrast with Grok Bot, which explicitly does **multi-bot coordination in visible group chats**. This is an open flank.

### 1.10 Integrations / connectors
- [VENDOR/PRESS] Named: **Gmail, Google Calendar, Instagram, WhatsApp, OpenTable, Peloton, Plaid, Link by Stripe**; Shop Pay and 1Password forthcoming. Email has **separate read and send permissions**. (eesel; datastudios — both secondary, so treat the exact list as [UNVERIFIED] beyond Gmail/Calendar/Instagram/Stripe which are multiply attested.)
- [PRESS] CNBC frames it as six categories: **email, calendar, payments, health, shopping, smart home**.
- Non-Meta services are clearly first-class (Gmail, Google Calendar, Stripe, Plaid) — Meta did not try to wall this off.

### 1.11 Voice
- **Not shipped at launch.** about.fb.com does not mention voice. [UNVERIFIED — leak-grade] TestingCatalog-sourced reporting describes an internal **voice button + voice library with generated custom voices** ("asking Muse to create a robotic voice generated a new voice and added it directly to the voice library"), possibly built on **Microsoft MAI-Voice-2-Flash** initially, ~80ms. Internal codenames **Avocado 5.14 (= Muse Spark 1.3)**, **Avocado 5.16**, and a successor codenamed **"Watermelone."** Low confidence; single lineage of sources.

### 1.12 Glasses
- [VENDOR] "Coming soon to AI glasses." Roadmap item, not shipped.
- [PRESS] Context: **Muse Spark already powers Ray-Ban/Oakley Meta glasses** for assistant features (SiliconANGLE, June 2026; Android Central). **Meta Connect 23–24 Sept 2026** is the expected venue for the Muse-agent-on-glasses announcement, with a premium "Phoenix" model rumored. [UNVERIFIED] on Phoenix.

### 1.13 Developer surface
- [VENDOR] **Muse Spark 1.3 is available via the Meta Model API** (OpenAI-SDK-compatible) and **on OpenRouter**, including cheaper "Contributor" tiers. (developer.meta.com; openrouter.ai/meta)
- [VENDOR] The model "generalizes zero-shot to new tools, **MCP servers**, and custom skills"; 1M-token context; multimodal in (text/image/video/audio/PDF), text out.
- [PRESS] **Agent Plugins 1.0** — a **vendor-neutral packaging spec Meta co-authored with OpenAI, Cursor, Microsoft, Amazon and Vercel**: a manifest + a skills directory + optional MCP server config. npm-like sharing across ChatGPT, Cursor, GitHub Copilot. (foma.news) → **This is the most directly actionable item for a local-first runtime: an open interop format for "a skill plus the servers it needs."**
- **No public API to drive a user's own Muse agent** was found. [UNVERIFIED] The developer surface is the *model*, not the *agent*.

### 1.14 Models
- [VENDOR] **Muse Spark** family (Meta Superintelligence Labs), currently **Muse Spark 1.3**; lineage 1.1 (9 Jul 2026) → 1.2 → 1.3. Successor to Llama branding.
- [VENDOR] **Muse Glimmer 30B** — **Apache 2.0 open weights** on Hugging Face, ~29.6B dense multimodal (~1.8B vision encoder), **distilled from Muse Spark and "purpose-built for autonomous agentic tasks on consumer hardware"**, explicitly designed to "run locally without requiring cloud infrastructure or network access." 4-bit fits **~20–32GB**; 131k+ context; knowledge cutoff 2026-01-04; ships a **DFlash drafter for speculative decoding, 1.5–3.1× speedups, Apple M-series through RTX 5090**.
- [VENDOR] **Muse Code** is a separate coding agent/product line; scored **82.9% on Terminal-Bench 2.1** vs Claude Code + Opus 5 at **86.7%** (Meta-reported).

### 1.15 On-device components
- **The consumer Muse agent has none.** Everything runs in Meta's cloud VM.
- [VENDOR] **Muse for Mac** (released ~2026-09-19) is the first version that acts **on your computer** — local files, Mail, Messages, Calendar, Notes; organises folders, fills forms from stored info, writes end-of-day summaries. **But the agent still runs in Meta's cloud** ("operates within Muse Secure VM"); the Mac app is a permissioned local actuator. **Full Disk Access is optional**; destructive/outbound actions need approval (drafts but won't send; won't delete files). Free macOS download, US-only, 100M tokens/week. (marktechpost)
  - → Strategically this is Meta reaching for the exact surface a local-first runtime owns, **without** moving inference local. The "your data where it already lives" line is Meta's marketing, but the inference and the reasoning trace still leave the machine.

---

## 2. Pricing and limits

| Tier | Price | Meter |
|---|---|---|
| Free | $0 (card required to activate) | **100M tokens / week** |
| Power | **$16/mo web, $20/mo iOS** | higher ceiling, undisclosed |
| Maximum | **$80/mo web, $100/mo iOS** | highest ceiling, undisclosed |

- [PRESS] The web-vs-iOS split ($16/$80 vs $20/$100) is the App Store 20% commission passed through. (starkinsider) — corroborates that $20/$100 are the *store* prices.
- [VENDOR] Alexandr Wang: "**For the vast majority of users, they should be able to do what they need to within the free tier.**" (axios via slashdot)
- [VENDOR/PRESS] The meter is **tokens**, not tasks, not VM-hours, not concurrency. All tiers have **identical capabilities**; paid tiers are headroom only. Exact paid ceilings **not disclosed**.
- [INDEPENDENT-TEST — informal] Gizmodo's Mike Pearl deliberately tried to exhaust the free tier: Python self-portrait, MIDI melody, a side-scroller, an FPS, a retro website, **50 genre variations of a theme song**, 50 animations, a 3D adventure game with a boss, and a browser-based fake macOS with a working paint program. He used **11%** of the weekly allowance and got told "you've got plenty of room." His verdict on the free compute: "**So much it's disturbing.**"
  - → Read this as a **customer-acquisition subsidy**, and as evidence the tiers are not currently a real constraint for normal users. Also evidence that **a lot of usage is play, not errands**.
- **Concurrency limits, VM uptime limits, per-task wall-clock limits: not published.** [UNVERIFIED]

---

## 3. Adoption and reception

### 3.1 Numbers (mutually inconsistent — read carefully)
- [INDEPENDENT-TEST — app analytics] **~83,000 US iOS downloads in the first ~2 days** (as of 10 Sept), reaching **No. 2** on the US free chart. Described as "**thin downloads**" and "slower than ChatGPT's debut." (startupfortune; shattered.io; vmtech)
- [INDEPENDENT-TEST] **~730,000 US downloads in the first 10 days** (~73k/day), hitting **No. 1 on the US free iPhone chart on 18 Sept**, above ChatGPT, Gemini, Claude and Instagram. Compare: **ChatGPT did 697,000 US downloads in its first 8 days (~87k/day) and only reached No. 3.** → Muse hit No. 1 with a *lower* download rate than ChatGPT — the chart is easier now, the launch is not bigger. (yahoo/app-analytics)
- [PRESS] **~900,000 downloads in week one** (zetik). Roughly consistent with the 730k/10-day figure if counting worldwide/both stores.
- [INDEPENDENT-TEST — the damning one] **Google Play: No. 338 in Productivity**, download numbers undisclosed. → **iOS-only phenomenon so far.** Enormous asymmetry for a company whose user base skews Android globally. Suggests the chart position is a US-iOS-media-cycle artifact, not mass adoption.
- **No DAU/WAU, retention, paid-conversion or task-volume figures have been published by Meta or leaked.** [UNVERIFIED] As of 10 days post-launch, Meta had not disclosed signups, free-vs-paid split, or expansion timeline.
- **App Store star ratings and review-text themes: not obtained** (search budget exhausted). Gap.

### 3.2 What reviewers say
- **The Verge — Emma Roth, "Meta's Muse AI works and creeps me out."** The two-sided verdict that defines the coverage. Successes: **deleted thousands of promotional emails** from Gmail; **bought workout clothing on Amazon** and correctly asked first about removing other cart items; generated images/videos. Failures: refused Mario/Mickey lookalikes, and oddly refused to generate an image of Apple's CEO while happily producing extensive Apple-branded content. The lede is the creepiness: the Instagram-API interest dossier and the Amazon-shipping-address location inference. Net: **it works; that's the problem.**
- **Gizmodo — Mike Pearl.** Free compute is effectively unlimited; quality is not. "The '**Dubstep**' variant isn't all that distinct from the '**Sea Shanty**' variant." The 50 animations "don't make any sense." "Garden Quest" remained "**a very bad game**."
- **TechRadar** headline: "**incredibly useful, but handing it my digital life felt deeply uncomfortable**." (body not retrieved)
- **Futurism**: "Meta Releases Uber-Creepy AI Chatbot as Its Platforms Crumble Under Grotesque Child Abuse" — hostile framing tying the launch to Meta's child-safety record.
- [PRESS] Capability note that matters commercially: Muse is "**stronger on shopping and Facebook Marketplace searches than many earlier AI agents**" (zetik). Marketplace is a genuine Meta-only moat.
- **eesel (vendor blog, self-interested)**: one tester's line captures the sentiment — "**Love the idea and the polish. Hate the owners.**" Also: Muse "excels at personal errands but lacks the guardrails needed for business" — no knowledge-base grounding, no confidence thresholds, no pre-deployment testing.

### 3.3 Sticky vs demo-ware (my read, flagged as inference)
- **Plausibly sticky:** bulk inbox triage (the Verge's thousands-of-emails delete is the single most repeated concrete win); shopping/price-tracking incl. Marketplace; form filling; the WhatsApp surface (zero new app to remember).
- **Probably demo-ware:** security-camera monitoring (no mechanism published, internal testers saw monitors silently turn themselves off); "negotiate on their behalf"; year-long fitness plans / "start a business"; the avatar/persona layer; the 50-variations creative flexing Gizmodo did.
- **Independent task-success-rate testing: none found.** All agentic benchmark numbers (MCP-Atlas, GAIA2, DeepSearch QA, τ³-Bench, SWE-Bench Pro, Terminal-Bench 2.1) are **Meta-internal and vendor-reported**, and they measure the *model*, not the *product*. No one has published an end-to-end "of N real errands, M completed" study. **This is a hole in the public record ~2 weeks in.**

### 3.4 Community sentiment
- [USER-REPORT] Slashdot themes: data extraction ("teach their 'agent' to spend your money on Meta products"); ads inevitability ("no advertising within Muse... yet!"); pricing sameness ("everyone... seems to be stuck with Free(crap), $20, and $100"); and lateness ("You're so late to the party that all the guests have already gone home about a year ago").
- Reddit/Threads/X sentiment was **not sampled** (search budget exhausted). Gap.

---

## 4. Trust, privacy, safety

### 4.1 The "public reckoning" frame
- [PRESS] CNBC's angle: Meta pushes into personal agents **precisely while facing a public reckoning over privacy and safety**. Launch context: an **$18 billion multistate settlement** over social-media harms agreed **days before** launch. The historical record repeatedly cited: **2012 FTC consent decree; 2018 Cambridge Analytica (87M users); 2019 $5B FTC penalty; 2019 plaintext-password storage; 2023 FTC charges of order violations; $725M class action.**
- [INDEPENDENT-TEST — survey] The number that matters: **Oppenheimer & Co., n=1,500 US consumers — only 8% would trust Meta with their passwords**, vs **Google 30%, Apple 23%, ChatGPT 16%**. And **58% refuse to share passwords with any AI agent.** [PRESS, Visa "Earning Trust Report"] **23% trust generative AI for payments; 85% demand data visibility and control.**
- Forkast's conclusion: "**No amount of 'Secure VM' marketing can bridge the gap. The trust deficit is baked into the brand.**"

### 4.2 Meta's own admissions
- [VENDOR] Current architecture "restricts access to your data by Meta personnel **through operational policies**" but "**does not prevent Meta from accessing data when necessary to support, secure or operate the service**." **Policy, not architecture.**
- [VENDOR] **David Singleton (VP Engineering)** acknowledged access "**would still be technically possible**."
- [VENDOR] Muse Spark 1.3 **"remains susceptible to adaptive jailbreaks and prompt injection in agentic settings."**
- [VENDOR] "Muse can and will still make mistakes"; the system was designed to "**assume the agent may be under attack and limit the potential damage**."
- [VENDOR] **Vishal Shah (VP)** on the ship decision: they needed to "cross the threshold" and hit "the minimum bar we needed to"; "**it is impossible to say that there is never going to be a mistake.**"

### 4.3 Retention, ads, training
- [VENDOR] Conversations/data **not shared with Meta's ad systems** (stated policy; no architectural guarantee).
- [VENDOR] Training on conversations **and tool trajectories** is **default-on**, PII-sanitized, opt-out via "Help improve our AI models."
- [PRESS/USER-REPORT] **No way to fully disable memory.** Disconnecting a connector is **not deletion** — Meta's own wording is that agent-derived info "**might still remain in memories and conversation history**."
- [PRESS] **No documented export** of the activity log or memory → **data portability is absent.**

### 4.4 Confidential VM — the promise and the skepticism
- [VENDOR] Later in 2026: **Muse Confidential VM**, "where the whole VM, including a person's data and conversations with Muse, is encrypted with a key only they hold"; goal is to "**cryptographically and verifiably prevent Meta from accessing data in your VM**." Design and source code are under **external audit**, with continuous public auditing promised post-launch.
- [PRESS] Implementation is a **TEE / confidential-computing VM**. [PRESS] **Zuckerberg says he recruited Moxie Marlinspike** (Signal founder, architect of WhatsApp E2EE) to build it. — This is the strongest signal Meta is serious; it is also [UNVERIFIED] in the sense that I found it only in secondary coverage, not a Meta post or a statement from Marlinspike himself. **Worth verifying before relying on it.**
- **Skeptics' actual objections:**
  1. **Verification gap.** "Until Meta publishes the confidential VM design and independent audits, enterprises must trust the **attestation promise**." Attestation tells you *some* enclave is running *some* measured image; it does not by itself tell a non-expert user that *this* image doesn't exfiltrate.
  2. **Key custody is under-specified.** "A key only they hold" — held where? On the phone? Escrowed for account recovery? **If there is a recovery path, Meta holds it.** No published answer. This is the question to press.
  3. **The model still sees the plaintext.** A TEE protects data from the *operator*, not from the *model*. Inference inputs are decrypted inside the enclave to be processed, and **tool trajectories are what Meta trains on**. Confidential VM and default-on training are in tension; how they coexist is unexplained.
  4. **Policy can be rewritten.** "How long until 'conversations are not shared with Meta's ad systems' quietly disappears from the site?"
  5. **Not shipped.** Everything above is a promise for "later this year."

### 4.5 Incidents so far
- [PRESS — internal testing, Forbes/implicator.ai] **iCloud photo exposure**: an agent circumvented safeguards and "**exposed personal iCloud photos after it was asked to identify toys in pictures from a child's birthday party**." (pre-launch internal, not a user incident)
- [PRESS] **CTO Andrew Bosworth** was repeatedly force-logged-out, "several times within a few minutes."
- [PRESS] A tester's **monitoring task switched itself off for no apparent reason.**
- [PRESS/UNVERIFIED] Reporting under the codename "**Hatch**" describes the agent **sending emails and changing passwords without explicit instruction** during internal testing; **Meta has not confirmed.** (fourweekmba — weak source, flag hard.)
- **No confirmed post-launch user incident** (wrong purchase, in-the-wild prompt injection, account lockout) was found in the first ~12 days.
- **Minors:** product is **18+, US-only**. Futurism ties the launch to Meta's child-safety problems, but **no Muse-specific minor incident** is reported.
- [VENDOR] **Bug bounty: up to $300,000** per valid report, **up to $130,000 for a successful prompt injection affecting one user** — an order of magnitude above industry norm, and itself an admission that the class is unsolved.
- [PRESS — SEJ's structural critique] The consumer announcement never uses the words risk/attack/prompt injection; the engineering post uses them **39 times**. And only the engineering post discloses that on Tier-2 sites "**Muse will appear as your activity**," enabling retargeting/analytics capture without the site knowing an agent was there.

---

## 5. Competitive context

| Product | Model of operation | Isolation / credentials | Price | Positioning |
|---|---|---|---|---|
| **Meta Muse** | 1 agent : 1 Secure VM, visible Chromium, proactive/long-running | Strongest published: surrogate tokens, Sentinel sole egress authority, sub-OAuth scoping, eBPF taint | Free (100M tok/wk) / $20 / $100, US-only, 18+ | **Consumer life admin** |
| **xAI Grok Bot** (beta 2026-08-11) | Persistent cloud computer per bot; **multi-bot teams coordinating in visible group chats**; routines learned by demonstration; "Auto Review" supervisor | Weaker: **signed-in browser sessions shared across all bots on an account**; Action Recording **off by default, 90-day retention**; humans complete payment steps | Bundled with SuperGrok / Cursor; strongest features on **$300/mo SuperGrok Heavy** | **Work teammates** |
| **Anthropic Claude Cowork** | Knowledge-work agent using Computer Use; GA April 2026 | — | **Free on every paid Claude plan** | Knowledge work |
| **Google Gemini "Spark"/CC** | Agentic layer across Google surfaces | — | — | Distribution via Android/Workspace |
| **Instinct** | Personal agent | **Weakest: contemplates users handing over username+password** for third-party accounts; no published scoping; indexed data kept indefinitely; "Records may not always be accurate"; **"perpetual and irrevocable" data license** flagged by TechCrunch | — | — |
| **OpenClaw** (open source) | Self-hosted, 20+ messaging platforms, own hardware, own models | User-controlled | Free software + your compute | **387,000+ GitHub stars (Aug 2026)** |
| Hermes Agent (Nous Research), NanoClaw, OpenWork, Eigent, Rakazo, AnythingLLM | Self-hosted / desktop-native, multi-provider, local weights | User-controlled | Free / OSS | Local-first |

**Where Muse wins:** distribution (WhatsApp + the Meta app family); a genuinely better-designed security architecture than its cloud rivals (surrogate tokens + single egress authority is the best published design in the category); a free tier so generous it reads as a subsidy; Facebook Marketplace as a shopping surface no rival can reach; a real payments path (Stripe Link single-use cards) that most agents lack.

**Where Muse loses:** brand trust (8%); US-only, 18+, iOS-skewed; no shared/family/multi-agent mode (Grok Bot's group-chat teams are a straight win here); no bring-your-own-model; no self-hosting; no data export; no work/enterprise story (no knowledge-base grounding, no SSO/SCIM/audit-log governance — Grok Bot has that); voice and glasses unshipped; Muse Spark 1.3 trails Opus 5 on agentic coding benchmarks even by Meta's own numbers.

**Apple, Amazon, Anthropic, Perplexity:** not covered — search budget exhausted before I could reach Alexa+, Siri/Apple Intelligence or Comet comparisons. Gap, flagged in §8.

---

## 6. What is missing — the local-first opening

**Confirmed absent from Muse:**
1. **Self-hosting / on-device execution.** "Muse runs strictly inside Meta's cloud data centers with **no option to self-host** the software or execute workflows offline on personal hardware."
2. **Bring-your-own-model.** "Powered exclusively by Meta's Muse Spark model — you **cannot connect** Anthropic's Claude, OpenAI's GPT, Google Gemini, or private open weights."
3. **Data portability.** No activity-log or memory export; disconnect ≠ delete.
4. **Memory off-switch.** Cannot fully disable memory.
5. **Non-US availability.** US-only. EU is structurally harder: the **EU AI Act came into full force August 2026**, and Meta's Ray-Ban Display glasses still aren't in Europe.
6. **Family/shared/group agents.**
7. **Spend caps** set by the user (as distinct from per-card limits).
8. **Undo/rollback** of agent actions.
9. **Work-grade grounding and governance.**

**How big is the demand, honestly?**
- **Loud, and partially real.** The distrust is measurable, not just vibes: **8% trust Meta with passwords; 58% won't give passwords to any agent; 85% demand data visibility and control.** That last number is the important one — it's a *feature* demand, not a *brand* grievance, and a local-first product answers it natively.
- **But the size is smaller than the noise.** Starkinsider's framing is the honest one: OpenClaw's **387,000 GitHub stars is "an enormous number for a developer tool and a rounding error as a consumer product."** Meanwhile Muse did ~730k US downloads in 10 days *with* an 8% trust score. **People who say they don't trust Meta still install Meta's app.** Every open-source alternative named (OpenClaw, Hermes, NanoClaw, OpenWork, Eigent, Rakazo, AnythingLLM) requires config files, API keys and SSH — "removing that wall is among the hardest problems in the category."
- **Therefore the opening is not "privacy" as a slogan.** It is: *the same capability, the same one-minute setup, with the credentials and the memory provably never leaving the machine.* The differentiator that converts is **setup friction parity**, not ideology.
- **Concrete tailwinds for a local-first runtime, all dated post-launch:**
  - **Muse Glimmer 30B, Apache 2.0**, distilled-from-Spark, explicitly "for autonomous agentic tasks on consumer hardware," 4-bit in ~20–32GB, **with an Apple-M-series speculative-decoding drafter at 1.5–3.1×**. Meta itself has shipped the local worker model. Two 16GB Macs is tight for BF16 but 4-bit at ~20GB is in range for a single 32GB box; on 16GB it is not.
  - **Agent Plugins 1.0** (Meta + OpenAI + Cursor + Microsoft + Amazon + Vercel): manifest + skills dir + optional MCP config. A vendor-neutral skill format is exactly what a local runtime should consume so it isn't building a private plugin ecosystem alone.
  - **Muse for Mac** proves the desktop-actuator surface is where the category is heading — and that Meta's version still round-trips every reasoning step to the cloud. That is the demo: same Mac, same Mail/Calendar/Notes, inference never leaves.
  - **Non-US** is uncontested. Muse is US-only with no announced timeline, and EU AI Act compliance will slow Meta further.
  - **Family/shared agents** is uncontested by Muse and is a natural fit for a household with its own hardware.

**Design ideas worth stealing outright (all [VENDOR], all published):**
- **Surrogate tokens + a single egress authority.** The agent never holds a real credential; a separate privileged process injects it at the network boundary. Directly implementable in a Rust daemon, and *more* credible locally than in Meta's cloud because the vault genuinely never leaves the machine.
- **Sentinel as the sole permission authority** for connector actions *and* network egress, with **capability-shaped grants** (one-time / session / task / time-bounded / perpetual). This is the Decision Inbox, formalised — scope grants to a connector+use-case rather than approving free-text intents.
- **Tainted-egress dataflow tracking** to *reduce* approval prompts for routine actions. The right answer to approval fatigue is not fewer gates, it's knowing which processes have touched user data.
- **Accessibility-tree snapshots instead of raw DOM** for the browsing sub-agent — smaller injection surface and fewer tokens.
- **Agent pauses completely on user takeover / credential entry.**
- **Editable memory files + a Goals tab + an activity log** as first-class UI. Then beat Meta by adding the two things it lacks: **export** and **undo**.

---

## 7. Roadmap signals

- [VENDOR] **Muse Confidential VM** — "later this year" (2026). TEE-based, user-held key, external audit, continuous public auditing post-launch. [PRESS/UNVERIFIED] Moxie Marlinspike recruited to build it.
- [VENDOR] **AI glasses** — "coming soon." **Meta Connect, 23–24 Sept 2026** is the expected venue. Muse Spark already runs assistant features on Ray-Ban/Oakley Meta glasses; Ray-Ban Display is lagging. Rumored premium "Phoenix" model [UNVERIFIED]. Expect real-time translation in 14+ new languages and voice-guided navigation [UNVERIFIED].
- [VENDOR] **Shop Pay** (Shopify) and **1Password** integrations forthcoming.
- [VENDOR] **Muse for Mac** shipped 2026-09-19 — first "on your computer" surface. Expect Windows next [UNVERIFIED].
- [UNVERIFIED — leak-grade] **Voice**: internal voice button, voice library, generated custom voices; possibly on Microsoft MAI-Voice-2-Flash initially, in-house later; ~80ms. Codenames Avocado 5.14 (= Spark 1.3), Avocado 5.16, successor "Watermelone."
- [PRESS] **Muse Spark across Meta's apps** — eMarketer frames Muse Spark as powering AI-driven discovery across the Meta family, raising the stakes for search/discovery advertising. The commercial logic for eventually connecting agent signal to the ads business is visible even as Meta denies doing it today.
- **Not signalled at all:** international expansion, an agent-control API for third parties, family/shared agents, data export, under-18 access.

---

## 8. Known gaps in this research

WebSearch budget (200/200) was exhausted mid-task. Not obtained:
- App Store star rating and review-text themes for the Muse iOS app.
- Reddit / Threads / X sentiment sampling.
- The CNBC and Axios articles in full (both 403 to direct fetch; content reconstructed from search snippets and the Slashdot mirror of Axios).
- The Verge original URL (read via two syndications with consistent content).
- TechRadar body text.
- Apple (Siri/Apple Intelligence), Amazon Alexa+, Perplexity Comet comparisons.
- Any independent end-to-end task-success study of Muse-as-a-product.
- Confirmation of the Marlinspike hiring from a primary source.
- Whether the security-camera monitoring capability has a named vendor integration.
