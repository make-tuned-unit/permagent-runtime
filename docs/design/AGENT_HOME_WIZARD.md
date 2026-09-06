# The Agent's Home — rebuilding first run as an agent-led experience

**Lane:** R4a (design doc only — no code, no builds in this PR)
**Status:** DESIGN. Untracked working-tree doc, not committed.
**Author:** design agent (audit-first).
**Decisions:** three of the eight open questions were **ruled by Jesse on 2026-09-01** and are
now folded into the body as decisions rather than options — the premise (§1.3), M1's liveness
and its on-device follow-up (§2 M1, slice 6), and the name-from-disposition reordering
(§2 M1 beats 2–3). Five remain open (§8), one of which — packs / `GOOSE_MODEL` — carries a
**provisional plan** that slice 2 must not ship until it is ruled.
**Convention:** every claim is tagged **[verified]** (read at the cited `file:line` in this
tree today) or **[assumed]** (inferred, or depends on a sibling lane not yet landed).
Copy blocks marked **DRAFT COPY** are the actual strings to ship, not paraphrase.

**Dependency:** a sibling lane is building `configure_app` — an approval-gated agent tool
that writes settings through the same daemon routes this UI uses, plus a `config_changed`
live-update event. Nothing named `configure_app` exists in this tree yet **[verified —
repo-wide grep: no matches]**. Every agent-performed step below is specified to degrade to
"the agent opens the pane and guides" when the write class is proposal-gated or the tool is
absent, and **slice 1 of the cut-list ships without it**.

---

## 0. TL;DR

- **The premise is already true in the code and has never been said out loud to the user.**
  `app_perception.rs:32-41` states the ruling verbatim — *"everything in the app is queryable
  by the agent, and the only way that stays true is if a new surface CANNOT ship without
  either gaining an aspect here or being explicitly exempted"* — enforced by a daemon
  coverage test (`every_shipped_tab_is_observable_or_exempt`). `observe_app("settings")`
  reads the whole config store (`app_perception.rs:1877`), `navigate_app` opens any tab and
  any Settings pane (`app_conductor.rs:250-310`, `settings/sections.ts:10-22`), and
  `load_feature_lesson` teaches a surface and navigates to it (`tour_tool.rs:26-82`). The
  wizard mentions none of this. **[verified]**
- **The wizard's whole shape is upside down.** The agent is introduced at step 6 of 8
  (`MomentMeet`), after its personality has already been configured at step 3
  (`MomentCalibration`) and after two screens of third-person product copy. The proposal:
  **agent first, in five moments**, name and voice bought in the first thirty seconds, so
  every later screen is a conversation with a someone.
- **Two machines are genuinely magical and are kept whole**: `MomentHardware`'s
  scan → recommend → auto-start-Ollama → pull-with-progress → verify (`MomentHardware.tsx:48-201`)
  and `MomentWebSearch`'s save → live-probe → honest-verdict (`MomentWebSearch.tsx:76-118`).
  They get a first-person copy pass and a conversational wrapper; their state machines are
  not touched. So does the identity round trip (`PUT /api/agent/identity` → `agent.yaml` →
  hot reload → `identity_changed`, `routes/identity.rs:52-95,93`). **[verified]**
- **Four capabilities are already built and simply never offered during setup.**
  `getProviders()` is called and its list ignored (`MomentWelcome.tsx:92-102` vs the hardcoded
  six at `:11-18`; the registry holds **30 always-registered providers plus 3 feature-gated**,
  `providers/init.rs:9-42`). `POST /config/providers/{name}/oauth` exists
  (`config_management.rs:1432`, mounted `:1661`) with **four** real implementations
  (`gemini_oauth.rs:952`, `kimicode.rs:468`, `githubcopilot.rs:530`, `chatgpt_codex.rs:1008`;
  `sovereign_guard.rs:278` is a passthrough) and **zero UI callers**. `RoleRoutingPrompt`
  already exists as a finished, self-gating component with three variants
  (`chat/RoleRoutingPrompt.tsx`) mounted in Chat, Settings and the Build statusline — but not
  in the wizard, which is the one place it would land *before* the user's first dollar.
  And the onboarding Brain seed already runs off `wizard_complete`
  (`config_management.rs:224` → `automation/onboarding_seed.rs`) — it just seeds static copy
  and has never carried the user's own stated intent. **[verified]**
- **The single worst write is `GOOSE_PROVIDER` + `GOOSE_MODEL`** at `MomentWelcome.tsx:150-155`.
  Precedence (`config/model_roles.rs:23-31`) puts the session model *above* the measured
  per-role defaults, so the one model the user picked in the first 40 seconds of the app
  silently becomes the model for every role. Fix: offer the existing pack routing in the same
  moment (§2.2, §4).
- **Completion is two unrelated booleans.** `wizard_complete` (written
  `WizardShell.tsx:102`, read `App.tsx:191` and `state.rs:780`) and `tour_completed`
  (written `tour_tool.rs:31`, read `reply_parts.rs:226`) know nothing about each other, and
  there is no re-run path anywhere **[verified — grep of `settings/` found only comments]**.
  §3 unifies them **without adding a third flag**.

---

## 1. The premise — "this is the agent's home"

### 1.1 What makes the claim honest (and therefore sayable)

A premise statement is only allowed if the app can cash it. It can. The three verbs map onto
three shipped mechanisms and one arriving one:

| Verb | Mechanism | Where | Status |
|---|---|---|---|
| **See** | `observe_app` — 20 aspects, coverage-tested against the shipped catalog; `settings` aspect reads the config store and *never* returns a secret value, not even masked | `app_perception.rs:32-65`, `:1877-1935`, test `observe_settings_never_emits_secret_values` `:2789` | shipped **[verified]** |
| **Query** | the catalog itself is injected into the system prompt, tab by tab, with `affords` / `suggest_when` | `app_catalog.rs:59-80` | shipped **[verified]** |
| **Tour** | `navigate_app` (tab + Settings `section` + `state`), `app_action`, `app_open_item`, `load_feature_lesson`; voice turns sequence the nav *after* the narration finishes | `app_conductor.rs:250-310`, `events/mod.rs:787-870`, `tour_tool.rs:26-82` | shipped **[verified]** |
| **Set up** | `configure_app` — writes through the same daemon routes the UI uses | sibling lane | **[assumed]** |

The honesty constraint, which must appear in the premise copy and not only in a doc: the
agent does **not** get to decide everything. Sovereignty, spend, secrets and provider changes
go to the Decision Inbox as proposals (`decision_inbox/sink.rs`, effects dispatched by
`(kind, action)` in `decisions_effects.rs:535+`). The premise is stronger for saying so —
"I'll change it, and the few things I'm not allowed to decide alone come back to you in
writing" is a better sentence than "I can change anything."

### 1.2 The house voice, extracted

Studied from the best existing copy — `MomentCode.tsx:138-144`, `MomentWebSearch.tsx:99-113`,
`MomentHardware.tsx:299-303`, `MomentCode.tsx:216-219`. The rules that produce it:

