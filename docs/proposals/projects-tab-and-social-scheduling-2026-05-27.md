# Scope Proposal: Projects Tab + Social Media Scheduling

**Date:** 2026-05-27
**Author:** CC session (backlog-next worktree)
**Status:** DRAFT — awaiting Jesse's answers to open questions

---

## 1. Issues Addressed

### Projects Tab (Kanban view)

| Issue | Title | Status |
|-------|-------|--------|
| #69 | Epic: Projects as workspaces (Phase 2) | Open (epic) |
| #70 | Schema + project_id propagation | Open (partially delivered by PR #189) |
| #71 | [Projects] Workspace UI | Open — **primary target** |
| #72 | Details panel — services and credentials reference | Open |
| #73 | Details panel — resources, people, activity | Open |

**PR #189** (in CI) delivers the schema foundation: `projects` + `project_tags` tables, REST CRUD, MCP tools, system prompt, and `ProjectChip` in the Build tab. This is the prerequisite for #71.

**Epic #70** remains open for FK propagation (`project_id` on memories/entities/goals/tasks/skills). The Kanban view does NOT block on this — it can use app-layer joins until FK columns land.

### Social Media Scheduling

**No issue exists.** Searched all 200+ issues (open and closed) across terms: social, schedule, post, media, content, twitter, publish, buffer, campaign, feed, draft, queue, x.com, linkedin, instagram, facebook. Zero matches.

**Action needed:** Jesse should file this issue before we scope it. Key questions below.

---

## 2. Recommended Scope for This PR Cycle

### Tier 1: Projects Workspace UI (#71)

Build the workspace shell that lives in the existing **Build tab** (or a new "Projects" tab — see open question #1). This is the Kanban-style view from Epic #69 step 3.

**What to build:**
- Project selector dropdown (consuming `/api/projects` from PR #189)
- Kanban board skeleton with columns by status/goal-state
- Card components for goals/tasks (placeholder data until orchestrator primitives exist)
- Project switching loads the correct board context
- Right panel skeleton (details placeholder — #72/#73 content comes later)

**Surfaces touched:**
- UI: new components in `ui/command-center/src/components/` (ProjectsView, KanbanBoard, KanbanColumn, KanbanCard, ProjectSelector)
- REST: consumes existing `/api/projects/*` endpoints from PR #189
- No new DB migrations needed (schema already delivered)
- No new MCP tools (CRUD tools already in PR #189)

**Estimated effort:** 3-5 CC hours (UI-heavy, no backend changes)

### Tier 2: Social Media Scheduling (issue TBD)

Cannot scope without an issue and answers to fundamental design questions. This is a significantly larger feature with external API dependencies.

**Rough shape (if Jesse confirms the vision):**
- New schema: `scheduled_posts` table (project_id FK, platform, content, media refs, scheduled_at, status, posted_at, external_id)
- New REST endpoints: CRUD for scheduled posts, trigger/cancel
- New MCP tools: `schedule_post`, `list_scheduled`, `cancel_post`
- New UI: scheduling view (calendar or list), draft editor, platform selector
- External integrations: Twitter/X API, LinkedIn API, etc. (OAuth flows, token storage)
- Automation hook: Henry/conductor triggers posts at scheduled time

**Estimated effort:** 8-15 CC hours minimum, depending on number of platforms and level of automation

---

## 3. Recommended Order

**Option A — Single PR (recommended if social scheduling is deferred):**
Ship #71 (Projects Workspace UI) as one PR. Social scheduling becomes a separate epic once scoped.

**Option B — Two PRs (if both land this cycle):**
1. PR 1: Projects Workspace UI (#71) — can merge independently
2. PR 2: Social Media Scheduling — depends on PR 1 for project scoping

**Recommendation: Option A.** Social scheduling has no issue, no schema, no API integration work done, and requires fundamental design decisions. Ship the Kanban view now; file and scope social scheduling as a proper epic.

---

## 4. Open Questions for Jesse

### Q1 (Critical): Where does the Projects workspace live?
The Build tab currently has terminal + browser + Mobius. Does the Kanban board:
- (a) Replace the Build tab content when a project is selected?
- (b) Live in a new top-level tab ("Projects")?
- (c) Become a panel within the Build tab (split layout)?

### Q2 (Critical): What are the Kanban columns?
Epic #69 mentions "columns by goal state." PR #189 delivered project status (active/paused/archived/dead) but no goals/tasks schema yet (that's orchestrator work — Epic #59). Options:
- (a) Columns = project status (active | paused | archived) — cards are projects themselves
- (b) Columns = custom stages (To Do | In Progress | Done) — cards are tasks/items within a project
- (c) Placeholder columns with manual card management until orchestrator goals land

### Q3 (Important): Social media scheduling — what's the vision?
- Is this "Henry autonomously posts on my behalf on a schedule"?
- Is this "I draft posts in Permagent and schedule them like Buffer"?
- Which platforms? (X, LinkedIn, Instagram, Bluesky, etc.)
- Does this connect to a specific project, or is it cross-project?
- Is there an existing service (Buffer, Typefully, etc.) we should integrate with rather than build from scratch?

### Q4 (Minor): Card data source before orchestrator?
Until goals/tasks primitives exist (#59), what populates Kanban cards?
- (a) Hardcoded demo data for the shell
- (b) Brain memories tagged with project_id
- (c) A lightweight "project items" table as an interim

---

## 5. Dependencies and Risks

| Dependency | Status | Risk |
|------------|--------|------|
| PR #189 (projects schema) | In CI, expected to merge | Low — foundation is solid |
| Epic #70 (FK propagation) | Open | **Not blocking** — app-layer joins work |
| Epic #59 (orchestrator) | Not started | **Medium** — Kanban cards need a data source; placeholder design needed |
| Ubuntu CI (#190) | Blocking all PRs | **Known** — admin-bypass in place, macOS tests are the real signal |
| Social scheduling APIs | No work done | **High** — OAuth flows, rate limits, platform TOS, token refresh |

### Risk: Kanban without orchestrator
The Kanban view is the UI for orchestrator goals, but the orchestrator doesn't exist yet. Risk of building UI that needs significant rework when goals/tasks schema lands. Mitigation: design the board component to be data-source-agnostic (accepts cards via props, doesn't hardcode schema assumptions).

---

## 6. Summary

| Piece | Issues | Effort | Recommendation |
|-------|--------|--------|----------------|
| Projects Workspace UI | #71 (child of #69) | 3-5 CC hours | **Ship now** |
| Social Media Scheduling | None filed | 8-15 CC hours | **File issue first, scope separately** |
