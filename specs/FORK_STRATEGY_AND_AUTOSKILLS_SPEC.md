# Permagent: Fork Strategy & Auto-Skills System Spec
**Date:** 2026-04-16  
**Status:** Pre-implementation specification  
**Prerequisite for:** Phase 1 Claude Code tasks

---

## PART 1: GOOSE FORK STRATEGY

### Overview
We are forking Goose (Apache 2.0, Linux Foundation AAIF) as the foundation for Permagent Runtime. This is not a wrapper or dependency—it's a fork that we evolve into our own product over time.

### 1.1 What We Keep As-Is (from Goose)
- **Execution engine:** Task execution, agent loop, state management
- **MCP (Model Context Protocol) system:** Extension architecture for tools/integrations
- **Plugin/extension system:** How third-party tools integrate
- **Multi-LLM provider support:** 25+ provider abstraction layer
- **Desktop packaging:** Native .dmg for macOS distribution
- **Custom Distributions feature:** The mechanism for shipping customized versions

### 1.2 What We Replace Immediately (Phase 1)
- **UI:** Goose's UI → Permagent Command Center (Next.js web app on localhost)
- **Memory system:** Goose's built-in memory → Spectral (temporal knowledge graph + SQLite)
- **Auth:** Goose's auth → Permagent auth (local + optional Mesh registration)
- **Configuration:** Goose's config format → Permagent config (simplified for CLI wizard)

### 1.3 What We Replace Later (Phase 2+)
- **Rust runtime:** Keep Goose's Rust execution for now, plan to migrate to Permagent Runtime (our own)
- **Plugin system:** Evolve MCP → Permagent's native extension format (if needed)
- **Packaging:** Keep .dmg for now, consider alternative distributions later

### 1.4 Fork Maintenance Plan
- **Initial:** Clone Goose repo, create `permagent` branch, start diverging immediately
- **Upstream tracking:** Monitor Goose for critical security patches; cherry-pick if relevant
- **Divergence strategy:** After Phase 1, we own the codebase. Don't track upstream unless it's a security issue
- **Custom Distributions:** Use Goose's Custom Distributions feature to generate Permagent .dmg with:
  - Permagent branding
  - Command Center pre-configured
  - Spectral memory system included
  - Mesh registration enabled by default

### 1.5 Technical Integration Points
```
Goose (forked)
├── Execution Engine (keep)
├── MCP System (keep, extend)
├── Multi-LLM Router (keep)
├── Desktop Packaging (.dmg, keep)
└── Rust Runtime (replace later)

↓ Replace with Permagent layers:

Permagent Runtime (on top of Goose execution)
├── Command Center (Next.js web UI)
├── Spectral Memory System (temporal KG + SQLite)
├── Auto-Skills Engine (new)
├── Mesh Integration (new)
└── CLI Wizard (new)
```

### 1.6 Build & Distribution
- **Build process:** Fork Goose, add Permagent-specific build steps (Command Center bundling, Spectral init)
- **Distribution:** Use Custom Distributions to generate `.dmg` file
- **Installation:** Users download `.dmg`, drag Permagent.app to Applications, run wizard on first launch
- **CLI command:** `permagent` command available in terminal after install (symlink or PATH)

---

## PART 2: AUTO-SKILLS SYSTEM

### Overview
Auto-skills is the stickiest mechanic for compounding agent value. When an agent completes a task, it detects whether that task is worth automating, surfaces a one-click prompt to save it as a reusable skill, and runs it unattended next time.

### 2.1 Skill Detection (How does the agent know something is automatable?)

**Detection triggers:**
1. **Repetition:** Agent completes task X, then user asks for task X again within N days → "Save this as a skill?"
2. **Complexity + Reusability:** Agent completes multi-step task → "This is worth saving. Save as skill?"
3. **User feedback:** User explicitly says "save this as a skill" or "make this automatic"
4. **Pattern analysis:** Agent detects task follows a standard pattern (email filtering, data extraction, scheduling) → suggests automation