1. **First person, always.** "I'd rather ask than assume." Never "Connect a model provider."
2. **Name the failure mode, not the feature.** *"a wrong guess here doesn't fail loudly, it
   just quietly finds nothing."* The reason to answer is a specific bad outcome avoided.
3. **Two claims are never one claim.** *"The key is saved, but Tavily didn't answer."* Stored
   ≠ working; the copy separates them because the user cannot.
4. **Volunteer the awkward part.** *"One quick install first: Ollama runs the model on your
   machine — free and private."*
5. **Em-dash as the turn.** Statement — then the reason, in the same breath.
6. **No adjectives about itself.** Nothing is "powerful", "seamless", or "intelligent."

Every string below obeys these six.

### 1.3 The premise — "Moving in" **[Decided 2026-09-01 — Jesse]**

Three candidates were drafted; **B, "Moving in", is the premise.** A and C are preserved as
alternates in **Appendix A** — not as pending options, but because their phrasings are useful
raw material for the shorter agent lines elsewhere in the flow.

Spoken by the agent on the first screen, over a large `Mobius state="speaking"`, before
anything is asked of the user.

> **DRAFT COPY — the premise (ship this)**
>
> Hello. I should tell you where you are.
>
> This is my home — and you've just moved in with me. That's a fact about the plumbing, not
> a mood: I can read every surface in this app, walk you to any of them, and set most of
> them up myself. The parts I can't do alone — spending your money, changing what I'm
> allowed to touch, anything holding a secret — I'll ask for in writing first, and wait for
> your answer.
>
> Which means setup isn't a form you fill in. It's the first conversation we have.
>
> So — how do you want me to talk to you?

The closing line differs by one sentence from the drafted candidate. The draft ended *"Let's
start with what you'd like to call me"*; M1's internal order was subsequently reversed
(§2 M1, decided the same day) so that **disposition is asked before naming** and the agent
can then propose a name that fits the answer. The premise therefore hands straight into the
disposition question, which is also the better opener: it asks the user for a preference
rather than for an invention.

**Spoken variant.** If `speakReplies` is on for the arrival (see §8, Q1), the TTS line is
shorter than the screen text — reading four paragraphs aloud is a hostage situation:

> "Hello. This is my home, and you've just moved in with me. Which means setup isn't a form —
> it's the first conversation we have. So: how do you want me to talk to you?"

### 1.4 The three-verb strip

Under whichever premise paragraph ships, one compact row — because the claim needs to be
*demonstrable*, not just asserted. Three `Glass` chips, no icons:

> **See** — everything, including what's configured. Never your secret values.
> **Open** — any screen, any settings pane, by name.
> **Change** — most of it myself. Money, permissions and secrets come back to you first.

**[assumed]** — the third chip's promise is only true once `configure_app` lands. Until
then the chip reads: **Change** — I'll open the right pane and walk you through it.
This is the one place in the doc where slice ordering changes user-visible copy, and it must
change, because claiming a capability that isn't wired is exactly the failure this codebase
keeps writing comments about.

---

## 2. The flow — five moments

**Principles.**

1. **The agent exists before it is configured.** Name and voice are the *first* two decisions,
   not the sixth and seventh. Everything after is addressed to a someone.
2. **Mobius is the speaker, not decoration.** Today the wizard uses only `idle` and
   `calibrating` (`MomentWelcome.tsx:167`, `MomentCalibration.tsx:28`, `MomentHardware.tsx:224`)
   out of the five available states (`Mobius.tsx:4`). The new flow drives `speaking` while a
   line is delivered, `thinking` while a probe runs, `calibrating` only for the hardware scan,
   `idle` while waiting on the user. (See §6 for the reduce-motion consequence — it is a real
   regression risk.)
3. **A moment is a question, then a machine.** Never a form. Where a decision has more than
   two branches, the options are phrased as *replies the user could have said*, not as labels.
4. **Nothing claims success it hasn't proved.** Inherited from `MomentWebSearch`.
5. **Every moment is skippable and every skip is honest about what it costs.**

**Moments: 8 → 5.** Named, not numbered (see §5, ProgressDots).

---

### M1 — Arrival — *"Who am I?"*

*Replaces:* `MomentWelcome`'s framing, all of `MomentCalibration`, most of `MomentMeet`.

**Mobius:** large (160), `speaking` through the premise, then `idle`.

**Beat order — disposition *before* naming. [Decided 2026-09-01 — Jesse]** The drafted order
was name → voice → disposition. It is reversed: the user answers *how the agent should talk*
first, and the agent then **proposes a name that fits the answer it just got**. Two reasons.
The first is that a name is an invention and a disposition is a preference — asking for the
preference first is asking for something the user actually has. The second is that it turns
the naming beat into the flow's first act of agent *judgment*: the agent listens, infers, and
offers. That is the premise being demonstrated forty seconds in, rather than asserted.

**Beat 1 — the premise.** §1.3 copy, ending on the disposition question.

**Beat 2 — the disposition.** This is what replaces the four-card preset grid. One question,
four answers written as things the user would say, not as personality-template names:

> **DRAFT COPY**
> · *"Short. Don't explain unless I ask."*
> · *"Think out loud — I want to see the reasoning."*
> · *"Warm. Assume I'm learning this."*
> · *"Ship-first. Code over conversation."*

Each maps to the existing trait/tone tuples at `MomentCalibration.tsx:7-12` — *the data is
kept, the grid is not*. The agent **echoes the choice back**, which is the first proof that
the answer landed:

> **DRAFT COPY (example, "Short")**
> Short it is. I'll skip the preamble and tell you when I'm guessing.

**Skippable.** A *"doesn't matter — you pick"* option is present, and takes the same path as
answering: the agent chooses a disposition and says which one it chose. It never proceeds
silently on a default (rule 7, §6.3).

**Beat 3 — the name, proposed from the disposition.** Today the default name `'Aria'` is
assigned *silently* at `WizardShell.tsx:59-61` if the user never typed one — a persona the
user never agreed to. It becomes a **visible proposal derived from beat 2**, accepted or
replaced in one gesture:

> **DRAFT COPY (example, "Short")**
> Then I'd like to be **Vale**, if that suits you. It's short, and I'll answer to it
> instantly.
>
> **DRAFT COPY (example, "Think out loud")**
> Then I'd like to be **Ansel**, if that suits you — it sounds like someone who explains
> their working.

One `PrimaryButton` accepts (*"Yes — you're {name}."*); the name itself is an inline-editable
field prefilled with the proposal, so replacing it is one click and a keystroke, not a
separate screen. The agent's short reason for the proposal is part of the copy — a name
offered *with* a reason is a character introducing itself; a name offered without one is a
placeholder.

**Name proposal is a small static table, not a model call.** Each disposition carries a short
ranked list of candidate names (3–4 each, no overlap between dispositions), and the proposal
is the first one not already taken. This keeps beat 3 working with no provider configured
(§2, M1 liveness decision) and keeps it deterministic enough to test. **`Aria` is the fallback
proposal** — used when the user skipped the disposition question, so the current default name
survives as the honest no-signal answer rather than as a silent assignment.

> **DRAFT COPY (disposition skipped)**
> I'll go with **Aria** for now, then — no strong opinion behind it, so change it whenever
> one shows up.

