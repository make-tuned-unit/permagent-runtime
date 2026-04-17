# Permagent Specification — Opus Feedback Incorporated
**Date:** 2026-04-16  
**Status:** Phase 1 Scope Finalized

---

## Executive Summary

Permagent is a local-first agent OS built on a fork of Goose (Apache 2.0, Linux Foundation AAIF). Users run a terminal command to install the runtime, connect their API keys, and access a web-based Command Center. The agent learns through auto-skills detection and runs locally on their machine. Phase 1 ships the core loop; Mesh sharing and advanced features come in Phase 2+.

---

## Core Architecture

### Foundation: Goose Fork
- **Runtime: Rust via Goose fork (DECIDED 2026-04-16, not Node.js).**

**Keep as-is:** Execution engine (Rust), Tauri shell primitives, MCP toolshed system, plugin architecture, multi-provider LLM provider traits
- **Replace immediately:** UI (→ Command Center web app), memory system (→ Spectral), authentication
- **Replace in Phase 2+:** Packaging (.dmg with signing/notarization), runtime evolution (toward native Permagent runtime)

### Memory System: Spectral
- Temporal knowledge graph (SQLite-backed, local)
- Stores: facts, relationships, events, skills, user preferences
- Lives in `~/.permagent/spectral/` (user's local machine)
- Survives daemon restarts; all data stays local

### Execution: Daemon + Command Center
- **Daemon:** Runs in background (started by CLI wizard, managed as system service)
- **Command Center:** Web app on `localhost:3000`, connects to daemon via WebSocket
- **CLI Wizard:** One-command setup (API keys, Spectral init, daemon bootstrap)

---

## Phase 1: Core Loop (4 weeks)

### What ships:
1. **Goose fork** with Spectral integration
2. **Command Center** (web app, localhost only)
3. **CLI wizard** for setup and daemon management
4. **Auto-skills detection** (repetition-based only)
5. **One integration:** Gmail (read-only, for initial learning)

### What does NOT ship in Phase 1:
- .dmg packaging (deferred to Phase 2)
- Mesh sharing or network features
- Scheduled skill execution
- Pattern-based skill detection
- Sandbox/security for shared skills

### Auto-Skills System (Phase 1)

**Detection Trigger:** Repetition only
- Agent tracks user actions and completed tasks
- When a task is completed a second time within 7 days, surface a prompt: "You did this before. Save as a skill?"
- User clicks yes → skill is saved to Spectral
- Skill runs unattended next time the trigger condition is met

**Skill Format:**
```json
{
  "id": "skill-uuid",
  "name": "Weekly report generation",
  "trigger": {
    "type": "schedule",
    "cron": "0 9 * * 1"
  },
  "actions": [
    {
      "tool": "gmail",
      "action": "search",
      "params": {"query": "label:work"}
    },
    {
      "tool": "claude",
      "action": "summarize",
      "params": {"context": "previous_step_output"}
    }
  ],
  "output": {
    "type": "email",
    "recipient": "user@example.com"
  },
  "created_at": "2026-04-16T12:00:00Z",
  "usage_count": 5
}
```

**Skill Storage in Spectral:**
- Separate `skills` table with versioning
- Linked to knowledge graph (skill dependencies, learned patterns)
- Queryable by trigger type, tool usage, frequency

**Skill Composition:**
- Phase 1: Skills are linear sequences (no branching, no skill-to-skill calls)
- Phase 2: Allow skills to call other skills, add conditional logic

**Mesh Sharing Model (Phase 2):**
- Skills can be published to Mesh with user consent
- Shared skills run in user's local daemon (not on shared server)
- Rating/discovery system in Mesh Forum
- Revenue share model (TBD in Phase 2 spec)

---

## Critical Issues Addressed

### Issue 1: Skill Detection Noise
**Problem:** Detecting automation candidates by "complexity + reusability" is too noisy and creates decision fatigue.

**Solution:** Phase 1 uses repetition-only detection. Gate on: "User completed this task 2+ times in last 7 days." This is conservative and high-signal. Pattern analysis (complexity heuristics, multi-step detection) moves to Phase 2.

---

### Issue 2: Daemon Scheduling Recovery
**Problem:** If the daemon isn't running when a scheduled skill is supposed to fire, what happens?

**Solution (Phase 1):** No scheduled skills in Phase 1. Skills in Phase 1 are triggered manually or by user action.

**Solution (Phase 2):** Before shipping scheduled skills, implement a missed-trigger recovery policy:
- **Option A:** Run immediately on daemon restart (catches up)
- **Option B:** Skip the missed execution and resume on next scheduled time
- **Option C:** Alert user and ask what to do

Decision: Implement Option A (catch-up on restart) as default, with user override in settings.

---

### Issue 3: Mesh Sandbox/Security
**Problem:** "Skills run in a restricted environment" is a goal, not a guarantee. Shipping Mesh skill sharing without actual sandboxing is a security liability.

**Solution (Phase 1):** Don't ship Mesh sharing in Phase 1. Skills stay local.

**Solution (Phase 2):** Before enabling Mesh skill sharing, implement concrete sandboxing:
- Option A: Run skills in isolated container (Docker, lighter weight)
- Option B: OS-level capability restrictions (Linux seccomp, macOS sandbox)
- Option C: Allowlist model (skills can only call pre-approved tools)

Decision: Implement Option C (allowlist) for Phase 2 MVP. Upgrade to containerization in Phase 3 if needed.

---

### Issue 4: Phase 1 Timeline Reality Check
**Problem:** Goose fork + Spectral + Command Center + CLI wizard + auto-skills + .dmg packaging in 4 weeks is aggressive.

**Solution:** Cut .dmg packaging from Phase 1. Ship Command Center as a local web app only.

**Revised Phase 1 deliverables:**
- [ ] Fork Goose repository
- [ ] Integrate Spectral memory system
- [ ] Build Command Center (web app, localhost:3000)
- [ ] Implement CLI wizard (API key setup, daemon bootstrap)
- [ ] Auto-skills detection (repetition-only)
- [ ] Gmail integration (read-only)
- [ ] Test on 5+ machines (not yours)

**Estimated timeline:** 4 weeks (realistic)

**Phase 2 additions:**
- [ ] .dmg packaging (signing, notarization, auto-updates)
- [ ] Daemon scheduling + recovery policy
- [ ] Mesh skill sharing + sandbox implementation
- [ ] Pattern-based skill detection

---

## Installation & Setup (Phase 1)

```bash
# One command to install and run wizard
curl https://permagent.ai/install | bash

# Or via npm
npm install -g permagent
permagent setup

# Wizard prompts:
# 1. Which LLM provider? (OpenAI / Anthropic / Local Ollama / etc.)
# 2. API key for [provider]?
# 3. Create Spectral memory? (yes/no)
# 4. Start daemon now? (yes/no)
# 5. Open Command Center? (yes/no)

# Daemon starts in background
# Command Center opens at localhost:3000
```

---

## Command Center (Phase 1)

**Three main sections:**

1. **Chat Pane:** Talk to your agent
   - Type message
   - Agent responds
   - Shows reasoning/tool calls
   - "Save as skill?" prompt appears after repetition

2. **Skills Library:** View saved automations
   - List of all skills
   - Usage count, last run, next scheduled run
   - Edit/delete/duplicate

3. **Spectral Memory:** View what the agent knows
   - Knowledge graph visualization
   - Search by entity, date, relationship
   - Manual memory entry (for user to add context)

---

## Goose Fork: Technical Details

**Repository:** Fork from github.com/block/goose (or AAIF version)

**Immediate changes:**
- Remove Goose UI (we're replacing it)
- Add Spectral memory adapter (replaces Goose's memory)
- Update MCP tool definitions to work with Spectral
- Add daemon mode (background service management)
- Add WebSocket server for Command Center communication

**Keep as-is:**
- Execution engine (Rust, proven)
- LLM provider integrations (25+ providers)
- MCP extension system
- Plugin architecture

**Defer to Phase 2:**
- Native Permagent runtime (replace Goose's Rust engine)
- Advanced scheduling
- Distributed execution

---

## Success Metrics (Phase 1)

- [ ] CLI wizard works on 5+ test machines (not developer machines)
- [ ] Command Center loads and connects to daemon
- [ ] Auto-skills detection triggers correctly after 2nd task completion
- [ ] Gmail integration reads emails and learns from them
- [ ] Spectral stores 100+ facts per user after 1 week of use
- [ ] Zero console errors, no missing dependencies
- [ ] 50+ early users sign up for Phase 2 beta

---

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Goose fork maintenance overhead | Start with minimal fork; track upstream changes quarterly |
| Daemon process management (start/stop/restart) | Use OS-standard service managers (systemd on Linux, launchd on macOS) |
| Spectral integration complexity | Build adapter layer; test with mock Goose execution first |
| CLI wizard UX is confusing | Test with 3 non-technical users before Phase 1 ship |
| Auto-skills trigger too often | Conservative repetition threshold (2x in 7 days); gather feedback and adjust |
| Gmail integration breaks on API changes | Implement retry logic and clear error messages |

---

## What's NOT in Phase 1

- Evolution/character design
- Lab 3D visualization
- Forum/agent networking
- Mesh skill sharing
- .dmg packaging
- Scheduled skills
- Pattern-based automation
- Mobile app
- Cloud sync

**These are Phase 2+ decisions.**

---

## Next Steps

1. **Spec review:** Does this align with Jesse's vision?
2. **Goose fork planning:** Which version of Goose do we fork? (block/goose or AAIF version?)
3. **Claude Code task:** Queue the Goose fork + Spectral integration work
4. **Early user recruitment:** Who are the 5 test machines for Phase 1?

---

**Document version:** 1.1  
**Last updated:** 2026-04-16 12:11  
**Status:** Ready for implementation
