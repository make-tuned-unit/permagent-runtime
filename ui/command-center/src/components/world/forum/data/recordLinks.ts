/**
 * World W1 — record links: a stable way to deep-link a World evidence item to
 * its HOME view (Brain, History, Grow, Finance, Projects, Decision Inbox, the
 * goal detail, the canonical chat).
 *
 * Callback-based by design: this module never imports the store or a router.
 * The integrator (W6) builds a `WorldNavigation` once — usually with
 * `createStoreNavigation(useCommandCenter.getState, navigateToTool)` — and
 * passes it down. Every opener returns whether the destination was reachable,
 * so a control can say "No workspace hosts Brain" instead of looking like it
 * worked (the `focusWorldAgent` / `openDecisionInbox` rule in store.ts).
 *
 * Opening a link only NAVIGATES. No link answers a decision, approves a goal,
 * or sends a message: approval stays in the Decision Inbox and goal detail.
 */

export type HistoryTab = 'sessions' | 'activity' | 'spend';

export type WorldRecordLink =
  | { to: 'brain'; memoryId: string | null; memoryKey: string | null; preview?: { text: string; timestamp: string | null } }
  | { to: 'goal'; projectId: string; goalId: string }
  | { to: 'decision_inbox'; decisionId: string | null }
  | { to: 'conversation'; sessionId: string }
  | { to: 'history'; tab: HistoryTab }
  | { to: 'grow'; projectId: string | null }
  | { to: 'finance' }
  | { to: 'project'; projectId: string }
  | { to: 'project_note'; projectId: string; noteId: string }
  | { to: 'skills' }
  | { to: 'agent_settings'; agentId: string }
  | { to: 'build' }
  /** Council reports live on Home's Council card. There is no per-session deep
   *  link in the app today, so this opens Home; the session id is kept for the key. */
  | { to: 'council'; sessionId: string };

export type WorldRecordTarget = WorldRecordLink['to'];

/** The navigation seams World needs. Each mirrors an existing store action. */
export interface WorldNavigation {
  /** store.focusBrainMemory (navigates to the 'memory' tool). */
  focusBrainMemory(target: { id?: string; key?: string; preview?: { text: string; timestamp?: string | null } }): boolean;
  /** store.openGoalDetail (opens the canonical goal modal, no navigation). */
  openGoalDetail(projectId: string, goalId: string): boolean;
  /** store.openDecisionInbox — false when no workspace hosts Home. */
  openDecisionInbox(): boolean;
  /** store.switchToSession — opens the canonical shared chat. */
  openConversation(sessionId: string): boolean;
  /** History panel on a given segment. */
  openHistory(tab: HistoryTab): boolean;
  /** store.growProject / navigateToTool('grow'). */
  openGrow(projectId: string | null): boolean;
  /** navigateToTool for a tool-only destination. */
  openTool(tool: 'finance' | 'build'): boolean;
  /** Projects view on one project (its Market panel lives there). */
  openProject(projectId: string): boolean;
  /** store.focusProjectNote. */
  focusProjectNote(projectId: string, noteId: string): boolean;
  /** Skills panel. */
  openSkills(): boolean;
  /** store.openAgentSettings. */
  openAgentSettings(agentId: string): boolean;
  /** Home (dashboard), where the Council card renders reports. */
  openHome(): boolean;
}

export type OpenResult = { ok: true } | { ok: false; reason: string };

const UNREACHABLE: Record<WorldRecordTarget, string> = {
  brain: 'No workspace currently hosts Brain. Open it from the app navigation.',
  goal: 'The goal detail could not be opened.',
  decision_inbox: 'No workspace currently hosts Home, where decisions are reviewed.',
  conversation: 'The conversation could not be opened.',
  history: 'History could not be opened.',
  grow: 'No workspace currently hosts Grow.',
  finance: 'No workspace currently hosts Finance.',
  project: 'No workspace currently hosts Projects.',
  project_note: 'No workspace currently hosts Projects.',
  skills: 'Skills could not be opened.',
  agent_settings: 'Agent settings could not be opened.',
  build: 'No workspace currently hosts Build.',
  council: 'No workspace currently hosts Home, where Council reports are shown.',
};