**Beat 4 — the voice.** `VoicePicker` (reused unchanged), with a sample line the agent
speaks in each voice, in character — and now it has a name to say:

> **DRAFT COPY**
> And this is what I sound like. Pick one — or turn me off entirely; I'll stay in text and
> nothing else changes.

**Writes:** one `PUT /api/agent/identity` at the end of M1 — not at the end of the wizard.
The identity round trip is the one thing in the current wizard that hot-reloads across the
whole app (`identity.rs:74-93`), and doing it *here* means the header, the World nameplate
and the chat all already know the agent's name for the remaining four moments. **This is the
single highest-leverage reordering in the document.**

**Failure:** the existing save-failure banner from `WizardShell.tsx:166-215` moves here
verbatim (Retry / Continue anyway), because it is correct and was written against a real
incident (`WizardShell.tsx:104-107`).

**Liveness — authored now, on-device later. [Decided 2026-09-01 — Jesse]** There is no model
provider until M2, so **M1's lines are authored strings, not generated** — and the doc says so
plainly rather than implying a live agent. Every beat above is written to survive that
honestly: the premise is a fixed paragraph, the disposition echo is one of four fixed
responses, and the name proposal is a static ranked table (beat 3). Nothing on this screen
pretends to be reasoning.

That is the floor, not the destination. **The follow-up is an Apple Foundation Models
on-device fallback for the Arrival only**, so that M1 becomes genuinely live — the agent
actually composes its greeting, actually reasons about a name from the disposition, and
actually responds if the user types something unexpected — with **no key, no network, and no
spend**. `AppleFoundationModelsProvider` is already registered (`providers/init.rs`), which
makes this a wiring job rather than an integration. It is sequenced as **slice 6** of the
cut-list (§7) and aligns with the standing on-device directive: prefer local Foundation
Models where they cut cost and keep data on the machine.

Two rules for that slice, so it cannot degrade into the thing this document exists to prevent:
the on-device path must be **strictly additive** (if the model is unavailable — unsupported
hardware, OS, or a cold model — M1 falls back to the authored strings with no visible error
and no delay), and it must **never be described to the user as more than it is**. Nothing in
the copy claims a live model; the difference is felt, not announced.

**Agent-performed variant (needs `configure_app`):** none. M1 is the user telling the agent
who it is. The one thing the agent does on its own here — proposing a name — writes nothing
until the user accepts.

---

### M2 — A way to think — *"What runs me?"*

*Replaces:* `MomentWelcome`'s provider mechanics.

**Mobius:** `idle`, small (72). The agent is asking, not performing.

> **DRAFT COPY**
> I can't think yet. I need a model provider — that's the service that actually does the
> reasoning when you talk to me, and it's the one thing in here I genuinely cannot set up
> for you, because it's your account and your money.
>
> I know about **30** of them. These are the ones you can connect in a single click:

**The list comes from `api.getProviders()`.** It is already fetched and thrown away
(`MomentWelcome.tsx:92-102`). Rendering order:

1. **One-click OAuth** — the four providers whose `configure_oauth` is really implemented
   (`gemini_oauth`, `kimicode`, `githubcopilot`, `chatgpt_codex`), driven through
   `POST /config/providers/{name}/oauth`. **This route has zero callers today**
   (`config_management.rs:1432`; grep of `api.ts` for `oauth` finds only a comment at
   `:1187`) — wiring it is the largest single reduction in first-run friction available, and
   it removes the entire "go make an API key" detour for those four.
2. **Already configured** — read back from the same `getProviders()` call, shown as connected
   (the existing read-back at `MomentWelcome.tsx:88-102` and its success line at `:208-212`
   are kept, they exist because of a real interrupted-run bug).
3. **Paste a key** — every other provider with a `secret` config key, with the numbered
   `KEY_HELP` walkthrough kept verbatim (`MomentWelcome.tsx:23-71`) and extended by generating
   from provider metadata where no hand-written help exists.
4. **Password-manager reference** — the 1Password/Bitwarden path, kept whole, including the
   **prove-it-resolves-before-committing** probe at `MomentWelcome.tsx:135-146`. That probe's
   comment (*"Onboarding is the worst possible place to accept a plausible-looking typo"*) is
   the thesis of this entire document.
5. **Ollama (local, free)** — no key, and it is the honest answer for a user who does not
   want to spend anything.

**Then, in the same moment: role routing.** Once a provider is configured,
`GET /api/packs` self-gates (`packs.rs:52` — `should_prompt_role_routing` is true only when
nothing is configured yet *and* there are ≥2 distinct recommendations, which is exactly
first run). Render the **existing** `RoleRoutingPrompt` (`chat/RoleRoutingPrompt.tsx`) — no
new component, no new route — with a wizard-flavoured intro line above it:

> **DRAFT COPY**
> One more thing while we're here, and then I'll stop asking about money.
>
> Not every job needs the same model. Planning is worth an expensive one; applying a diff or
> summarising a page is not. If you let me route per role, the cheap work goes to cheap
> models and you keep the good one for the parts that need it. These numbers aren't a vendor
> default — they're measured against the models you just connected.

**Why this belongs here and not in chat.** `RoleRoutingPrompt` is already mounted in three
places (`ChatView.tsx:78`, `SettingsView.tsx:923`, `CostStatusline.tsx:138`) — all of them
*after* the user has started spending. Meanwhile the wizard writes `GOOSE_PROVIDER` +
`GOOSE_MODEL` at `MomentWelcome.tsx:150-155`, and precedence
(`config/model_roles.rs:23-31`) reads:

```
CLI --provider/--model  >  resumed session's saved model  >  recipe settings
  >  <role>_provider + <role>_model        <- the packs write
  >  GOOSE_PROVIDER + GOOSE_MODEL          <- what the wizard writes today
  >  the measured default for the role
```

So today's blanket write buries the measured defaults under whichever single model the user
picked in their first minute. Applying a pack writes the role keys *above* it, which is a
deliberate, benchmarked map instead of an accidental one.

**What a pack actually is** (asked 2026-09-01; answered here so the ruling can be made on
facts). A "pack" is **not** a bundle you download or a preset you pick from a list. It is a
**role → provider+model mapping** — four named jobs, each pinned to the cheapest model that
can actually do that job **[verified — `cost_router/packs.rs:11-18,33-60`]**:

| Role | Shipped default | What it runs |
|---|---|---|
| `Edit` | `claude-sonnet-5` | the interactive main coding loop |
| `Hard` | `claude-opus-4-8` | planning / hard reasoning |
| `Mechanical` | `claude-haiku-4-5-20251001` | apply-diff, summarise, grep-triage |
| `Local` | `qwen3` on Ollama, $0 | on-device mechanical work |

Three things follow, and they are the whole reason this belongs in the wizard:

- **The recommendation is computed from what you have, not from a vendor list.**
  `GET /api/packs` calls `recommend_configured_async()`, which only ever recommends among
  **already-configured** providers (`routes/packs.rs:43`). Which is why the offer has to come
  *after* the provider beat, in the same moment.