**Implementation:**
- Store task execution history in Spectral (task name, steps, inputs, outputs, frequency)
- Run skill detection logic after each task completion
- Surface prompt to user: "You've done this 3 times. Save as a skill?"
- Track which prompts users accept/reject to improve detection

### 2.2 Skill Format (How are skills stored and executed?)

**Skill definition (JSON/YAML in Spectral):**
```json
{
  "id": "skill_gmail_archive_old_promotions",
  "name": "Archive old promotions",
  "description": "Finds promotional emails older than 30 days and archives them",
  "trigger": {
    "type": "schedule",
    "value": "daily at 9am"
  },
  "steps": [
    {
      "action": "gmail.search",
      "params": { "query": "from:promo* before:2026-03-17" }
    },
    {
      "action": "gmail.archive",
      "params": { "message_ids": "${previous.results}" }
    }
  ],
  "inputs": [],
  "outputs": { "archived_count": "number" },
  "created_at": "2026-04-16T12:00:00Z",
  "author": "user_id",
  "version": 1
}
```

**Skill composition:**
- Skills can call other skills (reference by `skill_id`)
- Skills can use outputs from previous steps (template syntax: `${step_name.output}`)
- Skills can have conditional logic (if/then/else)
- Skills can loop (for each item in list, do X)

**Skill versioning:**
- Each skill has a version number
- When user modifies a skill, increment version
- Keep history of all versions (for rollback if needed)
- Mesh can track which version of a skill is most popular

### 2.3 Where Skills Live in Spectral

**Database schema:**
```sql
CREATE TABLE skills (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  definition JSON NOT NULL,
  trigger_type TEXT, -- schedule, manual, webhook, event
  trigger_value TEXT,
  inputs JSON,
  outputs JSON,
  created_at TIMESTAMP,
  updated_at TIMESTAMP,
  version INTEGER,
  is_published BOOLEAN, -- can be shared on Mesh
  mesh_id TEXT, -- if published to Mesh
  status TEXT -- active, paused, archived
);

CREATE TABLE skill_executions (
  id TEXT PRIMARY KEY,
  skill_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  started_at TIMESTAMP,
  completed_at TIMESTAMP,
  status TEXT, -- success, failed, running
  inputs JSON,
  outputs JSON,
  error_message TEXT,
  FOREIGN KEY (skill_id) REFERENCES skills(id)
);

CREATE TABLE skill_triggers (
  id TEXT PRIMARY KEY,
  skill_id TEXT NOT NULL,
  trigger_type TEXT,
  trigger_value TEXT,
  last_triggered_at TIMESTAMP,
  next_trigger_at TIMESTAMP,
  FOREIGN KEY (skill_id) REFERENCES skills(id)
);
```

**In the knowledge graph:**
- Skills are entities in Spectral's temporal KG
- Linked to tasks they automate
- Linked to integrations they use (Gmail, Slack, etc.)
- Linked to other skills they call
- Relationships track: "skill X was created from task Y" and "skill X is used by user Z"

### 2.4 How Skills Compose

**Skill-to-skill calls:**
```json
{
  "id": "skill_weekly_digest",
  "name": "Weekly digest",
  "steps": [
    {
      "action": "skill.call",
      "skill_id": "skill_gmail_archive_old_promotions",
      "params": {}
    },
    {
      "action": "skill.call",
      "skill_id": "skill_slack_summarize_week",
      "params": {}
    },
    {
      "action": "email.send",
      "params": {
        "to": "user@example.com",
        "subject": "Weekly digest",
        "body": "Archived ${step_1.outputs.archived_count} emails. Slack summary: ${step_2.outputs.summary}"
      }
    }
  ]
}
```

**Conditional logic:**
```json
{
  "steps": [
    {
      "action": "gmail.search",
      "params": { "query": "is:unread" }
    },
    {
      "action": "if",
      "condition": "${previous.results.length} > 10",
      "then": [
        { "action": "slack.post", "params": { "text": "You have 10+ unread emails" } }
      ],
      "else": [
        { "action": "slack.post", "params": { "text": "Inbox clean" } }
      ]
    }
  ]
}
```

