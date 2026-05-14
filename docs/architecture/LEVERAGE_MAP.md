# Permagent Leverage Map

**Status:** Snapshot as of commit `3f521e94b`
**Date:** 2026-04-30

## Executive Summary

The inherited Goose codebase contains two high-leverage systems hiding in plain sight: (1) a fully functional multi-agent coordination layer via `summon` + `orchestrator`, and (2) a complete autonomous scheduling system via `recipes` + `schedules` that creates sessions, runs agents, and persists results — all without user interaction. Together with `skills` and Spectral Brain, these five surfaces form a workable foundation for Permagent's worker system without building from scratch. The biggest noise comes from 5 ACP adapters (require npm-installed binaries, heavy subprocess lifecycle, developer-tool oriented) and the `tutorial` MCP server (Goose brand content). Provider variety (30 structs) is large but low-cost to carry since unused providers are never instantiated.

---

## Top 5 Leverage Wins

### 1. Recipes + Schedules = Autonomous Agent Work
**Why it matters:** Recipes are full agent configurations (system prompt + tools + model + output schema), and the scheduler fires them on cron expressions, creating sessions and running agents autonomously. This is "your agent works while you sleep" — Permagent's core story — already built.
**Classification:** LEAN ON
**Smallest wiring:** Inject Spectral recall into scheduled agent runs (currently recall/remember only wired in session_events.rs and reply.rs, not in scheduler.rs's `execute_job()`). Add command-center UI for schedule management.

### 2. Summon + Orchestrator = Multi-Agent Foundation
**Why it matters:** `summon` spawns background subagents with async result polling and cancellation. `orchestrator` manages full agent lifecycles (create, message, interrupt). Together they are the worker system primitives — delegation, coordination, and lifecycle management are already implemented and production-ready.
**Classification:** LEAN ON
**Smallest wiring:** Enable `orchestrator` by default (currently hidden/disabled). Wire Spectral recall into subagent runs. Surface active workers in command-center's World View.

### 3. Skills = Reusable Knowledge Loading
**Why it matters:** Skills are discovered from `.goose/skills/` directories as `SKILL.md` files with YAML frontmatter, loaded into agent context on demand. This is the foundation for Permagent's auto-skills / learned behavior system. Skills can be project-local or global.
**Classification:** ADAPT
**Smallest wiring:** Rename discovery paths from `.goose/skills/` to `.permagent/skills/`. Connect skill creation to Spectral (persist learned skills as memories). Surface skills inventory in command-center.

### 4. Autovisualiser = Charts in Chat
**Why it matters:** 8 visualization tools (line charts, sankey diagrams, radar charts, maps, mermaid diagrams) that generate self-contained HTML with embedded JavaScript libraries. High-value for data analysis conversations.
**Classification:** LEAN ON
**Smallest wiring:** Already works via MCP tool dispatch. Needs command-center rendering support for `text/html;profile=mcp-app` content type (may already work if the chat renderer handles HTML).

### 5. Chat History Search (FTS) = Brain View Backend Candidate
**Why it matters:** SQLite FTS over message content with keyword search, date filtering, and session grouping. Already powers the `chatrecall` platform extension. Could serve as the Brain View's search backend alongside Spectral's neural recall.
**Classification:** ADAPT
**Smallest wiring:** Surface search in command-center's Brain workspace. Merge FTS results with Spectral recall results for a unified "memory search" experience.

---

## Top 5 Noise Items

### 1. ACP Adapters (5 adapters)
**Why it doesn't fit:** All 5 require npm-installed binaries (`claude-agent-acp`, `codex-acp`, `copilot`, `pi-acp`, `amp-acp`), spawn heavy subprocesses, and are oriented toward developer tool integration. Permagent's consumer direction doesn't need "run Claude Code as a provider."
**Recommendation:** Feature-gate off in default builds. Keep code for enterprise/developer users who want it.

### 2. Tutorial MCP Server
**Why it doesn't fit:** Contains Goose-branded tutorials ("build-mcp-extension", "first-game"). Permagent needs its own onboarding story, not inherited Goose tutorials.
**Recommendation:** Disable by default. Replace with Permagent-specific onboarding when ready.

### 3. CLI Wrapper Providers (Claude Code, Codex, Cursor Agent, Gemini CLI)
**Why it doesn't fit:** These spawn external CLI tools and let them manage their own context (`manages_own_context() = true`). They're developer-to-developer integrations, not consumer agent infrastructure.
**Recommendation:** Accept as inherited debt. They don't cost anything when unused (never instantiated unless selected).

### 4. Gateway (Telegram only)
**Why it doesn't fit:** Only Telegram is implemented despite the prompt mentioning Slack/Discord. The gateway framework is extensible but has exactly one backend. Not aligned with v1.0 priorities.
**Recommendation:** Defer. The trait-based architecture is clean — add Slack/Discord when the product needs them.

### 5. Memory MCP Server
**Why it doesn't fit:** Simple file-based categorized storage (`.goose/memory/` text files) that overlaps with Spectral Brain's purpose. Two memory systems create confusion.
**Recommendation:** Defer decision. The Memory MCP server serves a different use case (explicit user-directed storage of preferences/configs) vs Spectral Brain (implicit conversation recall). May be worth keeping both with clear differentiation, or migrating Memory's storage to Spectral.

---

## Full Classification Table

| Surface | Works | Aligned | Classification | Wiring Effort | Priority Notes |
|---------|-------|---------|---------------|---------------|----------------|
| **Recipes** | Yes | Yes | LEAN ON | Small | Wire Spectral into recipe execution; add CC UI |
| **Schedules** | Yes | Yes | LEAN ON | Small | Wire Spectral into scheduled runs; add CC UI |
| **Summon (delegate)** | Yes | Yes | LEAN ON | Small | Enable + wire Spectral into subagent runs |
| **Orchestrator** | Yes | Yes | LEAN ON | Small | Enable by default; surface in World View |
| **Autovisualiser** | Yes | Yes | LEAN ON | Small | Verify HTML rendering in CC chat |
| **Developer** | Yes | Yes | LEAN ON | None | Core file/shell tools, already working |
| **Analyze** | Yes | Yes | LEAN ON | None | Tree-sitter code analysis, already working |
| **Todo** | Yes | Partial | LEAN ON | None | Task persistence via session extension data |
| **Skills** | Yes | Yes | ADAPT | Medium | Rename paths, connect to Spectral, CC surface |
| **Chat History FTS** | Yes | Yes | ADAPT | Medium | Surface in Brain View, merge with Spectral recall |
| **Chatrecall** | Yes | Yes | ADAPT | Small | Already Permagent-built; extend with Spectral |
| **Tom (Top of Mind)** | Yes | Partial | ADAPT | Small | Useful context injection; rebrand env vars |
| **Apps** | Yes | Partial | ADAPT | Medium | HTML app generation; needs CC rendering surface |
| **Computercontroller** | Yes | Partial | ADAPT | Medium | PDF/DOCX/XLSX processing useful; computer control is v1.x |
| **Extension Manager** | Yes | Yes | LEAN ON | None | Runtime extension enable/disable, already working |
| **Summarize** | Yes | Partial | DEFER | Small | Disabled by default; single-LLM-call file summary |
| **Code Execution** | Yes | Partial | DEFER | Medium | TypeScript/Bash execution with tool callbacks; security-sensitive |
| **Prompts** | Yes | Partial | DEFER | Small | Template CRUD; unclear if needed beyond system.md |
| **Session Fork** | Yes | Partial | DEFER | None | Copies conversation; useful but not v1.0 priority |
| **Session Import/Export** | Yes | Yes | LEAN ON | None | Privacy/portability story; JSON format |
| **Session Insights** | Yes | Partial | DEFER | None | Aggregate stats; surface in CC when ready |
| **Action Required** | Yes | Yes | LEAN ON | None | Tool permission confirmation; already working |
| **Local Inference** | Yes | Yes | ADAPT | Small | GGUF model management; feature-gated; local-first story |
| **Integrations (Gmail)** | Partial | Partial | DEFER | Medium | Gmail OAuth exists; only readonly scope |
| **Tunnel** | Unknown | Unclear | DEFER | Unknown | Start/stop/status lifecycle; purpose unclear |
| **Anthropic Provider** | Yes | Yes | LEAN ON | None | Primary provider, already verified |
| **OpenAI Provider** | Yes | Yes | LEAN ON | None | Major provider, already working |
| **Google Provider** | Yes | Yes | LEAN ON | None | Gemini API, already working |
| **Ollama Provider** | Yes | Yes | LEAN ON | None | Local model serving, aligned |
| **Local Inference Provider** | Yes | Yes | ADAPT | Small | llama-cpp backend; feature-gated |
| **OpenRouter** | Yes | Yes | LEAN ON | None | Multi-provider gateway |
| **Other API Providers** | Yes | Partial | DEFER | None | Azure, GCP Vertex, Snowflake, Venice, etc. |
| **OAuth Providers** | Yes | Partial | DEFER | None | Kimi Code, ChatGPT Codex, Gemini OAuth, etc. |
| **CLI Wrapper Providers** | Yes | No | IGNORE | None | Claude Code, Codex, Cursor, Gemini CLI |
| **ACP Adapters (5)** | Unknown | No | IGNORE | None | Require npm binaries; developer-tool oriented |
| **AWS Providers** | Unknown | Partial | DEFER | None | Feature-gated; enterprise use case |
| **Tutorial MCP** | Yes | No | IGNORE | None | Goose-branded content |
| **Memory MCP** | Yes | Unclear | DEFER | Medium | Overlaps with Spectral; different use case |
| **Gateway (Telegram)** | Unknown | Partial | DEFER | Medium | Only Telegram implemented |

---

## Multi-Agent Foundation Assessment

### The Primitives

| Primitive | Surface | Status | Role in Worker System |
|-----------|---------|--------|----------------------|
| **Task Definition** | Recipes | Working | A worker's job description: prompt + tools + model + output schema |
| **Task Delegation** | Summon (`delegate`) | Working | Spawn a worker, give it a recipe/instructions, get results |
| **Worker Lifecycle** | Orchestrator | Working (hidden) | Create, message, monitor, interrupt workers |
| **Autonomous Execution** | Schedules | Working | Fire workers on cron; "work while you sleep" |
| **Learned Behavior** | Skills | Working | Workers load reusable skills from filesystem |
| **Memory** | Spectral Brain | Working | Recall past context before acting; remember results after |
| **Context Injection** | Tom (MOIM) | Working | Inject standing instructions into every turn |

### What's Missing for a Worker System

1. **Spectral wiring into scheduled/delegated runs.** Currently recall/remember only fires in the two HTTP chat handlers (session_events.rs, reply.rs). The scheduler's `execute_job()` in `scheduler.rs` creates its own agent and session but never touches the Brain. Summon's subagent runs also bypass Brain. **This is the single highest-value wiring task.**

2. **Worker-to-worker communication.** Summon and Orchestrator both exist but don't know about each other. A worker spawned by Summon can't be managed by Orchestrator, and vice versa. Unifying them under a single worker registry would make the system coherent.

3. **Lab View / World View representation.** Workers are invisible. The World View component is a placeholder (globe emoji). Surfacing active workers, their sessions, and their status in the World View would make the worker system tangible.

4. **Worker identity / persona.** Workers inherit the parent's provider and extensions but not a persona. A worker should know its role ("Librarian," "Researcher," "Builder") and carry that identity through its interactions.

### Verdict: Partial — Strong Foundation, Needs Wiring

The combination of summon + orchestrator + skills + recipes + schedules forms approximately 70% of a workable worker system. The primitives exist and function. What's missing is not new capability but **integration**: Spectral memory in worker runs, unified worker registry, visual representation, and persona awareness. This is wiring work, not greenfield development.

---

## Provider Strategy Recommendation

### Ship in Default v1.0 Build (Always Available)

| Category | Providers | Rationale |
|----------|-----------|-----------|
| API-based (mainstream) | Anthropic, OpenAI, Google | Core providers users expect |
| Multi-provider gateways | OpenRouter, NanoGPT | Flexibility without config burden |
| Local | Ollama | Local-first story; no API key needed |

### Ship but Non-Default (Available When Configured)

| Category | Providers | Rationale |
|----------|-----------|-----------|
| Enterprise API | Azure, Databricks, GCP Vertex, Snowflake | Enterprise customers need these |
| Alternative API | Tetrate, LiteLLM, Avian, XAI, Venice | Low-cost to carry; useful for power users |
| OAuth-based | GitHub Copilot, Kimi Code, ChatGPT Codex, Gemini OAuth | Users with existing subscriptions |

### Feature-Gate Off (Opt-In Only)

| Category | Providers | Rationale |
|----------|-----------|-----------|
| AWS | Bedrock, SageMaker TGI | Already feature-gated; heavy dependencies |
| Local Inference | Local Inference (llama-cpp) | Already feature-gated; Metal/CUDA compile-time |
| CLI Wrappers | Claude Code, Codex, Cursor, Gemini CLI | Developer tools, not consumer agent |
| ACP Adapters | Claude ACP, Codex ACP, Copilot ACP, Pi ACP, Amp ACP | Require npm binaries; developer-oriented |

---

## Brain View Backend Strategy

### Current State

Two search backends exist:

1. **Chat History FTS** (`chat_history_search.rs`) — SQLite LIKE queries over `json_extract(message.content, '$.text')`. Keyword-based, grouped by session, with date filtering. Already powers the `chatrecall` platform extension.

2. **Spectral Brain Recall** (`brain.recall()`) — Neural fingerprint matching over the memory store. Score-based ranking (`signal_score`), semantic similarity, returns memory hits with source and confidence.

### Relationship

They search **different data stores**:
- FTS searches **session messages** (the raw conversation history in the `messages` table).
- Spectral recall searches **memories** (distilled turn summaries written by the remember phase to `memory.db`).

They use **different algorithms**:
- FTS: keyword substring matching (`%keyword%` LIKE patterns)
- Spectral: semantic fingerprint similarity with scored ranking

### Verdict: Partial — Use Both, Merge Results

Neither alone is sufficient for Brain View. The ideal Brain View search combines:
- **Spectral recall** for semantic/contextual search ("things related to this concept")
- **FTS** for exact keyword search ("find the conversation where I mentioned 'Polybot'")

**Recommendation:** Brain View's search should query both backends and merge results. FTS provides precision (exact matches), Spectral provides recall (semantic similarity). Present unified results ranked by relevance. The chatrecall extension already demonstrates the FTS path; extending it to also query Spectral recall would create the unified search.

---

## Recommendations

### Wire to Spectral Immediately
1. **Scheduler's `execute_job()`** — add recall before agent invocation, remember after completion (same pattern as session_events.rs phases 3+4)
2. **Summon's subagent runs** — inject parent's recall context into subagent system prompt
3. **Skills** — persist learned/created skills as Spectral memories for cross-session discovery

### Surface in Command-Center for v1.0
1. **Schedule management** — create/list/pause/run schedules (10 API endpoints already exist)
2. **Active workers** — show running subagents in World View (orchestrator's `list_sessions` provides the data)
3. **Brain View search** — unified search combining FTS + Spectral recall
4. **Recipe browser** — list/create/run recipes (11 API endpoints already exist)

### Feature-Gate Off in Default Builds
1. **ACP adapters** (5) — require npm binaries, developer-oriented
2. **Tutorial MCP** — Goose-branded content
3. **CLI wrapper providers** (4) — developer tools managing their own context

### Order of Operations for Maximum Leverage

1. **Spectral wiring into scheduler** (small: ~100 lines, same pattern as session_events.rs)
2. **Enable orchestrator by default** (trivial: change `default_enabled: false` to `true`)
3. **Command-center schedule UI** (medium: CRUD against existing 10 endpoints)
4. **Brain View search** (medium: merge FTS + Spectral recall, new CC component)
5. **World View worker display** (medium: consume orchestrator's list_sessions, render in WorldView)
6. **Skills path rename + Spectral integration** (medium: path config + remember_with)

Steps 1-2 unlock the worker system with zero UI work. Steps 3-5 make it visible. Step 6 makes it learn.