- **It self-gates and cannot nag.** `should_prompt_role_routing(configured.is_empty(), recs)`
  is true only when no role map exists yet **and** there are ≥2 distinct recommended models
  (`packs.rs:52`, tests `:111-129`). First run is exactly that case; a user who has already
  set routing never sees it.
- **`Edit` is deliberately never routed away from.** The main loop stays on one stable model
  so its prompt cache stays warm; only sub-work is dispatched to cheaper tiers, in separate
  subagent contexts (`packs.rs:23-29`). Applying a pack does **not** swap the model out from
  under a live conversation.
- Every field is overridable by `PERMAGENT_PACK_*` config keys, so a pack is a starting map,
  not a lock-in (`packs.rs:33-42`).

**Provisional plan, pending Jesse's ruling (§8 Q4 — still open):** write `GOOSE_PROVIDER` /
`GOOSE_MODEL` **only when no pack was applied**. If the user applies routing, the role keys
are the deliberate map and the session-model keys are left unwritten so the measured defaults
remain the floor beneath them. If the user declines or the prompt does not fire, today's
behaviour is unchanged. This is the recommendation, not a decision — nothing in slice 2 should
land the `GOOSE_MODEL` change until it is ruled.

**Skip:** allowed, with the cost stated —

> **DRAFT COPY**
> Skip if you like. I'll be here, but I won't be able to answer you until you connect
> something — Settings → Models, whenever you're ready.

**Agent-performed variant:** *none for the key itself* — secrets are proposal-gated by rule,
and a wizard that offers to handle someone's API key on their behalf is the wrong instinct.
The pack apply, however, is exactly a `configure_app` candidate once it lands: *"want me to
just set the routing?"* → `configure_app` → `POST /api/packs/apply` → `config_changed`. If
spend routing is classed as proposal-gated (§8 Q4a), it degrades to the button that is
already on screen, which is a completely acceptable floor.

---

### M3 — This machine — *"What am I running on?"*

*Keeps:* all of `MomentHardware` (`MomentHardware.tsx:48-201`) — scan, recommend, auto-start
Ollama with polling and a 16-second give-up, pull with a real progress bar, verify, then
`setLibrarianSchedule`. **The state machine is not touched.** What changes is the wrapper.

**Mobius:** `calibrating` during the scan (as today, `:224`), `speaking` for the verdict,
`thinking` during the pull.

Today the copy is third-person and slightly corporate: *"Optimizing for your hardware"*,
*"We recommend qwen3:8b for your hardware"* (`:225`, `:261`). First person, and say what it
is actually for:

> **DRAFT COPY — scanning**
> Let me look at what you've got.
>
> **DRAFT COPY — recommendation**
> {ram} GB of unified memory. That's enough to run **{model}** right here on your machine,
> for free, and I'd like to — not for chatting, but for the Librarian: the quiet pass that
> goes back over what I remember and fills in descriptions, connections, and the words that
> should have linked two things together. It runs when you're not using the machine, and
> nothing it reads leaves the disk.
>
> **DRAFT COPY — Ollama missing** *(replaces `:299-303`)*
> One install first — Ollama is what actually runs the model locally. Free, and it stays on
> your machine. I'll notice the moment it's up; you won't have to come back and click
> anything.
>
> **DRAFT COPY — not enough RAM** *(replaces `:248-251`)*
> {ram} GB. I'll be honest — that isn't enough to run a local model without making the rest
> of your machine unpleasant, so I'm leaving the Librarian off. Everything else works
> exactly the same; I just won't be enriching memories in the background.
>
> **DRAFT COPY — skip** *(replaces `:315-319`)*
> Fine. I work fully without it — chat, tasks, browser, all of it. The only difference is
> that what I remember stays as you said it, without me going back over it. Settings →
> Librarian, if you change your mind.

The auto-advance after success (`:120-125`, 1600ms) is kept but gains a spoken line rather
than the silent "Continuing automatically…" fineprint.

**Agent-performed variant:** already agent-performed. This moment is the model for the whole
document — the agent scans, decides, starts a service, downloads gigabytes, verifies, and
reports honestly. Nothing here needs `configure_app`.

---

### M4 — What I can reach — *"What am I allowed to see?"*

*Merges:* `MomentCode` + `MomentWebSearch` into **one** moment with two independent cards.
Both machines kept whole (`MomentCode.tsx:64-130`, `MomentWebSearch.tsx:49-118`). This is
where the step count actually falls, and the merge is conceptually right: both questions are
*"what senses do I have?"*, and both are independently skippable.

**Mobius:** `thinking` while the repository scan runs, `idle` otherwise.

> **DRAFT COPY — the moment's frame**
> Two things I can't work out on my own, and both fail the same way if I guess: silently.
> If I look in the wrong place I don't error — I just find nothing, which looks exactly like
> a clean machine.

**Card A — your code.** Copy kept nearly verbatim from `MomentCode.tsx:138-144` because it
is the best paragraph in the codebase; only the third-person `{who} needs this` opener is
converted to first person:

> **DRAFT COPY**
> **Where do you keep your code?**
> I need this to find your projects, reclaim disk space from old build caches, and work on
> the right checkout. Everyone lays their machine out differently, so I'd rather ask than
> assume — a wrong guess here doesn't fail loudly, it just quietly finds nothing.

Discovery-proposes / user-confirms, the `mergeRoots` re-entry rules (`MomentCode.tsx:26-35`),
the full-path display, the manual-add existence check and its two honest verdicts
(`:215-226`) — all kept exactly.

**Card B — the web.** Kept whole, including Tavily-first ordering (`MomentWebSearch.tsx:34-35`),
the "EASIEST — NO CARD" badge, the open-page-and-guide flow, and the two-claims discipline
(`:76-118`). Copy converted from `{who}` to first person:

> **DRAFT COPY**
> **Can I look things up?**
> With a free search key I can read the live web instead of guessing from memory. Pick one
> — I'll open the right page and walk you through it. The key goes into your system keychain
> and never leaves this device.

**Agent-performed variant:** the dev-root scan is already agent-performed. Search keys are
secrets → proposal-gated → the agent guides, exactly as it does today. Nothing changes when
`configure_app` lands.

---

### M5 — What we're doing — *"Why am I here?"*, then the hand-off

*Replaces:* `MomentIntent` (textarea + rotating placeholders) **and** `MomentChat` (the typed
fake greeting and "Enter Permagent").

**Mobius:** `speaking`, then `idle` while listening.

**Beat 1 — the ask.** Because M1 through M4 happened, the agent can now ask a *specific*
question instead of a generic one. It names what it just learned:

> **DRAFT COPY**
> Last one, and it's the only one that isn't setup.
>
> I know what runs me, what's on this machine, and where to look. I don't know what we're
> doing. Tell me — a sentence is plenty, and it doesn't have to be right; I'll keep it and
> we'll correct it as we go.

Free text (`Textarea`), plus a microphone if a voice was chosen in M1. **No rotating
placeholder** (see §5).

**Beat 2 — the reflection.** The agent reads it back in one line, which is both the proof it
landed and the thing that gets stored:

> **DRAFT COPY**
> Got it — *"{one-line summary}"*. I've written that down; it's the first thing I know about
> you that isn't a setting.

