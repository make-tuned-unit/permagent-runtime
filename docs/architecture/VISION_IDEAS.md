# Vision Ideas — Open Ideation Pass (2026-07-01)

**Status:** Ideation only. Zero code, zero PRs. Every item is a proposal for Jesse to triage.
**Grounding:** Open issue tracker (80 issues), the decided architecture docs (UNIFIED_WORKSPACE,
WEBVIEW_LIFECYCLE, SWARM_VISIONS, DURABILITY_AUDIT, CONTROL_LOOP_SPEC,
GOAL_COMPLETION_AND_VERIFICATION, SELF_KNOWLEDGE_BRAIN, LEVERAGE_MAP), plus a code-level sweep of
the daemon (`crates/`) and frontend (`ui/command-center/`). Claims marked **[verified]** were
checked against source this pass; nothing here reverses a ruled decision — everything proposes
*forward* from them.

**Known in-flight, deliberately NOT re-reported:** idle-wedge (#562), associate-click (#561),
webview cluster (#517/548/550/551/553/555), CRM bridge (#554), Steward (#318), verifier family
(#454–458), World View issues already filed (#383 nameplate, #386 zone nav, #391 manual control),
voice issues already filed (#398 barge-in, #452 multi-part, #244 pronunciation, #267 over-narration).

---

## The Top 10 (ranked by impact-per-effort, cross-section)

| # | Idea | Section | Effort | Type |
|---|------|---------|--------|------|
| 1 | **Wire the Initiative layer** — it's fully built and connected to nothing | H1 | S–M | Cheap win |
| 2 | **The rotunda shows the work** — goal-driven workpieces, honest ambient Kanban | D1 | M | Big-bet feel, medium cost |
| 3 | **Daemon-health pill** — surface half-dead honestly (durability F1 made visible) | U1 | S | Cheap win |
| 4 | **The Day Log** — Henry's nightly journal: narrative of the day, recallable + spoken on arrival | N1 | M | Cheap-high-impact |
| 5 | **Ask-the-agents** — click a character, talk to it in character about its real domain | N2 | M | Big delight × capability |
| 6 | **The orchestrator enable session** — the master key five subsystems wait behind | H2 | S (session, not code) | Cheap win |
| 7 | **Memory motes** — memory_added becomes visible light in the world | D2 | S | Cheap win |
| 8 | **Sound layer v0** — three sounds, zero exist today | D4 | S | Cheap win |
| 9 | **Goal cards show their budgets** — elapsed/wallclock, attempt N of cap | U5 | S–M | Cheap win |
| 10 | **Self-knowledge Phase 2** — `query_permagent_knowledge`, designed and unbuilt | H3 | M | Compounds onboarding |

---

## 1. MAGIC / DELIGHT

The delight substrate is genuinely strong and under-used: agents already have a six-pose
body-language state machine driven by **real** runtime state (`AgentCharacterV2.tsx`,
`behavior.ts`), the hall's column veins already brighten with working-agent count
(`ambience.ts`), Henry already publishes his position every frame and light gathers where he
stands. The presence-honesty law (SWARM_VISIONS V1-E) is the design constraint AND the
opportunity: everything below is driven by real events, never faked.

### D1. The rotunda shows the work itself — goal workpieces `M · high impact`
**What:** When a goal dispatches, the assigned agent takes a desk (seat-claims already exist in
`behavior.ts`) and a glowing workpiece materializes at that desk — a slab/scroll keyed to the
goal. When the goal enters Review, the workpiece drifts to a plinth near Henry's antechamber
path. On Approve it ascends into the dome / is absorbed into a column vein; on Reject it returns
to the desk. All driven by `goal_state_changed` events already on the broadcast bus.
**Why:** This is the single biggest "living OS" move available. Today the world shows *that*
agents work but not *what* — the Kanban and the world are disconnected truths. This makes the
World View a truthful ambient Kanban you can feel from across the room. It also gives Review a
physical presence: unreviewed work visibly accumulates on the plinth, which is honest pressure.
**Builds on:** goal state machine [verified in `goal_state.rs`], event bus, existing prop
pattern (Librarian's mining tablet in `motion.ts`).

### D2. Memory motes — memory formation made visible `S`
**What:** On `memory_added`, a small mote of light rises from the responsible agent and is
absorbed into a column vein (the veins are already the "activity" metaphor). Librarian's
describe-pass emits the same mote from the mezzanine.
**Why:** The Brain is Permagent's soul and it is completely invisible in the world. One particle
+ one absorb animation makes "it remembered that" a felt moment. Reuses the vein-opacity system
and the tablet-event pattern — no new architecture.

### D3. The arrival moment — "while you were away" `S–M`
**What:** When the app opens after >N hours idle, the HeroCard (and optionally one spoken Henry
line) carries a 1–2 sentence digest built from real data: goals completed/parked, memories
formed, steward findings, decisions waiting. Data already exists (`/api/henry/status` daily
totals, events buffer, cards).
**Why:** Currently the app opens cold — "Henry is ready." An agent OS that worked overnight
should *greet you with what it did*. This is the cheapest possible version of N1 (Day Log) and
can ship independently.
**Guardrail:** respect #267 (over-narration) — one line, never a paragraph, voice only if voice
was already active.

### D4. Sound layer v0 — three sounds `S`
**What:** Permagent has **zero** audio outside TTS [verified — no ambient/SFX in the UI tree].
Add exactly three: (1) a soft chime when a goal lands in Review, (2) a distinct quiet ping on
`decision_created`, (3) an optional low room-tone in World View only. Master mute in settings,
default: SFX on, room-tone off.
**Why:** Sound is the cheapest "alive" signal there is, and the two chosen events are precisely
the ones that want ambient attention (both mean "Jesse's input is now the bottleneck"). Three
sounds is the taste ceiling — more becomes noise.

### D5. Stargate honesty `S`
**What:** The Mesh Stargate's event horizon currently animates at full ripple permanently
(`Stargate.tsx` — uTime-driven, unconditional). Tie its intensity to real state: dormant shimmer
when no mesh peers, ripple bursts on actual network/mesh events; full spiral only when a peer is
live.
**Why:** It's the one object in the world that violates the presence-honesty law today. Making
it honest also makes it a status display for the Mesh epic (#306) for free — when M2 lands, the
gate wakes up, and that will *mean something*.

### D6. Henry's thinking pool `S · micro`
**What:** Henry's floor pool (presence light, `AgentCharacterV2.tsx:250`) currently gathers when
he stops walking. Add a distinct register: while his hudState is `tool_call`/`in_conversation`,
the pool ripples concentrically (slow, subtle). His live state is already published per frame
(`henryPresence.ts`) — this is a shader/opacity tweak.
**Why:** "Henry is thinking about your question right now" becomes visible in the world, not
just in the chat spinner. Pairs with D1: the world answers "what is everyone doing" at a glance.

### D7. The Steward's rounds `S–M`
**What:** When the steward recipe fires (it's a scheduled worker with a real cadence), a
character does a visible "tending" circuit of the rotunda — the tending pose and haul-stance
animation **already exist** in the pose machine [verified, `AgentCharacterV2.tsx`]. Findings-in-hand:
if the sweep produced a report card, the circuit ends at Henry.
**Why:** SWARM_VISIONS already ruled workers get real presence (Enricher-as-Librarian-task,
V1-0). The Steward is the first autonomous worker with a heartbeat — giving its runs a body makes
"autonomous background work" legible and charming instead of invisible. Direct forward-build from
the ratified swarm-visions direction.

---

## 2. HALF-DONE — finish or cut

Surfaced by code sweep. Each item: state, evidence, and the finish-vs-cut framing.

### H1. ⭐ Initiative layer: fully authored, wired to NOTHING `finish: S–M`
**State:** `crates/goose/src/initiative/` contains the complete organ — `command_counter.rs`
(pattern detection with threshold+window), `gate.rs`, `draft.rs` (template + model draft with
prune-on-bounce), `tick.rs`, `emit.rs` (surfaces a goal card in Triage, with the staged
one-line card→decision swap comment) — **all with passing unit tests**. But **[verified this
pass]**: zero references from `goose-server`, the scheduler, or the events layer. Nothing calls
`tick`. The differentiator organ ("ambient goal origination — cloud agents can't watch
everything," Phase 0 design-locked with 4 rulings, epic #360) is built and not connected to the
bloodstream.
**Finish:** wire the tick into the activity stream per the locked design (reuse Steward
seam + scheduler tick — the design doc already ruled this). Likely a small, well-scoped PR.
**Cut:** hard to justify — the expensive part (design + rulings + module + tests) is already paid.
**Recommendation: FINISH. This is the highest leverage-per-line item in the repo.**

### H2. The orchestrator enable session — the master key `finish: one supervised session`
**State:** Five built subsystems idle behind `orchestrator.enabled=false` [verified via config +
`dashboard.rs:213` gating]: the Decision Inbox surface, recognition-event volume (~0 until
enabled), the staged Steward card→decision swap (`steward/mod.rs:190` — "the only line that
changes"), the staged Initiative swap (`initiative/emit.rs:16`), and Henry-policy Tier-1
auto-answers. #390 already frames this as a real enable-then-dogfood in a manual session.
**Why it's listed here:** it's not code — it's the *milestone* that converts five dormant
investments into live product at once. Nothing else on this list compounds harder.
**Recommendation: schedule the supervised enable session; treat it as the gate for D-ideas
that surface decisions.**

### H3. Self-knowledge Phase 2: `query_permagent_knowledge` `finish: M`
**State:** Tier 1 (`<permagent_self>` brief) shipped and healthy — 21 tool descriptors, 3
workers, 7 surfaces, a guard [verified in `self_knowledge/mod.rs`]. Tier 2 — the on-demand
lookup tool backed by a tagged markdown corpus (SELF_KNOWLEDGE_BRAIN §5, est. 3–4 days) — was
designed and never built.
**Why finish:** Henry can *name* his capabilities but can't answer "how do I…" depth questions
about his own product. Phase 2 + the existing tour infrastructure (H5) together make Henry his
own onboarding. **Recommendation: finish after H2, before any new user-facing surface.**

### H4. HenryHUD CHAT and TOOLS tabs: disabled "SOON" `finish: S / cut: S`
**State:** Both tabs render permanently disabled with a "SOON" label
(`HenryHUD.tsx:44-45`). They've been "soon" long enough to be furniture.
**Finish:** CHAT is nearly free — the chat system exists; wire the tab to open the chat widget
(or better: the N2 in-character session). TOOLS could render the already-built self-knowledge
tool inventory read-only.
**Cut:** remove the tabs; dead chrome in the flagship delight surface is worse than absence.
**Recommendation: finish CHAT as part of N2; cut TOOLS until there's a real design.**

### H5. Tour / teaching: infrastructure live, 4 lessons, never dogfooded `decide`
**State:** `load_feature_lesson` tool, `tour_completed` flag, first-run offer in
`reply_parts.rs:221`, lessons for reader/brain/scheduler/persona [verified]. The v1 ruling
(4 lessons, World View deferred to v2) shipped; behavioral dogfood never happened.
**Recommendation: one dogfood session decides this. If the lessons land, add the World View v2
lesson *after* D1 gives it something to teach. If they feel thin, cut the first-run offer and
keep the on-demand tool.**

### H6. entity_fields write path — next decided slice, no new decision needed
**State:** `set_entity_field()` exists, is tested, and is never called in production
(UNIFIED_WORKSPACE §1.2 — "one route away"). This is exactly slice 2b, already sequenced by
Decision D (2a → B → 2b → 4). Listed only for completeness: **it is the next domino, and it
gates the Enricher (slice 4) which gates SWARM_VISIONS V1.** No triage needed — just sequencing
priority.

### H7. Librarian Behavior C (unload-when-done) `finish: S–M`
**State:** Scheduler warms Ollama for the full idle window (Behavior A); Behavior C
(warm-at-start, unload-when-queue-empty) is blocked on the Librarian exposing a queue-empty
signal (`librarian/scheduling.rs:115`). ~60% done.
**Why it matters more than it looks:** V1-0 just made the Librarian the Enricher too — its idle
window is about to get more load and the M1's 16GB makes model residency a real cost.
**Recommendation: finish as part of the enrichment slice, not before.**

### H8. Recognition outcome loop: half-instrumented `rides H2`
**State:** Recall-side persistence + outcome write-back built; ambient probe-recall path
deliberately not instrumented (drops ids); volume ~0 until orchestrator enables.
**Recommendation: no action; re-evaluate after H2 produces real volume. Then the interesting
finish is a consumer — see N1, which is the natural first reader of recognition data.**

### H9. Brain wiring in scheduled + subagent runs — needs a fresh audit `audit: S`
**State:** LEVERAGE_MAP's #1 finding (May): recall/remember fire only in HTTP chat handlers, not
in the scheduler's `execute_job()` or Summon subagent runs — "scheduled Henry has amnesia." The
map is old and much has landed since; **this pass did not re-verify it**.
**Recommendation: one-hour audit. If still true, it's ~100 lines for a categorical capability
jump (scheduled jobs that remember and learn), and it's a soft prerequisite for N1's nightly
job being any good.**

### Correction to the record
The daemon sweep initially flagged Reader document extraction as stubbed. **False** — verified:
`reader::ingest_document` is implemented (`reader/mod.rs:263`) and routed. The real, already-filed
gaps are #468 (font-encoded PDF garbling) and the scanned-PDF OCR follow-up. Nothing new to file.

---

## 3. UI TWEAKS — specific and concrete

### U1. Daemon-health pill in the app chrome `S · do this one`
**What:** A small persistent pill (chrome corner, near the tab bar): green = daemon heartbeat
fresh, amber = degraded (stale heartbeat / a subsystem's last-tick is old), red = unreachable.
Fed by a lightweight heartbeat endpoint; later upgraded by the DURABILITY_AUDIT Part B probe.
**Why:** The durability audit's F1 (half-dead daemon) has *no user-visible symptom today* —
failed fetches silently retry everywhere (e.g. `HenryHUD.tsx:100` `catch { /* silently retry */ }`).
Jesse discovers wedges by noticing absence. This is the honest-presence law applied to the daemon
itself, and it makes every future durability fix observable. Complements #562/#564, doesn't
overlap them.

### U2. Goal cards show their budgets `S–M`
**What:** In-flight goal cards (InFlightCard + Kanban) show elapsed-vs-wallclock and "attempt 2
of 3" when >1. Data exists — the S4 budget machinery (attempt cap, token, wallclock) is on the
goal [verified in goal engine].
**Why:** Parked/exhausted goals currently look identical to healthy ones until they emit an
unblock decision. Budget visibility turns "why is this taking forever" from a chat question into
a glance. Also makes #467 (wallclock too short) self-evident when it bites.

### U3. Decisions card pulses on arrival `S`
**What:** On `decision_created` (already on the event bus), the DecisionsCard does one soft glow
pulse and increments live, instead of waiting for the next poll.
**Why:** Decisions are THE human-in-the-loop bottleneck; their arrival is the single event most
worth a felt signal. Pairs with sound D4(2).

### U4. Ticking time-ago `S`
**What:** `RecentCard`/`DecisionsCard` compute "Xm ago" at render and never refresh — a 30s
interval re-render hook fixes staleness perception.
**Why:** A dashboard whose timestamps freeze reads as dead even when the system is alive —
cheap anti-staleness.

### U5. First-paint skeletons on the dashboard `S`
**What:** Cards render a subtle skeleton until their first fetch resolves (today: empty states
flash before data arrives).
**Why:** The empty-state copy is good ("No active goals…") but showing it falsely for 300ms on
every boot teaches the user to distrust it.

### U6. World-HUD token sweep → fold into #271 `S · mechanical`
**What:** ~50 hardcoded grays/rgba in the world HUD family (`#9CA3AF` ×15, `#4B5563` ×8,
assorted `rgba(...)` pills in HenryHUD/HudShell/ReaderHUD), plus a scrim token
(`rgba(0,0,0,0.5)` hardcoded in overlays) and the hardcoded "◠" spinner glyph
(`HenryHUD.tsx:160`) → Mobius or standard loader.
**Why:** #271 exists for exactly this; the world HUDs postdate it and regressed the discipline.
One mechanical pass; no design decisions.

### U7. Empty In-Flight state gets one Mobius `S · micro`
**What:** The In-Flight empty state is text-only; float the idle Mobius (13fps idle animation
already exists) above the copy.
**Why:** The card that says "Henry is ready for work" should carry his mark. Smallest possible
delight-per-line.

---

## 4. NEW HEIGHTS

### N1. The Day Log — Henry's journal `M · cheap-high-impact` ⭐
**What:** A nightly job (inside the existing Librarian idle window — the model is already warm)
composes a short narrative of the day from real substrate: events buffer, goal transitions,
recognition events, steward findings, memories formed. Written to the Brain as a recallable
memory (`journal:YYYY-MM-DD`), surfaced as one board card, and — via D3 — the source of the
arrival greeting and a spoken one-liner. The steward report→card+Brain-memory seam (#318, just
merged) is the exact pattern to reuse.
**Why:** This is the local-first thesis made tangible: an agent that *lived through your day
with you* and can tell you about it — and about last Tuesday, because journals are recallable.
It compounds four existing investments (events, recognition, librarian window, steward
card-seam) and creates the first real *consumer* of recognition data (H8). Cloud agents
structurally cannot do this.
**Effort:** M. Almost pure composition — one scheduled recipe + one prompt + reuse of #318's seam.

### N2. Ask-the-agents — in-character sessions grounded in real domains `M · big delight`
**What:** Click a World View character → the HUD CHAT tab (H4) opens a real chat session with a
persona overlay + domain context preloaded via the **already-shipped discuss-with-persona seam**
(#303: `extend_system_prompt('app_context')`, no new endpoint). Librarian answers about what's
in the Brain (it literally wrote the descriptions); Steward answers from its latest repo-health
report; Reader answers about what it ingested. Same Henry underneath — an honest lens, not a
fake mind.
**Why:** This converts the World View from a diorama into a *place where the characters are
real* — the single most "living OS" capability available at medium cost. Every persona's
grounding data already exists; the seam already exists; the tab already exists (disabled).
**Guardrail:** presence honesty — each persona only speaks to its actual domain and says so.

### N3. Initiative activation — "Henry noticed…" `S–M wire + polish · the differentiator`
**What:** H1's wiring plus the product moment: when the command-counter crosses threshold, the
drafted proposal lands as a card ("You've run this deploy sequence 5 times this week — want me
to make it a scheduled recipe?"), with prune-on-bounce already built so declined ideas stay
declined.
**Why:** Ambient goal *origination* from unscoped observation is the ruled differentiator (#360:
"cloud agents can't watch everything"). Everything below the surface exists and is tested; what
remains is the connection and the copy. First proposal Henry originates himself will be the most
important moment in the product's life so far.

### N4. The self-health organ — durability made felt `M`
**What:** Build DURABILITY_AUDIT Part A/B instrumentation (5-min probe: task liveness, WAL size,
scheduler tick freshness, disk) and expose it three ways: the U1 pill, a `<permagent_self>`
integration-state line ("up 26 days, all organs nominal"), and amber cards when a threshold
trips ("my Brain WAL grew 10× — a checkpoint is wedged").
**Why:** The audit's bar is *weeks untouched*; you can't trust what you can't observe. Making
Henry aware of his own body is both the instrumentation the audit says must precede the fixes
AND a character moment — an agent that says "I'm not feeling well" before it wedges is trust
you can't buy. Forward-build from the audit's own deferred Part A/B; no fix-ordering decisions
pre-empted.

### N5. The project-scoped-recall spike — one stone, two epics `L · big-bet, sequencing insight`
**What:** A single spike delivering `project_id` on Brain memories + project-filtered recall
(likely Spectral-side + the association-table read path per UNIFIED_WORKSPACE §5's
scope-is-association-not-graph ruling).
**Why:** This one capability is the *shared* hard-gate on two epics: CONTROL_LOOP S6
(project-state memory consumer — spec calls it "THE GATING BLOCKER") and the Project Workspace's
deferred Memories/Docs lens (#471 audit: "brain recall global" was why it deferred). The docs
each treat it as their own blocker; nobody owns it. Naming it as one spike changes the roadmap
math: L effort, but it un-gates S6 + #471-Memories + strengthens N1's per-project journals.
**Not proposing an architecture** — proposing that it get an owner and a slot.

### N6. Ambient mode — the world as idle display `M–L · big-bet, hold`
**What:** After N minutes idle, the app drifts into the World View — the day's activity visible
as D1 workpieces and D2 motes, camera on a slow orbit. Any input returns to the last view.
**Why listed despite the hold:** it's the natural convergence of D1+D2+D5 and the strongest
"this is an OS, not an app" statement. **Why hold:** the webview-lifecycle cluster (#548/551/553)
must be fully settled first — an ambient mode built on today's occlusion-throttling behavior
would inherit every render-wedge bug. Sequence strictly after WEBVIEW_LIFECYCLE S-track proves out.

### N7. Business-Development Initiative — proactive growth work, bounded `M–L · big-bet, gated`
**What:** A future *consumer* of the Initiative layer (gated behind the #360 wiring landing
first): Henry **proposes and drafts** growth work — never autonomously acts externally.
- Henry **notices** signals through the Initiative gate (a project needs awareness, a launch is
  near, the waitlist grew).
- Henry **drafts** the artifacts — launch posts, outreach emails, campaign plans, waitlist
  updates — and **surfaces** them for approval via the same card→decision seam every other
  proposer uses.
- The **human publishes/sends.** Every external action (post, email, ad spend, contacting a
  person) stays gated behind explicit user approval — the same detector/proposer boundary the
  Steward is ruled to (SWARM_VISIONS V2-A, inviolable) and the same egress-control principle as
  the Librarian's enrich task (V1-0: egress is a control, not a label).

**Explicitly NOT:** an autonomous agent that publishes, contacts people, or spends money on its
own. That external-action-without-approval shape is ruled out here — the failure modes
(reputational, financial, correctness) are severe *and silent*, which violates both the
observable-by-design and human-in-the-loop principles this system is built on.

**Why:** The leverage is in the *noticing and the drafting* — that's the hard 80% of growth work,
and it's exactly what the Initiative gate + cheap-draft pipeline is shaped for. Hitting send is
trivial and stays human. This is controllable power, not autonomous risk: the marketing surface
UNIFIED_WORKSPACE already reserves (slice 5's third consumer) gets its proactive engine without
ever loosening an approval gate.

**Effort:** M–L, big-bet. **Gated behind:** Initiative wired (#360) first; then any
external-action *tools* (posting, sending) would need their own bounded egress layer with
per-action approval — a separate epic if ever pursued, not part of this consumer.

---

## Suggested sequencing (if the ranking holds)

1. **Now, cheap:** U1 health pill · D6 thinking pool · U3 decision pulse · U4/U5/U7 · D4 sounds
2. **Next, the key:** H2 orchestrator enable session → unlocks H8 volume, staged swaps
3. **The differentiator arc:** H1/N3 initiative wiring → N1 Day Log (with H9 audit first) → D3 arrival moment
4. **The delight arc:** D1 workpieces → D2 motes → D5 stargate → D7 steward rounds → N2 ask-the-agents (+H4 CHAT tab)
5. **The trust arc:** N4 self-health organ (instrumentation before durability fixes, per audit)
6. **The big bet needing an owner:** N5 project-scoped recall spike
7. **Hold until webview cluster settles:** N6 ambient mode
8. **Gated behind the initiative arc:** N7 business-development consumer (proposer/drafter only; external actions stay human)

*Authored by a read-only vision pass, 2026-07-01. No code was changed; no worktrees were touched.*
