# Permagent Phase 1 Spec — Opus Gap Fixes
**Date:** 2026-04-16  
**Status:** Ready for Claude Code task

---

## Gap 1: Auto-Skills Detection Needs Signal

**Problem:** Phase 1 spec lists "Gmail integration (read-only)" as the sole integration, but auto-skills detection is supposed to track "completed tasks." If Gmail is read-only and there's no action-taking integration, what tasks are being detected and repeated?

**Solution:** Add a generic task/action log to Spectral where the agent records all completed work (regardless of integration). The repetition detector reads from this log.

**Implementation:**
- Spectral gets a `tasks` table: `id, timestamp, description, tool_used, status, repetition_count`
- When agent completes any task (via any integration or internal action), it logs to this table
- Repetition detector queries: "tasks with same description + same tool_used in last 7 days, count >= 2"
- When threshold is met, surface "Turn this into a skill?" prompt

**Phase 1 integrations (revised):**
1. Gmail (read-only) — learn from emails
2. Slack (write-capable) — post summaries, create reminders
3. Internal task log — track all completed work

This gives the repetition detector real signal to work with.

---

## Gap 2: Daemon Scheduling Recovery Policy

**Problem:** Spec mentions scheduled skills but doesn't address: what happens when the daemon isn't running when a skill is scheduled to fire?

**Decision needed before Phase 1:**
- **Option A:** Run immediately on next daemon start (catch-up mode)
- **Option B:** Skip the missed run (fire-and-forget)
- **Option C:** Alert the user that a skill was missed

**Recommendation:** Option A (catch-up) + Option C (alert). If a scheduled skill misses its window, run it immediately when the daemon restarts, and notify the user via the Command Center.

**Implementation detail:** Store `last_run_timestamp` for each scheduled skill. On daemon start, check for any skills where `next_scheduled_run <= now` and `last_run_timestamp < next_scheduled_run`. Execute those immediately and log the catch-up.

---

## Gap 3: Mesh Sandbox Implementation

**Problem:** Spec says "skills run in a restricted environment (can't read files, can't modify system)" but doesn't specify *how*. This is a goal, not an implementation.

**Decision needed before Phase 2:**
- Container-based sandbox (Docker/OCI)?
- OS-level capability restrictions (seccomp, AppArmor)?
- Capability-based security model (what can a skill do)?
- Allowlist/denylist of system calls?

**For Phase 1:** Don't ship Mesh skill sharing. Skills stay local. Phase 2 specs the sandbox mechanism before users can upload skills to the Mesh.

---

## Gap 4: Phase 1 Timeline Too Aggressive

**Problem:** Goose fork + Spectral integration + Command Center + CLI wizard + auto-skills detection + .dmg packaging in 4 weeks is too much.

**Opus's recommendation:** Cut .dmg packaging from Phase 1. Ship Command Center as a local web app (localhost:3000). Packaging (signing, notarization, update feed) moves to Phase 2.

**Revised Phase 1 scope:**
- Fork Goose (AAIF or block/goose — TBD)
- Integrate Spectral memory system
- Build Command Center (Next.js web app, runs on localhost:3000)
- CLI wizard to bootstrap runtime + Spectral
- Auto-skills detection (repetition-only, reads from task log)
- Slack + Gmail integrations
- No .dmg, no Mesh sharing

**Revised Phase 1 timeline:** 4-6 weeks (realistic)

**Phase 2 timeline:** 2-3 weeks
- .dmg packaging (signing, notarization, updates)
- Daemon scheduling recovery policy
- Mesh skill sharing (with sandbox implementation)
- Pattern-based skill detection

---

## Three Decisions to Lock Before Claude Code Task

### 1. Goose Fork Source

**Options:**
- **AAIF version** (Linux Foundation, newer, officially maintained)
- **block/goose** (original, more active community, better docs)

**Action:** Check GitHub activity on both. Which has more recent commits, better issue response time, active maintainers? Fork that one.

**Decision owner:** Jesse or Henry

---

### 2. macOS-First for Phase 1

**Confirm:** Phase 1 targets macOS only. launchd for daemon management. Linux/Windows deferred to Phase 2.

**This simplifies:**
- CLI wizard (no cross-platform path handling)
- Daemon service management (launchd only)
- .dmg packaging deferred
- Testing scope (one OS)

**Decision owner:** Jesse

---

### 3. WebSocket vs. HTTP Polling

**Spec says:** WebSocket for Command Center ↔ daemon communication

**Confirm this is correct:**
- Lower latency (important for real-time event streaming)
- Persistent connection (daemon can push events to UI)
- Better for task execution feedback (watch tasks complete in real-time)

**If yes:** Daemon needs a WebSocket server built in. Command Center connects to `ws://localhost:3000/events` on startup.

**Decision owner:** Henry or Claude Code

---

## Ready for Claude Code Task

Once these three decisions are locked, the Claude Code task is:

**Task:** Spec the Goose fork integration, Spectral memory layer, Command Center architecture, CLI wizard, and auto-skills detection system. Include:
1. Which Goose pieces stay, which get replaced
2. Spectral schema (users, memories, tasks, skills, integrations)
3. Command Center wireframes (chat, task log, memory dashboard)
4. CLI wizard flow (API keys, Spectral init, daemon start)
5. Auto-skills detection algorithm (repetition-only, threshold, UI prompt)
6. WebSocket event schema (task created, task completed, memory added, etc.)
7. Slack + Gmail integration architecture
8. Estimated lines of code and time to implement Phase 1

---

## Summary

**Gap 1 (fixed):** Auto-skills detection now has signal via task log + Slack integration.  
**Gap 2 (noted):** Daemon scheduling recovery policy deferred to Phase 2, but decision needed before shipping.  
**Gap 3 (noted):** Mesh sandbox deferred to Phase 2, no skill sharing in Phase 1.  
**Gap 4 (fixed):** .dmg packaging moved to Phase 2, Phase 1 is web app only.  

**Three decisions locked in:** Goose fork source, macOS-first, WebSocket architecture.

**Next:** Lock the three decisions, then queue Claude Code task for full Phase 1 architecture spec.