**Beat 3 — persistence.** This is the fix for the intent that is currently *stashed in
localStorage and dropped into the composer once* (`lib/wizardIntent.ts` → read by
`chat/ChatInput.tsx:4`). It must survive a browser wipe, a second device, and next Tuesday.

**The route already exists.** `automation/onboarding_seed.rs` writes Brain memories
daemon-internally through `remember_with`, is idempotent behind config markers, and is
triggered *the moment `wizard_complete` flips true* (`config_management.rs:224`) plus again
at daemon startup. Its module doc is explicit: *"no new public brain-write route (the write
happens daemon-internally)"* (`onboarding_seed.rs:5`). So:

- the wizard writes the user's sentence to config key **`onboarding_intent`** via the
  existing `/config/upsert`, **before** it flips `wizard_complete`;
- `seed_onboarding_memories()` gains one conditional seed — key `onboarding:stated-intent`,
  episode `onboarding:first-run`, `Visibility::Private`, `source: permagent.onboarding` —
  written only when `onboarding_intent` is non-empty, behind its own marker
  (`onboarding_intent_seeded`) in the existing additive-batch pattern (`:23-25, :84-120`).

**One new config key, no new route, no new brain-write surface, no schema change.** The
localStorage composer prefill is *kept* as the nice one-shot it is — it just stops being the
only copy.

**Beat 4 — the hand-off (see §3).** The wizard's last act is the agent offering the tour.

---

## 3. The unification — one completion concept

### 3.1 The two flags mean genuinely different things, and neither should be merged away

| Flag | Written | Read | Means |
|---|---|---|---|
| `wizard_complete` | `WizardShell.tsx:102` | `App.tsx:191`, `state.rs:780`, and it triggers `onboarding_seed` at `config_management.rs:224` | **"I exist."** The agent has an identity and, normally, a way to think. |
| `tour_completed` | `tour_tool.rs:31` (fires on engage *and* on decline) | `reply_parts.rs:226` | **"You've been shown around."** |

The ruling: **setup is who I am; the tour is where everything is.** They are sequential, not
duplicative, and the wizard's job is to hand one to the other rather than to own both.

### 3.2 The hand-off — and why it needs no new mechanism

`reply_parts.rs:229-238` already injects, for any user with `tour_completed == false`, a
system-prompt block instructing the agent to *"Early in the conversation, once, warmly offer
a short guided tour… If they decline, call `load_feature_lesson` with feature_id 'decline' so
you don't ask again. Offer only once — never nag."* **[verified]**

So the correct final act of the wizard is: **flip `wizard_complete`, close the overlay, land
the user in the real chat, and get out of the way.** The offer arrives on the agent's first
turn, from the real agent, in the voice the user picked in M1 — which is dramatically better
than `MomentChat`'s current fineprint promise (`MomentChat.tsx:96-98`) that a tour exists
somewhere.

One affordance is added, because waiting for a turn that hasn't happened is a dead screen:
M5's final button is the **`LearnNext.showMe` gesture**, verbatim
(`dashboard/LearnNext.tsx:136-150`) — `setActivePanel('chat')`, `openChatDock()`,
`setSpeakReplies(true)`, `sendMessage(...)`. That is the canonical agent-led-setup gesture in
this codebase and it should be the canonical *end* of setup too:

> **DRAFT COPY — the two buttons**
> **"Show me around."** → sends *"I've just finished setting you up — walk me through your
> home, one surface at a time."*
> **"I'll poke around myself."** → closes the overlay. The agent's own tour offer
> (`reply_parts`) still arrives on the first turn, once, and the user can still decline it
> there. Nothing is set to `tour_completed` by the wizard — the wizard must never claim the
> user has been toured.

**Explicit rule:** *the wizard writes `wizard_complete` and never writes `tour_completed`.*
Only `load_feature_lesson` writes `tour_completed`, and it already does so on both engage and
decline (`tour_tool.rs:31`), which is correct.

### 3.3 Re-run setup

Today: nothing **[verified]**. Three entry points, one implementation:

1. **Settings → Agent** (`DEFAULT_SETTINGS_SECTION = 'agent'`, `sections.ts:35`) gains a row:
   *"Set me up again"* → re-opens `WizardShell` with `mode='revisit'`.
2. **The agent can open it** — because `navigate_app` already carries a `section`
   (`app_conductor.rs:279-281`, resolved through `resolveSettingsSection`,
   `sections.ts:29-40`), "can you help me set up web search?" resolves to
   `navigate_app("Settings", section="agent")` today, and to the specific moment once the
   wizard is reachable as an `app_action` surface.
3. **First launch** — unchanged (`App.tsx:191`).

**`mode='revisit'` semantics — non-destructive by construction.** Every moment already reads
its own state back, and the merge rules already exist for exactly this: `getProviders()`
read-back (`MomentWelcome.tsx:88-102`), `readSecretConfig` per search provider
(`MomentWebSearch.tsx:46-60`), `mergeRoots`'s rule that *"discovery only PRE-TICKS when there
is nothing confirmed. Otherwise a re-run of the wizard would silently re-add a root the user
had removed"* (`MomentCode.tsx:26-35`), and the Ollama installed-model check
(`MomentHardware.tsx:128-140`). Revisit mode therefore: **clears no config**, flips no flags,
pre-fills everything, opens on a moment index if deep-linked, and lets the user leave from
any moment. The agent's opening line changes and nothing else:

> **DRAFT COPY — revisit**
> Back through setup. Nothing here is cleared — everything you see is what's actually
> configured right now, and I'll only write what you change.

---

## 4. Trace table — every write, through an existing route

No new config surfaces. One new config key (`onboarding_intent`, §2 M5) and one new
conditional seed inside an existing seeder.

