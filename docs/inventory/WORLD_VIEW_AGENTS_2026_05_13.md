# World View Agents — Diagnostic Report (2026-05-13)

## 1. INITIAL_AGENTS (full array)

Source: `ui/command-center/src/components/world/useAgentStates.ts:14-65`

| # | id         | name           | role           | togaTrimColor | isHenry | Initial Position        |
|---|-----------|----------------|----------------|---------------|---------|-------------------------|
| 1 | `henry`    | Henry          | `orchestrator` | `#00D9FF`     | true    | (0, 0, 0)               |
| 2 | `aria`     | Aria           | `agent`        | `#FFB347`     | false   | (4, 0, 2)               |
| 3 | `felix`    | Felix          | `agent`        | `#FF6B9D`     | false   | (-3, 0, -4)             |
| 4 | `nova`     | Nova           | `agent`        | `#A78BFA`     | false   | (-5, 0, 3)              |
| 5 | `librarian`| The Librarian  | `agent`        | `#8B7E6F`     | false   | (14, 10.15, 0) (mezz)   |

## 2. Per-agent analysis

### Henry
- **Source**: Hardcoded in INITIAL_AGENTS
- **Identity layer**: `~/.permagent/agent.yaml` → `primary.first_name: Henry` — this IS the configured primary agent
- **Backend wiring**: The `orchestrator` platform extension exists (`crates/goose/src/agents/platform_extensions/orchestrator.rs`) — manages agent sessions (list/view/start/stop). Henry is the chat agent identity the user talks to.
- **World View behavior**: Static character with random wander (ground level, y=0). `isHenry: true` gives him unique visual treatment.
- **Status**: **Real — identity matches agent.yaml primary persona**

### Aria
- **Source**: Hardcoded in INITIAL_AGENTS
- **Identity layer**: `PrimaryPersona::default()` uses `first_name: "Aria"` as the hardcoded Rust default — but agent.yaml overrides this to "Henry". Aria is the **upstream default name that was superseded**.
- **Backend wiring**: None. No platform extension, no worker entry in agent.yaml, no API.
- **World View behavior**: Static character with random wander (ground level). No HUD, no stats, no click action beyond third-person camera.
- **Status**: **Placeholder — vestige of upstream default persona name**

### Felix
- **Source**: Hardcoded in INITIAL_AGENTS
- **Identity layer**: Not referenced anywhere in Rust crates, agent.yaml, or config. Pure invention.
- **Backend wiring**: None.
- **World View behavior**: Static character with random wander.
- **Status**: **Placeholder — no backing identity or capability**

### Nova
- **Source**: Hardcoded in INITIAL_AGENTS
- **Identity layer**: Not referenced anywhere in Rust crates, agent.yaml, or config. Pure invention.
- **Backend wiring**: None.
- **World View behavior**: Static character with random wander.
- **Status**: **Placeholder — no backing identity or capability**

### The Librarian
- **Source**: Hardcoded in INITIAL_AGENTS
- **Identity layer**: `librarian` platform extension registered in `PLATFORM_EXTENSIONS` map (`crates/goose/src/agents/platform_extensions/mod.rs:264-274`). Real Rust code with tools (`describe_memory`, `list_undescribed`), Ollama integration, BATCH_MUTEX, warm-date state.
- **Backend wiring**: Fully wired. Extension loads at session creation, scheduler ticks in `ollama.rs`, LibrarianHUD component fetches live stats via `/api/ollama/librarian-status`.
- **World View behavior**: Constrained to mezzanine ring (y=10.15, radius ~14). Has dedicated `LibrarianHUD.tsx` overlay showing batch progress and status.
- **Status**: **Real — fully wired platform extension with live data**

## 3. /api/agents endpoint

```
$ curl -sv http://localhost:3001/api/agents ...
< HTTP/1.1 404 Not Found
< content-length: 0
```

**The endpoint does not exist.** Not stubbed at `[]`, not implemented at all. Returns 404. There is an `agent.rs` route file but it handles session/extension management, not agent listing.

## 4. agent.yaml vs World View mismatch

### What agent.yaml defines

| Key       | Type    | Identity           | Notes                                  |
|-----------|---------|-------------------|----------------------------------------|
| `primary` | Primary | Henry              | The chat agent. Overrides default "Aria" |
| `workers.archivist` | Worker | Archivist | Role: "Library research and overnight memory consolidation" — **not rendered in World View** |

### What PLATFORM_EXTENSIONS registers (real capabilities)

Analyze, Todo, Apps, Chat Recall, Extension Manager, Summon, Summarize, Developer, **Orchestrator**, Top Of Mind, Skills, **Librarian** — plus Code Mode (feature-gated).

### Mismatch summary

| World View Agent | agent.yaml | Platform Extension | Verdict |
|-----------------|------------|-------------------|---------|
| Henry           | primary ✅  | Orchestrator ✅    | **Aligned** — real identity + real capability |
| Aria            | ❌ (superseded default) | ❌ | **Ghost** — old default name, nothing behind it |
| Felix           | ❌          | ❌                 | **Ghost** — invented, nothing behind it |
| Nova            | ❌          | ❌                 | **Ghost** — invented, nothing behind it |
| The Librarian   | ❌ (not in yaml) | librarian ✅ | **Aligned** — real extension, but identity not in agent.yaml (it's a platform extension, not a persona) |
| Archivist       | workers.archivist ✅ | ❌ | **Missing from World View** — configured worker with no visual representation |

## 5. Recommended actions

| Agent      | Action | Rationale |
|-----------|--------|-----------|
| Henry     | **Keep** | Real primary agent, wired to orchestrator |
| Aria      | **Delete or repurpose** | Placeholder. If future multi-agent needs a second persona, wire it to a real worker; otherwise remove. |
| Felix     | **Delete** | Pure decoration. No path to real data without new infrastructure. |
| Nova      | **Delete** | Same as Felix. |
| Librarian | **Keep** | Fully wired, has live HUD |
| Archivist | **Consider adding** | Has a worker entry in agent.yaml but no visual presence. Low priority — only relevant if workers get their own sessions. |

## 6. Key finding

3 of 5 visible agents (Aria, Felix, Nova) are purely decorative placeholders with no backend identity, no API data, and no platform extension. They wander randomly with zero connection to real state. The World View currently conveys a false impression of multi-agent activity.

The `/api/agents` endpoint doesn't exist, so even if we wanted to drive World View from real data, there's no API to consume yet.

## 7. Post-script: Archivist cleanup

- "Archivist" identified as vestigial — it was an earlier name for the Librarian (same agent, two names)
- Cleanup completed via PR `chore/remove-archivist-vestige`: removed default worker entry from `agent_identity.rs`, updated docs
- 16 historical references in Brain memory left intact (conversational record from the rename period)
- `mod.rs:270` "Memory archivist" kept as-is — lowercase descriptive English, not a vestigial entity reference