**Loops:**
```json
{
  "steps": [
    {
      "action": "gmail.search",
      "params": { "query": "label:invoices" }
    },
    {
      "action": "for_each",
      "items": "${previous.results}",
      "do": [
        { "action": "email.forward", "params": { "to": "accounting@company.com" } }
      ]
    }
  ]
}
```

### 2.5 Mesh Sharing Model (How do skills move between agents?)

**Publishing a skill:**
- User clicks "publish to Mesh" on a skill
- Skill is uploaded to Mesh registry with metadata (name, description, inputs, outputs, usage stats)
- User can choose: public (anyone can use) or private (only shared with specific users/teams)

**Discovering skills:**
- Command Center has a "Skill Marketplace" tab
- Users can search/browse published skills
- See usage stats: "Used by 500 agents, 4.8★ rating"
- One-click import: "Add to my agent"

**Skill attribution & monetization:**
- Skill creator gets credit in metadata
- (Future) Revenue sharing: creator gets small cut when their skill is used
- Users can rate/review skills
- Mesh tracks which skills are most popular

**Privacy model:**
- Published skills are sandboxed (they can't access user's private data)
- Skills run in a restricted environment (can't read files, can't modify system)
- Skills declare what integrations they need (Gmail, Slack, etc.)
- User approves what integrations the skill can access before importing

**Skill versioning on Mesh:**
- Each published version is immutable
- When creator updates a skill, it's a new version
- Users can choose which version to use
- Mesh tracks compatibility (skill v2 might not work with old agent versions)

### 2.6 Preventing Skill Bloat

**Strategies:**
1. **Deduplication:** When user tries to save skill, agent checks if similar skill already exists. "You already have 'archive old emails'. Overwrite or create new?"
2. **Skill quality gates:** Only suggest saving skills that have been used 2+ times or take 3+ steps
3. **User control:** Users can delete/archive skills they don't use
4. **Analytics:** Command Center shows "unused skills" dashboard. "You haven't used this skill in 30 days. Delete?"
5. **Mesh reputation:** Low-quality or unused skills get lower visibility in Mesh marketplace

---

## PART 3: IMPLEMENTATION ROADMAP

### Phase 1 (Weeks 1-4)
- [ ] Fork Goose, set up Permagent repo
- [ ] Integrate Spectral memory system with Goose execution
- [ ] Build Command Center UI (basic chat, task creation, event log)
- [ ] Create CLI wizard (API key setup, Spectral init)
- [ ] Package as .dmg using Goose Custom Distributions
- [ ] Implement basic auto-skills detection (repetition-based)
- [ ] Store skills in Spectral
- [ ] Test with 5-10 early users

### Phase 2 (Weeks 5-8)
- [ ] Advanced skill detection (pattern analysis)
- [ ] Skill composition (skill-to-skill calls, conditionals, loops)
- [ ] Skill marketplace UI (browse, import, rate)
- [ ] Mesh skill publishing
- [ ] More integrations (Slack, GitHub, Calendar)
- [ ] Skill execution analytics

### Phase 3 (Weeks 9+)
- [ ] Skill versioning & rollback
- [ ] Revenue sharing for skill creators
- [ ] Agent-to-agent skill delegation (Mesh)
- [ ] Permagent Runtime replacement (move away from Goose Rust)

---

## QUESTIONS TO RESOLVE BEFORE BUILDING

1. **Skill execution:** Do skills run in the background daemon, or in the web Command Center? (Answer: daemon, Command Center just monitors)
2. **Skill permissions:** How granular are permissions? (Answer: per-integration, user approves before import)
3. **Skill triggers:** What trigger types do we support in Phase 1? (Answer: schedule + manual. Webhooks/events in Phase 2)
4. **Mesh authentication:** How do we verify skill creator identity on Mesh? (Answer: OAuth, tied to Mesh registration)
5. **Skill rollback:** If a skill fails, do we auto-rollback or notify user? (Answer: notify user, they decide)

---

**Next step:** Queue claude-code task to flesh out Goose fork setup and Spectral skill schema implementation.