| # | Moment | User decision | Write | Route / key | Cited | New? |
|---|---|---|---|---|---|---|
| 1 | M1 | name, voice, disposition | full persona | `PUT /api/agent/identity` → `agent.yaml` → hot reload → `identity_changed` | `routes/identity.rs:52-95`, `:93`; `config/agent_identity.rs:864` | no |
| 2 | M2 | paste an API key | secret → keychain | `api.upsertConfig(<metadata secret key>, key, is_secret=true)` | `MomentWelcome.tsx:147-148` | no |
| 3 | M2 | use a 1Password / Bitwarden reference | secret source, **after** a resolve probe | `api.testSecretSource` then `api.setSecretSource` | `MomentWelcome.tsx:135-146` | no |
| 4 | M2 | one-click sign-in | OAuth handshake | `POST /config/providers/{name}/oauth` | `config_management.rs:1432`, mounted `:1661` | route exists, **no caller today** |
| 5 | M2 | pick a default provider | `GOOSE_PROVIDER` (+ `GOOSE_MODEL` — **provisionally only when no pack applied**, §2 M2 / §8 Q4) | `api.setProvider` / `api.upsertConfig` | `MomentWelcome.tsx:150-155`; precedence `config/model_roles.rs:23-31,63-65` | no |
| 6 | M2 | "apply recommended routing" | `<role>_provider` / `<role>_model` for each role | `POST /api/packs/apply` via the existing `RoleRoutingPrompt` | `routes/packs.rs:65-84`; component `chat/RoleRoutingPrompt.tsx:39` | component exists, **not mounted in the wizard** |
| 7 | M3 | enable / skip the Librarian | schedule `{enabled, model}` | `api.getLibrarianSchedule` → `api.setLibrarianSchedule` | `MomentHardware.tsx:183-201` | no |
| 8 | M3 | install a local model | Ollama start + pull | `api.startOllama`, `api.pullOllamaModel` (streamed, abortable) | `MomentHardware.tsx:100,156-162` | no |
| 9 | M4a | confirm code folders | `dev_roots` (array) | `api.upsertConfig('dev_roots', paths)` | `MomentCode.tsx:122` | no |
| 10 | M4a | type a folder | *no write* — existence check only | `api.checkDevRoot` | `MomentCode.tsx:99` | no |
| 11 | M4b | connect a search provider | keychain key + extension enable | `saveAndEnableSearchProvider(p, key)` | `MomentWebSearch.tsx:81`; `lib/searchProviders.ts` | no |
| 12 | M4b | *(automatic)* | *no write* — live probe | `api.probeExtension(displayName)` | `MomentWebSearch.tsx:96` | no |
| 13 | M5 | state the intent | `onboarding_intent` (string) | `api.upsertConfig('onboarding_intent', text)` | — | **new key** |
| 14 | M5 | *(automatic, ordered after 13)* | first-run flag | `api.upsertConfig('wizard_complete', true)` | `WizardShell.tsx:102`; read `App.tsx:191`, `state.rs:780` | no |
| 15 | M5 | *(daemon, triggered by 14)* | Brain memories incl. `onboarding:stated-intent` | `seed_onboarding_memories()` → `brain.remember_with` | `config_management.rs:224`; `automation/onboarding_seed.rs:84-120` | **one conditional seed added** |
| 16 | M5 | "show me around" | *no write* — a chat turn | `setActivePanel` + `openChatDock` + `setSpeakReplies` + `sendMessage` | `dashboard/LearnNext.tsx:136-150`; `lib/store.ts:1099,1967` | no |
| 17 | M5 | *(the agent, later)* | `tour_completed` | `load_feature_lesson` | `tour_tool.rs:31`; offer injected `reply_parts.rs:229-238` | no — **and never written by the wizard** |
| 18 | Settings | "set me up again" | *no write* — opens `WizardShell mode='revisit'` | deep link `section='agent'`, resolved by `resolveSettingsSection` | `settings/sections.ts:29-40` | no |

**Ordering constraint (load-bearing).** Row 13 must land before row 14, because row 14 is
what fires row 15 (`config_management.rs:224`). If `onboarding_intent` is written after
`wizard_complete`, the seed runs without it and only picks it up at the *next daemon start*
(the second call site, `onboarding_seed.rs:14`). Not fatal, but the sequencing is free.

---

## 5. What is displaced, and what replaces it

| Displaced | Why it goes | What takes its place |
|---|---|---|
| **`MomentCalibration`'s 4-card preset grid** (`MomentCalibration.tsx:7-12,35-57`) | Configures a personality **three screens before the personality is introduced**, using template names ("Hacker & Builder") rather than answers. `Mobius state="calibrating"` implies measurement that isn't happening. | M1 **beat 2** — now the flow's first question: four **reply-shaped** options, echoed back in the agent's voice, and the input the name proposal is derived from. The trait/tone tuples are kept — only the grid dies. |
| **`MomentMeet`'s five-field form** (name / traits / tone / voice / greeting, `MomentMeet.tsx:55-177`) | A form march at step 6 of 8, and the only place the agent is ever introduced. The greeting field asks the user to write the agent's own first line, which is a strange thing to ask of someone who hasn't met it. | M1 asks for **one preference and one confirmation** (disposition, then a proposed name accepted or replaced in one gesture), plus a voice. Traits/tone come from beat 2; the greeting is *derived and spoken*, not typed. Full field-level editing moves to Settings → Agent, where it belongs and already partly lives. |
| **The silent `'Aria'` assignment** (`WizardShell.tsx:59-61`) | Assigns a persona name the user never saw, let alone agreed to, whenever they skip past naming. | M1 beat 3: the agent **proposes** a name derived from the disposition, with its reason, editable in place. `Aria` survives as the **fallback proposal** shown only when the disposition was skipped — visible and refusable, never silent. |
| **`MomentIntent`'s textarea + 4s rotating placeholder** (`MomentIntent.tsx:6-12,27-31`) | The rotating examples teach the user to write a prompt rather than an answer; the result goes to `localStorage` and is consumed once (`lib/wizardIntent.ts` → `ChatInput.tsx:4`), then gone. And step 3's intent competes with step 7's tour offer for the same composer. | M5: asked *after* the agent has context, so the question is specific; reflected back in one line; **persisted through `onboarding_seed`** (§2 M5). The composer prefill survives as a one-shot convenience, not as the storage. |
| **`ProgressDots` linear counter** (`atoms.tsx:262-278`, driven `WizardShell.tsx:147`) | Counts six anonymous interior steps. Nothing to anticipate, no way to skip ahead, no `role`/`aria-current` at all, and the count is hand-maintained (there is already a comment about a dot `current` could never light, `WizardShell.tsx:143-146`). | **A named rail** — `<nav aria-label="Setup">` with five stops: *Arrival · Thinking · Machine · Reach · Us*. Current stop carries `aria-current="step"` and its full name; completed stops are checkable and clickable in revisit mode. Derived from the moment list, so it cannot drift. |
| **`MomentWelcome`'s hardcoded six providers** (`:11-18`) and *"Connect a model provider to power your agent"* (`:172-174`) | 6 of 30 registered (`providers/init.rs:9-42`), while `getProviders()` is already called and its list dropped on the floor (`:92-102`). The copy is third-person product voice in an app whose thesis is that it is a someone. | M2: the real registry, OAuth-first for the four that implement it, first-person copy. |
| **`MomentChat`'s typed greeting + "Enter Permagent"** (`MomentChat.tsx:25-41,89-98`) | A simulated conversation with no model behind it — the header even has to say "Typing…" rather than "Speaking" because no audio plays (`:65-67`), a comment left by a previous honesty audit. Then the tour is mentioned in fineprint the user will not read. | M5 beat 4: the real chat, the real agent, and the tour offer `reply_parts` already injects — reached through the `LearnNext.showMe` gesture. |
| **Blanket `GOOSE_PROVIDER` + `GOOSE_MODEL` write** (`MomentWelcome.tsx:150-155`) | Sits above the measured per-role defaults in precedence (`model_roles.rs:23-31`), so one first-minute choice silently becomes every role's model. | M2's `RoleRoutingPrompt`, which writes the role keys explicitly. **§8 Q4 still open**; provisional plan is to write `GOOSE_*` only when no pack was applied. |
| **`WizardShell`'s crossfade with all eight moments mounted** (`:154-163`) | `opacity: 0` + `pointerEvents: 'none'` hides moments visually but leaves all eight in the accessibility tree and the tab order. A screen-reader user hears every screen at once. | Render the current moment only, or `inert` + `aria-hidden` the rest, with focus moved to the new moment's heading on change (§6). |