/** Open a record in its home view. Navigation only — never a write. */
export function openRecordLink(link: WorldRecordLink, nav: WorldNavigation): OpenResult {
  let ok: boolean;
  try {
    switch (link.to) {
      case 'brain': {
        if (!link.memoryId && !link.memoryKey) return { ok: false, reason: 'This record has no Brain id or key to open.' };
        ok = nav.focusBrainMemory({
          ...(link.memoryId ? { id: link.memoryId } : {}),
          ...(link.memoryKey ? { key: link.memoryKey } : {}),
          ...(link.preview ? { preview: link.preview } : {}),
        });
        break;
      }
      case 'goal': ok = nav.openGoalDetail(link.projectId, link.goalId); break;
      case 'decision_inbox': ok = nav.openDecisionInbox(); break;
      case 'conversation': ok = nav.openConversation(link.sessionId); break;
      case 'history': ok = nav.openHistory(link.tab); break;
      case 'grow': ok = nav.openGrow(link.projectId); break;
      case 'finance': ok = nav.openTool('finance'); break;
      case 'build': ok = nav.openTool('build'); break;
      case 'project': ok = nav.openProject(link.projectId); break;
      case 'project_note': ok = nav.focusProjectNote(link.projectId, link.noteId); break;
      case 'skills': ok = nav.openSkills(); break;
      case 'agent_settings': ok = nav.openAgentSettings(link.agentId); break;
      case 'council': ok = nav.openHome(); break;
    }
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : UNREACHABLE[link.to] };
  }
  return ok ? { ok: true } : { ok: false, reason: UNREACHABLE[link.to] };
}

/** Human label for the control that opens a link ("Open in Brain", …). */
export function recordLinkLabel(link: WorldRecordLink): string {
  switch (link.to) {
    case 'brain': return 'Open in Brain';
    case 'goal': return 'Open goal';
    case 'decision_inbox': return 'Review in Decision Inbox';
    case 'conversation': return 'Resume conversation';
    case 'history': return `Open History · ${link.tab}`;
    case 'grow': return 'Open Grow';
    case 'finance': return 'Open Finance';
    case 'project': return 'Open project';
    case 'project_note': return 'Open note';
    case 'skills': return 'Open Skills';
    case 'agent_settings': return 'Manage agent';
    case 'build': return 'Open Build';
    case 'council': return 'Open Council on Home';
  }
}

const enc = (v: string | null): string => (v === null ? '~' : encodeURIComponent(v));
const dec = (v: string | undefined): string | null => (v === undefined || v === '~' ? null : decodeURIComponent(v));

/**
 * A stable string for a link — used as a React key, as an evidence reference
 * inside drafts and field notes, and for de-duplication. Previews are NOT part
 * of the key: the key names the record, not a copy of its content.
 */
export function recordLinkKey(link: WorldRecordLink): string {
  switch (link.to) {
    case 'brain': return `brain:${enc(link.memoryId)}/${enc(link.memoryKey)}`;
    case 'goal': return `goal:${enc(link.projectId)}/${enc(link.goalId)}`;
    case 'decision_inbox': return `decision_inbox:${enc(link.decisionId)}`;
    case 'conversation': return `conversation:${enc(link.sessionId)}`;
    case 'history': return `history:${link.tab}`;
    case 'grow': return `grow:${enc(link.projectId)}`;
    case 'finance': return 'finance:';
    case 'project': return `project:${enc(link.projectId)}`;
    case 'project_note': return `project_note:${enc(link.projectId)}/${enc(link.noteId)}`;
    case 'skills': return 'skills:';
    case 'agent_settings': return `agent_settings:${enc(link.agentId)}`;
    case 'build': return 'build:';
    case 'council': return `council:${enc(link.sessionId)}`;
  }
}

