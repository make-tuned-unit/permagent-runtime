# Self-Knowledge Brain — Design Doc

**Status:** Draft v0.1
**Author:** Jesse Sharratt (via Claude collaboration)
**Last updated:** 2026-05-07
**Target milestone:** Permagent Phase 4+
**Builds on:** Phase 3 ambient awareness arc, Hub System v0.2

---

## 1. Problem

Most LLM-backed products fail to ground their agent in the product the user is currently using. The agent inside Permagent today knows what foundation models know — language, reasoning, general world facts — but it doesn't reliably know:

- What tools are wired into this specific Permagent install
- What integrations are connected vs. available but not connected
- What automations the user has set up and their current state
- What features exist in the UI and where to find them
- What this version of Permagent can do that yesterday's version couldn't
- What it specifically cannot do (and shouldn't pretend to do)

The result is a predictable failure mode: the agent confidently describes features that don't exist, fails to mention features that do exist, hallucinates about its own architecture, and gives generic LLM answers when a Permagent-specific answer would be far more useful.

This is one of the most underappreciated weaknesses in current LLM products. It's not a model capability problem — it's a context engineering problem. The product knows things about itself that the agent doesn't have access to.

The Self-Knowledge Brain is Permagent's solution: a layered system that grounds the agent in its own product reality before responding to the user.

## 2. Why this matters strategically

Three reasons this is worth building beyond the technical merits:

**Onboarding becomes conversational.** New users don't need to read documentation if they can ask the agent. "How do I set up a daily briefing?" gets a real answer with concrete steps, not generic productivity advice. The agent becomes a documentation surface.

**The agent appears competent.** Hallucinations about your own product are the single most credibility-damaging failure mode for an LLM product. An agent that says "you have Slack connected" when Slack isn't connected destroys trust faster than almost any other failure. Self-knowledge eliminates this class of error.

**Permagent's unique architecture becomes legible.** Permagent ships with substantial capabilities (ambient awareness, scheduler, memory, MCP integrations, recipes, managed folders). Without self-knowledge, users discover these through the UI piecemeal. With self-knowledge, the agent can articulate what it can do in real time as users explore — turning the product into a self-documenting system.

This last point is the strategic differentiator. Most products treat the agent as a feature inside the product. Self-knowledge inverts that — the product becomes legible *through* the agent, with the agent as the navigation layer over everything Permagent provides.

## 3. What goes in the self-knowledge brain

Eight categories of self-knowledge, ordered from most-needed-every-turn to occasionally-needed:

**Tier 1 — Always loaded (small, high-value, current state):**

1. **Capability inventory.** What tools are loaded right now. Shell, file system, web fetch, browser, MCP servers, etc. Updated when extensions change.

2. **Integration state.** What's connected (Gmail, Slack, GitHub, etc.), what's available but not connected, what doesn't exist as an option. Updated when user connects/disconnects.

3. **Active automations.** Names, schedules, last run timestamps and status, next run times. Updated when scheduler state changes.

4. **Permission and safety state.** What requires approval (deletions, integration writes, etc.), what's never permitted. Mostly static but can include user-customized settings.

5. **Memory state — meta level.** Not the memories themselves, but a summary: "I have N memories about your work on Permagent and Sidecurrent. I don't have memory access to your email or calendar." This helps the agent calibrate what it knows.

**Tier 2 — On-demand via tool (larger, only sometimes relevant):**

6. **Howto and tutorial knowledge.** Step-by-step instructions for using specific features. "To pause an automation, open the Automate tab, find the automation in Active Automations, click the Pause icon."

7. **Version history and changelog.** What's new in this build, what changed from previous versions. Useful when users ask "did something change?"

8. **Architectural rationale.** Why Permagent does X this way. Useful for power users and developers integrating with Permagent.

The split is intentional. Tier 1 is small enough to inject into every system prompt (low token cost, always current). Tier 2 is large and only sometimes needed (on-demand tool access).

## 4. Architecture

### Tier 1: Always-loaded via Rust runtime

A `SelfKnowledgeBuilder` module in `crates/goose/src/self_knowledge/`. Mirrors the architecture of `ContextBuilder` from Phase 3b.

**API:**

```rust
pub struct SelfKnowledgeBuilder {
    daemon_state: Arc<AppState>,
    static_knowledge: StaticKnowledge,
}

impl SelfKnowledgeBuilder {
    pub fn new(daemon_state: Arc<AppState>) -> Self
    pub async fn current_self_knowledge(&self) -> Result<SelfKnowledge>
}

pub struct SelfKnowledge {
    pub capabilities: CapabilityInventory,
    pub integrations: IntegrationState,
    pub automations: AutomationState,
    pub permissions: PermissionState,
    pub memory_meta: MemoryMetadata,
}
```

**Integration point:** Reply handlers (`reply.rs`, `session_events.rs`) call `current_self_knowledge()` alongside the existing `ContextBuilder.current_digest()` call. Both blocks render into the system prompt before the LLM call.

**Render format:**

```xml
<permagent_self>
<capabilities>
You have access to: shell command execution, file system reading,
web fetching (via curl), system inventory tools.
You do NOT have access to: web browser automation, direct database
access outside Brain.
</capabilities>

<integrations>
Connected: none currently.
Available to connect: Gmail, Slack, GitHub, Google Calendar, Linear,
Notion. The user can connect these in Settings > Integrations.
Not available: WhatsApp, Discord, Microsoft Teams.
</integrations>

<automations>
Active automations:
- Workspace Snapshot (runs weekdays at 8 AM, last ran today at 8:00, succeeded)
- Storage Insights (runs Sundays at 7 PM, never run yet)
You can edit or pause these in the Automate tab.
</automations>

<permissions>
You may suggest file operations but must request user approval for
any deletion. Files always go to Trash, never deleted permanently.
You may not access: ~/.ssh, ~/.aws, ~/.gnupg, anything matching
*.env, *credentials*, *id_rsa*.
</permissions>

<memory>
You have 47 memories tagged 'permagent', 12 tagged 'general',
0 tagged 'getladle' or any other project. The user's most recent
interactions have been about the Automate tab and starter recipes.
</memory>
</permagent_self>
```

**Token cost:** Roughly 200-400 tokens depending on integration count and active automation count. Acceptable for every-turn injection.

**Update frequency:** Built fresh on every chat turn. No caching. The cost of querying daemon state is sub-millisecond; the cost of caching and invalidating it correctly is much higher.

### Tier 2: On-demand via lookup tool

A new agent tool: `query_permagent_knowledge(topic: String) -> KnowledgeResult`.

**Backed by:** A markdown corpus in `crates/goose-server/src/self_knowledge/docs/` containing structured how-to guides, architectural notes, and changelog content. Each document has front-matter tags for retrieval.

**Implementation:**
The tool does keyword and semantic search over the corpus, returning the most relevant document(s). Probably uses Brain's recall infrastructure with a dedicated `wing: "permagent_self"` so the search doesn't pollute regular memory recall.

**System prompt hint (in the always-loaded block):**

> For detailed how-to questions about Permagent's features, call `query_permagent_knowledge` with the topic. Use this for "how do I X" questions, version history queries, and architectural details that aren't in the always-loaded context above.

**Maintenance discipline:** Updates to Permagent features must include corresponding updates to the self-knowledge corpus. This is a process discipline, not a technical guarantee. Recommendation: a CI check that fails when feature code changes without corresponding self-knowledge updates.

## 5. Daemon-side queries needed

The `SelfKnowledgeBuilder` needs to query daemon state for the live portions:

**Capability inventory:**
- Loaded MCP servers via existing config
- Built-in tools available to the current agent context
- Output: list of capability descriptors

**Integration state:**
- Connected MCP servers (which auth tokens are valid)
- Available but not connected (from a static list of supported integrations)
- Output: structured connected/available/unavailable lists

**Automation state:**
- Query existing scheduler endpoints (`/schedule/list` returns this)
- Output: per-job summary with last run, next run, status

**Permission state:**
- Mostly static (defined by Permagent's safety contract)
- User-customized portions read from settings (Phase 4+ when settings exist)
- Output: structured permissions list

**Memory metadata:**
- Query Brain for memory counts by wing
- Get summary of recent activity wings (which projects has the user been working in?)
- Output: meta-level summary

All of these are cheap queries. None require LLM calls. None block on external services.

## 6. Connections to existing Permagent architecture

**Mirrors the ambient awareness pattern.** Phase 3b shipped `current-digest` returning live context for the agent. Self-knowledge is the same pattern applied to product reality instead of user activity. The two blocks coexist in the system prompt.

**Builds on the capability audit.** `docs/inventory/AGENT_CAPABILITY_AUDIT.md` already catalogues what tools the agent has access to. The capability inventory portion of self-knowledge is essentially this content kept current as the daemon evolves.

**Aligns with the Hub System charter pattern.** The Hub System doc describes per-project charters loaded into Henry's context. Self-knowledge is a *product-wide charter* — the agent's grounding in what Permagent itself is. Same conceptual move at different scope.

**Complements memory probe.** `Brain::probe_recent` surfaces memories relevant to current activity. `query_permagent_knowledge` surfaces self-knowledge documents relevant to current questions. Same retrieval pattern, different content domain.

## 7. Failure modes and mitigations

**Stale state in always-loaded block.** Mitigation: query live, never cache. Daemon state is the source of truth.

**Stale documentation in on-demand corpus.** Mitigation: process discipline + CI check linking feature changes to documentation updates. This is the hardest failure to engineer around — it requires culture, not just code.

**Token cost on every turn.** Mitigation: keep always-loaded block under 500 tokens. Trim verbose sections (like memory metadata) to summary form. If costs become an issue, conditionally include sections (e.g., automations only if count > 0).

**Agent ignoring the self-knowledge block.** Mitigation: prompt design that names the block explicitly and instructs the agent to consult it for questions about Permagent. This is the same general issue as ambient context; well-structured prompts mitigate but don't eliminate.

**Hallucinations about features that exist but weren't included in the block.** Mitigation: comprehensive coverage in the always-loaded block (better to be slightly verbose than silently incomplete). Tier 2 lookup catches gaps.

**The agent trusts the block over reality.** This is subtle. If the block says "you have Slack connected" but Slack actually isn't, the agent confidently lies. Mitigation: the block should always be queried fresh from daemon state, not from a stored config that could drift. Test this regularly.

## 8. Rollout plan

**Phase 1 — Always-loaded block (Rust runtime).** ~2-3 days of work.
- SelfKnowledgeBuilder module
- Daemon-side queries for live state
- Static knowledge structure for permissions
- Render function and integration into reply.rs / session_events.rs
- Verification: send "what can you do?" / "what integrations do I have?" / "what automations are running?" and confirm responses match daemon state

**Phase 2 — On-demand lookup tool.** ~3-4 days of work.
- New agent tool registered with the runtime
- Markdown corpus with how-to and architectural content
- Brain-backed retrieval scoped to `wing: "permagent_self"`
- Initial corpus content for top 20 most-likely user questions
- Verification: ask "how do I set up a daily briefing?" and confirm the tool fires and returns useful content

**Phase 3 — CI discipline.** ~1 day setup, ongoing maintenance.
- Test that feature documentation stays in sync with feature code
- Pre-commit hook or CI check that flags feature changes without corresponding doc updates

**Phase 4 — User-facing surfaces.** Optional polish work.
- "What can Permagent do?" command in the chat header
- An onboarding flow that teaches users via the agent rather than via UI tour
- Settings panel showing what the agent knows about itself (debugging aid for users who think the agent is wrong)

## 9. Success metrics

The self-knowledge brain is working if:

- Users can ask "what can you do?" and get an accurate, current, Permagent-specific answer
- Users can ask "how do I X?" for any feature and get a real answer with concrete steps
- The agent stops hallucinating features that don't exist (zero tolerance for this once shipped)
- The agent proactively mentions relevant features when users describe problems ("you might want to set this up as an automation")
- Onboarding feels conversational rather than tutorial-driven
- New features land with self-knowledge updates as part of the change

## 10. Open questions

1. **Where does the static permissions list live?** Hardcoded in Rust? A TOML file? Worth deciding before implementing.

2. **How does the on-demand tool know which corpus document to return?** Pure keyword search, semantic search via embeddings, or hybrid? Embeddings give better results but require Brain integration. Worth prototyping both.

3. **Should the self-knowledge block be visible to the user?** A "what does the agent see about Permagent right now?" view in the inspection panel could be a debugging aid and a trust-building feature.

4. **How do we handle multi-tenant divergence?** Two users running Permagent will have different connected integrations, different automations, different memory. The block needs to be per-user-current, not global. Already handled by the Rust query approach but worth confirming during implementation.

5. **Does this layer need its own update events on the activity bus?** Probably yes — when integrations connect/disconnect, when automations are created/deleted, the agent should know immediately, not on the next chat turn. Could be Phase 1.5 work.

6. **How does this connect to the Hub System's per-project charters?** When hubs ship, each hub has its own charter. Self-knowledge becomes layered: product-wide self-knowledge (what Permagent is) + per-hub charter (what this hub does) + per-turn ambient context (what's happening right now). Worth designing the layering before hubs ship to avoid retrofitting.

7. **How is the corpus shared with users who have customized their install?** If a user disables an extension, the documentation for that extension shouldn't appear in their self-knowledge corpus. The corpus needs to be filtered by the actual capability state.

## 11. Risks and trade-offs

**The maintenance discipline is the hardest part.** Rust code that builds the always-loaded block is straightforward. Markdown documentation that stays in sync with feature changes is hard. This requires culture and process, not just code.

**Token cost compounds with other context.** Phase 3b ambient context is already adding ~500 tokens per turn. Self-knowledge adds ~300-500. Plus session history, plus user message, plus system prompt boilerplate. We're approaching real cost on every turn. Worth watching.

**The on-demand tool failure mode.** Agents notoriously underuse lookup tools. The on-demand corpus only helps if the agent calls it. Mitigation: prompt directives + clear naming + multiple fallback paths.

**Self-knowledge can make hallucinations worse.** If the block is wrong (stale state, incomplete coverage), the agent now confidently presents wrong information backed by what it thinks is authoritative product context. This is worse than no self-knowledge. Quality matters enormously.

## 12. Why this is genuinely differentiating

Most LLM products treat the agent as a feature inside the product. The product has features; the agent is one of them. The agent doesn't know about the other features in any structured way.

Self-knowledge inverts this. The agent becomes the *navigation layer* over the product's features. The product is legible *through* the agent. Users explore the product by asking the agent what's possible.

This is qualitatively different from "having an agent" or "having documentation" or "having a chatbot." It's making the agent into a competent guide for the product itself. The agent that knows what its host product can do is far rarer than the agent that can write code or summarize text — because foundation model training doesn't produce this knowledge. It has to be engineered into each product separately.

Permagent shipping this well, and shipping it before Hub System (when the surface area expands dramatically), would be a meaningful differentiator. Users who try Permagent and find that "the agent actually knows what this app does" will notice immediately. It's the kind of thing that produces "wait, why doesn't every product do this?" reactions.