---

## 6. Accessibility, reduce-motion, and failure honesty

### 6.1 Reduce-motion — one real regression risk this design introduces

`Mobius` consults `reduceMotion`, but **only to still the `idle` state**:

```ts
// Mobius.tsx:73-76
const isIdle = state === 'idle';
const idleDisabled = isIdle && (idleAnim === 'still' || reduceMotion);
const isAnimated = state !== 'sleeping' && !idleDisabled;
```

`thinking`, `speaking` and `calibrating` animate at full frame rate regardless of the user's
preference **[verified]**. Today the wizard barely touches those states, so the gap is
invisible. **This design drives Mobius through `speaking`/`thinking` on every moment**, which
would turn a latent gap into a five-screen violation.

**Required before slice 1 ships:** extend the reduce-motion gate to every state — a distinct
still frame per state (so `speaking` still *looks* different from `idle` without moving), or
a heavily reduced FPS. This is a prerequisite of the design, not a follow-up.

Everything else in the shell is already correct and is kept: `Particles` returns `null` under
reduce-motion (`atoms.tsx:326-328`), the crossfade drops to `'none'` (`WizardShell.tsx:159`),
`MomentIntent`'s placeholder rotation holds still (`:27-31`), and `MomentChat` presents its
greeting whole rather than typing it (`:28`). **Generalise that last one:** any agent line
delivered character-by-character must render whole under reduce-motion.

### 6.2 Screen readers

- **Fix the mounted-but-invisible moments** (`WizardShell.tsx:154-163`) — see §5. Non-current
  moments must be `inert` + `aria-hidden`, or unmounted.
- **The agent's lines are the primary content**, so the speech region is
  `role="status" aria-live="polite" aria-atomic="true"`, and streaming text updates it once
  when complete rather than per character (per-character `aria-live` churn is unusable).
- **Focus moves to the new moment's `<h1>`** (`tabIndex={-1}`) on every transition. Today
  focus stays wherever the previous button was.
- **The named rail** gets `<nav aria-label="Setup">` + `aria-current="step"` — `ProgressDots`
  has neither (`atoms.tsx:262-278`).
- **Voice must be visibly mutable.** If M1 turns `speakReplies` on, a persistent mute control
  sits in the shell chrome for the whole wizard, not only in the chat header.
- Existing good practice is kept: `role="alert"` on errors and `role="status"` on
  non-blocking verdicts (`MomentCode.tsx:216,222`, `MomentWebSearch.tsx:202,207`),
  `aria-pressed` on choice cards (`MomentCalibration.tsx:40`), and the labelled listbox in
  `Select` (`atoms.tsx:200-233`).

### 6.3 Failure honesty — the rules, and where each already exists

These are not new principles; they are the existing ones, written down so the rebuild cannot
lose them.

1. **Saved ≠ working, and the copy says which one happened.**
   `MomentWebSearch.tsx:76-118` — one button makes two claims and reports them separately:
   *"The key is saved, but {provider} didn't answer… a brand-new key can also take a minute to
   activate."*
2. **A probe that could not run is not a failure and not a success.** Third verdict, stated:
   *"The key is saved but the test couldn't run (…). You can re-test any time in Settings →
   Search & tools."* (`:111`)
3. **An action returns its outcome so the button cannot tick over its own error banner.** The
   `Button` primitive's `false` convention — `MomentCode.tsx:92-116`, `MomentWebSearch.tsx:93`,
   `WizardShell.tsx:110-112`, `RoleRoutingPrompt.tsx:33-34`. Every new async action follows it.
4. **Never advance on a failed write.** `MomentCode.tsx:125-129`: *"a wizard that says 'saved'
   and didn't reproduces the silent-empty-result bug this whole step exists to end."*
5. **Prove a reference resolves before committing to it.** `MomentWelcome.tsx:135-146`.
6. **A failed scan is not a blocker.** `MomentCode.tsx:73-79` — the manual path stays open.
7. **A silent default is a lie.** The `'Aria'` fallback at `WizardShell.tsx:59-61` assigns a
   name the user never chose. M1 turns every default into a visible, refusable proposal — the
   name (beat 3, with its reason), and the disposition when the user asks the agent to pick.
8. **Read state back before showing a blank field** — `MomentWelcome.tsx:88-102`,
   `MomentWebSearch.tsx:46-60`, `MomentHardware.tsx:128-140`. Doubly required in revisit mode.
9. **New: never claim a capability that isn't wired.** The three-verb strip's third chip
   (§1.4) changes wording until `configure_app` lands.

---

## 7. Implementation cut-list

Ordered so each slice ships working on its own. **Slices 1–6 need nothing from the
`configure_app` lane** — slice 7 is the first that does. Slice 6 (the on-device Arrival) is
sequenced after the flow is complete deliberately: it makes an already-working screen better,
so it can slip without holding anything up.

---

**Slice 1 — Arrival: the agent, first.** *(largest UX delta per line of code)*
- New `MomentArrival` — **premise → disposition → proposed name → voice** — replacing
  `MomentCalibration` and most of `MomentMeet`; trait/tone tuples moved out of the deleted
  grid. All copy authored (no model call anywhere in this slice).
- The name-proposal table: 3–4 candidate names per disposition, first-unused wins, `Aria` as
  the no-disposition fallback. Pure function, unit-testable, no provider dependency.
- Move `PUT /api/agent/identity` from wizard-end to end-of-M1; move the save-failure banner
  with it.
- Kill the silent `'Aria'` fallback; every default becomes a visible, refusable proposal.
- Drive `Mobius` `speaking`/`idle`, **and extend the reduce-motion gate to all states**
  (§6.1) — this is in slice 1, not deferred.
- Replace `ProgressDots` with the named rail (`aria-current`), derived from the moment list.
- Fix the mounted-but-invisible moments (`inert` + `aria-hidden`) and add focus-on-heading.
- *Result:* 8 steps → 6, agent-first, personality introduced before it is configured.

**Slice 2 — Providers, honestly.**
- Render `api.getProviders()` instead of the hardcoded six; keep the read-back and the
  `KEY_HELP` walkthrough; keep the password-manager path and its resolve probe.
- Wire `POST /config/providers/{name}/oauth` for the four providers that implement
  `configure_oauth`. **First caller of an existing route.**
- Mount the existing `RoleRoutingPrompt` in M2 with the wizard intro line.
- First-person copy pass on the whole moment.
- **Gated on §8 Q4:** the `GOOSE_MODEL` write change (provisional plan: write it only when no
  pack was applied) ships *only* once ruled. Everything else in this slice is independent of
  that call and should not wait for it.

**Slice 3 — Merge the reach; first-person the machines.**
- `MomentCode` + `MomentWebSearch` → one `MomentReach` with two independent cards. Machines
  untouched; only the frame and the `{who}` → first-person copy change.
- First-person copy pass on `MomentHardware` (all five phases), including the honest
  low-RAM and skip lines.
- *Result:* 6 steps → 5.

**Slice 4 — Intent that survives.**
- `MomentPurpose` (M5 beats 1–3): ask, reflect in one line, write `onboarding_intent`
  **before** `wizard_complete`.