/** Inverse of recordLinkKey. null for anything malformed (never a guess). */
export function parseRecordLinkKey(key: string): WorldRecordLink | null {
  const colon = key.indexOf(':');
  if (colon < 0) return null;
  const to = key.slice(0, colon);
  const parts = key.slice(colon + 1).split('/');
  const one = parts.length === 1 ? dec(parts[0]) : undefined;
  const two = parts.length === 2 ? [dec(parts[0]), dec(parts[1])] as const : null;
  try {
    switch (to) {
      case 'brain': return two && (two[0] || two[1]) ? { to, memoryId: two[0], memoryKey: two[1] } : null;
      case 'goal': return two && two[0] && two[1] ? { to, projectId: two[0], goalId: two[1] } : null;
      case 'decision_inbox': return parts.length === 1 ? { to, decisionId: one ?? null } : null;
      case 'conversation': return one ? { to, sessionId: one } : null;
      case 'history': return parts[0] === 'sessions' || parts[0] === 'activity' || parts[0] === 'spend' ? { to, tab: parts[0] } : null;
      case 'grow': return parts.length === 1 ? { to, projectId: one ?? null } : null;
      case 'finance': return parts[0] === '' ? { to } : null;
      case 'project': return one ? { to, projectId: one } : null;
      case 'project_note': return two && two[0] && two[1] ? { to, projectId: two[0], noteId: two[1] } : null;
      case 'skills': return parts[0] === '' ? { to } : null;
      case 'agent_settings': return one ? { to, agentId: one } : null;
      case 'build': return parts[0] === '' ? { to } : null;
      case 'council': return one ? { to, sessionId: one } : null;
      default: return null;
    }
  } catch {
    return null;
  }
}

/**
 * The store slice createStoreNavigation needs — declared STRUCTURALLY so this
 * module does not import store.ts (W1 may not couple to it; W6 wires it).
 * Every member exists on `useCommandCenter.getState()` today.
 */
export interface StoreNavigationSlice {
  focusBrainMemory(target: { id?: string; key?: string; preview?: { text: string; timestamp?: string | null } }): void;
  openGoalDetail(projectId: string, cardId: string): void;
  openDecisionInbox(): boolean;
  switchToSession(sessionId: string): Promise<void>;
  setActivePanel(panel: 'history' | 'skills'): void;
  setPendingSettingsSection(section: string | null): void;
  growProject(projectId: string): void;
  focusProjectNote(projectId: string, noteId: string): void;
  setPendingProjectNavigation(id: string | null): void;
  openAgentSettings(id: string | null): void;
}

/**
 * Adapter from the existing store actions to WorldNavigation. `navigateToTool`
 * is store.ts's exported function (false when no workspace hosts the tool).
 */
export function createStoreNavigation(
  getState: () => StoreNavigationSlice,
  navigateToTool: (tool: 'memory' | 'grow' | 'finance' | 'build' | 'projects' | 'dashboard') => boolean,
): WorldNavigation {
  return {
    focusBrainMemory(target) {
      // focusBrainMemory navigates internally; check reachability first so a
      // pending focus is not armed for a later, unrelated Brain visit.
      if (!navigateToTool('memory')) return false;
      getState().focusBrainMemory(target);
      return true;
    },
    openGoalDetail(projectId, goalId) { getState().openGoalDetail(projectId, goalId); return true; },
    openDecisionInbox() { return getState().openDecisionInbox(); },
    openConversation(sessionId) { void getState().switchToSession(sessionId); return true; },
    openHistory(tab) {
      const s = getState();
      s.setPendingSettingsSection(tab);
      s.setActivePanel('history');
      return true;
    },
    openGrow(projectId) {
      if (projectId === null) return navigateToTool('grow');
      if (!navigateToTool('grow')) return false;
      getState().growProject(projectId);
      return true;
    },
    openTool(tool) { return navigateToTool(tool); },
    openProject(projectId) {
      if (!navigateToTool('projects')) return false;
      getState().setPendingProjectNavigation(projectId);
      return true;
    },
    focusProjectNote(projectId, noteId) {
      if (!navigateToTool('projects')) return false;
      getState().focusProjectNote(projectId, noteId);
      return true;
    },
    openSkills() { getState().setActivePanel('skills'); return true; },
    openAgentSettings(agentId) { getState().openAgentSettings(agentId); return true; },
    openHome() { return navigateToTool('dashboard'); },
  };
}