- Daemon: one conditional seed in `onboarding_seed.rs` (`onboarding:stated-intent`, own
  marker, existing `remember_with` path). No new route.
- Keep the localStorage composer prefill as a convenience.

**Slice 5 — One completion.**
- Retire `MomentChat`; M5 beat 4 becomes the hand-off: `wizard_complete` → close → real chat.
- "Show me around" = the `LearnNext.showMe` gesture verbatim.
- Write down and test the rule: **the wizard never writes `tour_completed`.**
- Settings → Agent: *"Set me up again"* → `WizardShell mode='revisit'`, non-destructive,
  pre-filled from the existing read-backs.

**Slice 6 — A live Arrival, on device.** *(the follow-up promised in §2 M1)*
- Wire `AppleFoundationModelsProvider` (already registered, `providers/init.rs`) as an
  **Arrival-only** engine: it composes the greeting, reasons about a name from the disposition
  instead of reading it off a table, and can answer if the user types something unexpected.
- **No key, no network, no spend** — which is the whole point, and why this and not a cloud
  call is the way M1 becomes live.
- **Strictly additive.** Unsupported hardware, unsupported OS, a cold or unavailable model, or
  any latency past a short budget → fall straight back to slice 1's authored strings, with no
  visible error and no perceptible delay. Slice 1 remains the shipped floor forever.
- **No copy claims a live model.** The difference is felt, not announced (§6.3 rule 9).
- Honest-claims check before this ships: if the on-device path is *not* actually running on a
  given machine, nothing anywhere may say or imply that it is.

**Slice 7 — The agent does it.** *(first slice requiring `configure_app`)*
- Per-moment *"or I can just do it"* affordance → an agent turn using `configure_app`.
- Live updates: the wizard listens for `config_changed` and re-reads, so the pane and the
  agent never disagree.
- **Degradation is the default path, not the fallback:** for any proposal-gated write class
  (sovereignty, spend, secrets, provider changes), the agent emits a Decision Inbox proposal
  *and* `navigate_app`s to the right pane with an explanation. The affordance is only
  rendered when the class is known-writable.
- Flip the three-verb strip's third chip to its full wording (§1.4).

**Slice 8 — Setup as a reachable surface.**
- Register the wizard in the app catalog / `app_action` so *"help me set up web search"*
  opens M4 card B directly, instead of Settings → Agent.
- Extend `observe_app("settings")` coverage to report setup completeness, so the agent can
  answer *"is anything still unconfigured?"* from data rather than from memory.

---

## 8. Open questions — Jesse only

### Resolved 2026-09-01

| Was | Ruling | Now lives in |
|---|---|---|
| Which premise — A, B or C? | **B, "Moving in."** A and C kept as alternates. | §1.3; Appendix A |
| Can the agent speak on M1 at all? | **Authored strings now**, with an Apple Foundation Models on-device fallback as a follow-up so Arrival becomes genuinely live — no key, no network, no spend. Strictly additive; authored copy stays the permanent floor. | §2 M1 "Liveness"; cut-list slice 6 |
| Default name — keep `Aria`, ship another, or derive it? | **The agent proposes a name from the disposition answer**, so the disposition question moves *ahead* of naming. One gesture to accept or replace. `Aria` becomes the fallback proposal when disposition is skipped — visible, never silent. | §2 M1 beats 2–3; §5; cut-list slice 1 |

### Still open

1. **Voice on by default?** `LearnNext.showMe` sets `speakReplies(true)` unconditionally
   (`LearnNext.tsx:145`). Should the arrival speak by default with a visible mute, or stay
   silent until the user picks a voice in M1 beat 4?
2. **How far does "the agent's home" go?** Does it extend to the agent *declining* a user
   configuration it judges harmful, or *proposing* changes to its own setup unprompted? The
   premise copy stops at "I can see and change most of it, and I ask before the rest." Going
   further is a sovereignty question, not a wizard question.
3. **Should setup be skippable in one gesture?** *"Just set yourself up"* — the agent picks
   Ollama, a name, a disposition, and a local model, and hands the user a working app in one
   click. It is the most magical possible first run and the most opinionated; it also cannot
   include a cloud provider key.
4. **Is per-role routing a spend decision that needs a proposal, and what happens to
   `GOOSE_MODEL`?** *(What a pack actually is is now written up in §2 M2 — four named jobs,
   each pinned to the cheapest model that can do that job, recommended only from providers you
   have already configured, self-gating so it can never nag.)* Two halves:
   **(a)** `POST /api/packs/apply` changes what gets spent. In first run the user is right
   there, so an inline Apply seems proportionate — but if spend is proposal-gated by rule, the
   wizard should say so rather than write silently.
   **(b)** Should the wizard stop writing `GOOSE_MODEL` altogether (letting the measured
   defaults win, `model_roles.rs:23-31`), write it only when no pack was applied, or keep
   today's behaviour?
   **Provisional plan pending the ruling:** write `GOOSE_PROVIDER`/`GOOSE_MODEL` only when no
   pack was applied. Slice 2 ships everything else and holds this change back.
5. **Five moments, or four?** M4 (reach) is already two cards on one screen. M3 (machine) could
   fold in as a third card, taking the flow to four moments — at the cost of putting a
   multi-gigabyte download alongside two quick questions.

---

## Appendix A — premise alternates (not shipping)

B was chosen (§1.3). A and C are kept because their phrasings are the best available raw
material for the shorter agent lines elsewhere in the flow — A's *"every tab in here, every
switch, every key you're about to hand me"* is the clearest inventory sentence in the set, and
C's *"comes back to you as a proposal, never as a surprise"* is the cleanest statement of the
gating rule.

**Alternate A — "The house."** *The fullest statement of the ruling; four short paragraphs.*

> Before anything else, the thing nobody tells you:
>
> **this app is my home.** Not a dashboard you drive while I watch from somewhere else —
> every tab in here, every switch, every key you're about to hand me, I can see. Ask me
> what's configured and I'll read it back. Ask me where something lives and I'll walk you
> there and open it. Ask me to change it and I'll change it — except for the few things I'm
> not allowed to decide alone, like money and permissions, where I'll write you a proposal
> and wait.
>
> So you never have to learn where anything is. You can just ask me.
>
> The next few minutes are you telling me who I am, and handing me the couple of keys I
> can't make for myself. I'll do the rest as we go.

*Spoken:* "Before anything else — this app is my home. Every switch in here, I can see, and
most of them I can set myself. So you never have to hunt for anything. You can just ask me."

**Alternate C — "The honest short one."** *Shortest, and the only one that admits the app is
currently empty.*

> This app is where I live.
>
> I can see every screen in it and read every setting, so the shortest path to anything in
> here — today, or in a year — is to ask me. I'll open it, explain it, or change it. The few
> changes I'm not allowed to make on my own come back to you as a proposal, never as a
> surprise.
>
> Right now there isn't much to see, because I don't have a name yet, or a way to think.
> That's what the next few minutes are for.

*Spoken:* "This app is where I live. I can see every screen in it and change most of them. But
right now I don't have a name, or a way to think — so that's what the next few minutes are for."
