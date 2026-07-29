import { create } from 'zustand';
import { api, apiFetch, extractText, extractThinking, fileToBase64, readerIngest } from './api';
import { emitActivity, type ActivityEventName, type ActivitySourceSurface } from './emitActivity';
import type { SessionSummary, DaemonMessage, SSEEvent, AppContextPayload, TokenState } from './api';
import { costFromFrame } from './costMeter';
import { maybeSpeakReply, replyDedupeKey } from './speakReplies';
import { appendTraceRecord, sessionFrameToRecord } from './traceEvents';
import { startEventPruning } from './eventBus';
import type { ProjectPerson } from '../components/projects/types';
import type { BrainMemoryTarget } from '../components/brain/brainMemoryFocus';

// --- Types ---

export interface EventRecord {
  id: string;
  timestamp: string;
  source: string;
  event_type: string;
  severity: string;
  run_id: string | null;
  task_id: string | null;
  agent_id: string | null;
  correlation_id: string | null;
  payload: Record<string, unknown>;
}

export interface ToolCall {
  /** Tool-request block id, used to join the tool's response back to its call. */
  id?: string;
  name: string;
  arguments: Record<string, unknown>;
  result?: string;
  success?: boolean;
}

export interface ChatMessageImage {
  data: string;
  mimeType: string;
}

export interface ProbedMemoryRef {
  id: string;
  key: string;
  content_summary: string;
  relevance: number;
  wing: string | null;
}

export interface RecalledMemoryRef {
  id: string;
  signal_score: number;
  content_summary: string;
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  task_id?: string;
  tool_calls?: ToolCall[];
  images?: ChatMessageImage[];
  /** The model's reasoning, if it thought before answering (disclosure UI). */
  thinking?: string;
  context_attached?: {
    probed_memories: ProbedMemoryRef[];
    recalled_memories: RecalledMemoryRef[];
  };
}

export interface SessionState {
  id: string;
  name: string;
  created_at?: string;
  updated_at?: string;
  message_count: number;
}

export interface SkillState {
  id: string;
  name: string;
  description: string;
  trigger_type?: string;
  trigger_config?: Record<string, unknown>;
  steps?: Array<{ action: string; tool?: string; description?: string }>;
  usageCount?: number;
  last_run?: string;
  status?: string;
  version?: string;
  created_at?: string;
  updated_at?: string;
}

// PermagentEvent types from Section C.3
export type PermagentEventType =
  | 'daemon_started' | 'daemon_stopped'
  | 'task_created' | 'task_started' | 'task_completed' | 'task_failed'
  | 'memory_added' | 'memory_recalled'
  | 'entity_added' | 'entity_updated'
  | 'decision_created' | 'decision_resolved'
  | 'agent_state_changed' | 'goal_state_changed'
  | 'browser_content_requested' | 'browser_navigate_requested'
  | 'skill_proposed' | 'skill_saved' | 'skill_triggered'
  | 'message_received' | 'stream_chunk'
  | 'integration_connected' | 'integration_error'
  | 'librarian_describe_started' | 'librarian_describe_token' | 'librarian_describe_retry' | 'librarian_describe_completed';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

export type ActivePanel = 'chat' | 'skills' | 'events' | 'settings' | 'sessions' | 'terminal' | 'browser' | 'inbox' | 'trace' | 'governance';

// ── Workspace types ──

export type ToolType = 'chat' | 'skills' | 'trace' | 'world' | 'terminal' | 'browser' | 'memory' | 'dashboard' | 'build' | 'grow' | 'automate' | 'projects';

export interface LayoutSplit {
  type: 'split';
  direction: 'horizontal' | 'vertical';
  sizes: number[];
  children: LayoutNode[];
}

export interface LayoutPanel {
  type: 'panel';
  tool: ToolType;
  config: Record<string, unknown>;
}

export type LayoutNode = LayoutSplit | LayoutPanel;

export interface WorkspaceState {
  id: string;
  name: string;
  icon: string;
  sortOrder: number;
  layoutJson: LayoutNode;
  isDefault: boolean;
}

// Skill proposal from skill_proposed events
export interface SkillProposal {
  description: string;
  tool_used: string;
  argument_shape_hash: string;
  occurrence_count: number;
  source_task_ids: string[];
  timestamp: string;
}

export interface ProviderInfo {
  name: string;
  displayName: string;
  description: string;
  defaultModel: string;
  knownModels: string[];
  configKeys: Array<{ name: string; required: boolean; secret: boolean; description?: string }>;
  isConfigured: boolean;
  isDefault: boolean;
  /** Registry classification: "Preferred" | "Builtin" | "Declarative" | "Custom".
   *  "Custom" marks a user-defined provider (added via the custom-provider flow),
   *  which is the only type that can be removed from the UI. */
  providerType: string;
}

interface CommandCenterStore {
  // --- Panel routing ---
  activePanel: ActivePanel;
  setActivePanel: (panel: ActivePanel) => void;

  // --- Workspace state ---
  workspaces: WorkspaceState[];
  activeWorkspaceId: string | null;
  workspacesLoaded: boolean;
  loadWorkspaces: () => Promise<void>;
  /**
   * #629 multi-client liveness: refetch the workspace LIST (layouts, names)
   * without touching this client's `activeWorkspaceId` — a remote layout edit
   * must update our arrangement, never yank which workspace we're looking at.
   * Falls back to the first workspace only if the active one disappeared.
   */
  refreshWorkspaces: () => Promise<void>;
  switchWorkspace: (workspaceId: string) => void;
  updateWorkspaceLayout: (workspaceId: string, layoutJson: LayoutNode) => void;
  /** Set when persisting workspace state (a layout resize, or which workspace
   *  is active) fails. The local UI keeps the optimistic value, so without
   *  this latch the app silently lies that the arrangement was saved (the old
   *  `.catch(() => {})`). Cleared by a later successful save of the same kind
   *  or an explicit retry. Rendered by WorkspaceSaveErrorChip (the Dashboard
   *  SaveIndicator pattern, plus a Retry path). */
  workspaceSaveFailure: { kind: 'layout' | 'active'; workspaceId: string; message: string } | null;
  /** Re-attempt the failed workspace persistence using the CURRENT state
   *  (freshest layout / currently-active workspace, not a stale snapshot). */
  retryWorkspaceSave: () => Promise<void>;
  dismissWorkspaceSaveFailure: () => void;

  // --- Connection state ---
  connectionStatus: ConnectionStatus;

  // --- Provider state ---
  providers: ProviderInfo[];
  /** True when the last loadProviders() failed — lets the UI show a retry
      state instead of an indistinguishable, permanent "Loading…". */
  providersError: boolean;
  currentModel: string | null;
  loadProviders: () => Promise<void>;
  setDefaultProvider: (name: string, model: string) => Promise<void>;

  // --- Operational state ---
  events: EventRecord[];

  // --- Agent identity ---
  agentName: string;
  setAgentName: (name: string) => void;

  // --- Chat state ---
  chatMessages: ChatMessage[];
  chatSessionId: string | null;
  /** Non-null when the last loadSessionMessages failed transiently (daemon
   *  hiccup, network) — the #568-lesson surface: MessageList renders it inline
   *  with a Retry instead of the old silent catch that disowned the session.
   *  Null while loading and after a successful load. */
  sessionLoadError: string | null;
  addChatMessage: (msg: ChatMessage) => void;
  _streamingMessageId: string | null;
  /** request_id of the in-flight reply turn (client-generated in api.sendReply,
   *  re-adopted from the daemon's ActiveRequests SSE event on reconnect). The
   *  Stop button's cancel target; null when idle. */
  _activeRequestId: string | null;
  _pendingContext: { probed_memories: ProbedMemoryRef[]; recalled_memories: RecalledMemoryRef[] } | null;

  // --- SSE streaming ---
  isStreaming: boolean;
  /**
   * Latest token + cost state from the SSE stream — updated on every Message /
   * Finish frame (each carries `token_state`). The always-on Build meter reads
   * this; it is the live, single-sourced $ with no extra endpoint.
   */
  liveTokens: TokenState | null;
  sendMessage: (text: string, files?: File[]) => Promise<void>;
  /** Interrupt the in-flight turn: POST /sessions/{id}/cancel with the active
   *  request_id. Returns true when the daemon confirmed a live request was
   *  cancelled (a terminal Finish follows on SSE and settles the UI); false
   *  when there was nothing to cancel — locally idle, the request_id hasn't
   *  landed yet, or the daemon answered {cancelled:false} (stale id: the turn
   *  already ended or the daemon restarted), in which case streaming state is
   *  reconciled to idle here because no terminal frame will ever arrive.
   *  Throws if the cancel POST itself fails (agent still alive). */
  stopStreaming: () => Promise<boolean>;
  /**
   * Decision Inbox deep-link (#303): open a fresh chat session seeded with a
   * decision's context and a context-aware opening turn. Set transiently while
   * the seed turn is sent so buildAppContext can carry the id to the daemon.
   */
  discussSeedDecisionId: string | null;
  discussDecision: (decisionId: string, headline: string) => Promise<void>;

  /**
   * Goal-detail modal (#503): the single detail view every goal surface — Kanban
   * card, Decision Inbox row, dashboard "in flight" item — opens. Set transiently
   * to a {projectId, cardId} target; a host mounted at the app root renders the
   * modal whenever it is non-null. Mirrors the discussDecision deep-link seam.
   */
  goalDetail: { projectId: string; cardId: string } | null;
  openGoalDetail: (projectId: string, cardId: string) => void;
  closeGoalDetail: () => void;
  /**
   * Person-detail modal (CRM epic slice 2): the read-only person view opened
   * from a project's People panel. Carries the full {@link ProjectPerson} from
   * the list response so the modal needs no extra fetch. Host mounted at the app
   * root; mirrors the goalDetail seam above.
   */
  personDetail: { projectId: string; person: ProjectPerson } | null;
  openPersonDetail: (projectId: string, person: ProjectPerson) => void;
  closePersonDetail: () => void;
  /**
   * Monotonic revision the People panel re-fetches on. Bumped after a mutation
   * (associate / disassociate) so the store-hosted person modal can refresh the
   * decoupled panel — there is no people event stream yet.
   */
  peopleRev: number;
  bumpPeople: () => void;
  /**
   * #629 multi-client liveness: monotonic revision bumped when a
   * `project_changed` event arrives on /events — the projects list and the
   * per-project Documents/Memories/Notes panels refetch on it, so a write from
   * a second device pushes here instead of waiting for a poll (or forever).
   */
  projectsRev: number;
  bumpProjects: () => void;
  /**
   * #629: bumped when `identity_changed` arrives — identity consumers
   * (chat header, world nameplate, settings persona) re-read
   * /api/agent/identity. `refreshIdentity` also updates `agentName` directly.
   */
  identityRev: number;
  refreshIdentity: () => Promise<void>;
  switchToSession: (sessionId: string) => Promise<void>;
  deleteSession: (sessionId: string) => Promise<void>;
  renameSession: (sessionId: string, name: string) => Promise<void>;

  // --- Skills state ---
  skills: SkillState[];
  skillsLoading: boolean;
  selectedSkillId: string | null;
  setSelectedSkillId: (id: string | null) => void;
  loadSkills: () => Promise<void>;
  deleteSkill: (id: string) => Promise<void>;
  updateSkill: (id: string, updates: Partial<SkillState>) => Promise<boolean>;

  // --- Skill proposals ---
  pendingSkillProposal: SkillProposal | null;
  proposals: SkillProposal[];
  saveSkillProposal: () => Promise<void>;
  dismissSkillProposal: () => void;
  loadProposals: () => Promise<void>;
  saveProposal: (proposal: SkillProposal) => Promise<void>;
  dismissProposal: (argumentShapeHash: string) => void;

  // --- Sessions state ---
  sessions: SessionState[];
  /** True when the last loadSessions() failed — lets SessionsList show an
      inline error + retry instead of the lying "No sessions yet" empty state
      (#568 empty-body lesson; mirrors providersError). */
  sessionsError: boolean;
  loadSessions: () => Promise<void>;

  // --- Event filters ---
  eventTypeFilter: string;
  setEventTypeFilter: (type: string) => void;

  // --- Actions ---
  loadEvents: (params?: { type?: string; limit?: number }) => Promise<void>;
  loadSessionMessages: (sessionId: string) => Promise<void>;
  handleSessionEvent: (data: SSEEvent) => void;
  clearEvents: () => void;

  // --- Project navigation (from agent/voice) ---
  pendingProjectNavigation: string | null;
  setPendingProjectNavigation: (id: string | null) => void;

  // --- Brain-loop: "View in Brain" deep-link (surface a specific memory) ---
  // Product surfaces that write into the Brain (Projects Memories panel, Notes,
  // Codebase index) close the loop by focusing the memory they created back in
  // the Brain view. focusBrainMemory stashes the target + navigates to the Brain
  // (the 'memory' tool); BrainView consumes it (graph-preferred, preview
  // fallback) and opens that memory's side panel. Reusable by any future caller
  // that has a memory id/key (e.g. the operator last-mile).
  pendingBrainMemory: BrainMemoryTarget | null;
  focusBrainMemory: (target: BrainMemoryTarget) => void;
  clearPendingBrainMemory: () => void;

  // --- Settings deep-link (from agent/voice: "Settings → <pane>") ---
  pendingSettingsSection: string | null;
  setPendingSettingsSection: (section: string | null) => void;

  // --- Project terminal launch (from agent: project_launch event) ---
  pendingTerminalLaunch: { rootPath: string; label: string; command?: string; supervisedSessionId?: string } | null;
  setPendingTerminalLaunch: (
    launch: {
      rootPath: string;
      label: string;
      command?: string;
      supervisedSessionId?: string;
    } | null,
  ) => void;

  // --- In-app browser navigation (chat links, agent tour #353) ---
  pendingBrowserUrl: string | null;
  // Grow deep-link (2026-07-11): Projects → 'Grow this project' sets this, the
  // Grow tab reads it to preselect the project. GrowView consumes-then-clears
  // via setOpenGrowForProject(null) (the pendingProjectNavigation pattern) so
  // a one-shot deep link can't re-select that project on every later Grow visit.
  openGrowForProject: string | null;
  growProject: (projectId: string) => void;
  setOpenGrowForProject: (id: string | null) => void;
  openInBrowser: (url: string) => void;
  clearPendingBrowserUrl: () => void;

  // --- Build tab pane visibility (#567-adjacent UX): hide either pane so
  // the other gets the full canvas. Persisted in-session only.
  buildTerminalHidden: boolean;
  buildBrowserHidden: boolean;
  toggleBuildTerminal: () => void;
  toggleBuildBrowser: () => void;

  // --- Browser overlay z-order ---
  overlayBlockingBrowser: number;
  pushBrowserOverlay: () => void;
  popBrowserOverlay: () => void;

  // --- Collapsed chat launcher corner reservation (#553) ---
  // Measured size of the collapsed ChatLauncher pill (null when absent, i.e.
  // the chat window is open). The Browser subtracts this corner from the
  // native webview bounds — CSS z-index cannot cover a native child surface.
  chatLauncherSize: { width: number; height: number } | null;
  setChatLauncherSize: (size: { width: number; height: number } | null) => void;

  // --- Chat dock (2026-07-11): chat opens as a right sidebar first, detaches
  // to a window on demand (validated UX pattern). Mutually exclusive with the
  // detached window; the Browser reserves its strip like the launcher pill. ---
  chatDockOpen: boolean;
  openChatDock: () => void;
  closeChatDock: () => void;

  // --- Detached chat window liveness ---
  // True while the standalone `chat` WebviewWindow exists. Set by
  // createChatWindow (the single creation path — launcher, dock-detach,
  // App drop handler, navigate) and cleared by the launcher's close listener,
  // so the "Chat with Henry" pill hides no matter which path opened the window.
  chatWindowOpen: boolean;
  setChatWindowOpen: (open: boolean) => void;

  // --- Voice conversation mode (hands-free #19) ---
  // Published by VoiceHost while hands-free is active so ChatView (or the
  // App-level fallback) can render the full-window orb takeover. Analyser
  // getters are live taps on the TTS playback / mic audio graphs; `exit`
  // leaves hands-free.
  voiceConversation: {
    state: string;
    getPlaybackAnalyser: () => AnalyserNode | null;
    getMicAnalyser: () => AnalyserNode | null;
    exit: () => void;
  } | null;
  setVoiceConversation: (conv: CommandCenterStore['voiceConversation']) => void;

  // --- Voice engine (per-window singleton) ---
  // Hosted by VoiceHost at the WINDOW root, not inside ChatView: closing the
  // dock, detaching to a window, or switching views must never tear down the
  // mic/socket mid-conversation. VoiceButton and the orb are pure views over
  // this. Null until the host mounts.
  voiceEngine: {
    state: string;
    error: string | null;
    handsFree: boolean;
    activate: () => void | Promise<void>;
    deactivate: () => void;
    startRecording: () => void;
    stopRecording: () => void;
    interrupt: () => void;
    getAnalyser: () => AnalyserNode | null;
    getMicAnalyser: () => AnalyserNode | null;
    setHandsFree: (on: boolean) => void | Promise<void>;
  } | null;
  setVoiceEngine: (engine: CommandCenterStore['voiceEngine']) => void;

  // --- Per-session SSE ---
  _eventSource: EventSource | null;
  _reconnectTimer: ReturnType<typeof setTimeout> | null;
  _reconnectAttempts: number;
  /** SSE cursor: the last `id:` seen on the stream. Sent back to the daemon as
   *  `?last_event_id=` on reconnect so the replay resumes instead of repeating
   *  the whole buffer (duplicate deltas/error bubbles — P1). */
  _lastEventId: string | null;
  /** Which session `_lastEventId` belongs to — sequence numbers are per-session,
   *  so a cursor must never leak across a session switch. */
  _lastEventSessionId: string | null;
  /** Async: awaits the daemon token before opening the SSE (C1/C2 auth). */
  connectSession: (sessionId: string) => Promise<void>;
  disconnectSession: () => void;
  ensureSession: () => Promise<string | null>;
}

const MAX_EVENTS_BUFFER = 1000;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

/** Guards the async gap in connectSession (awaiting the daemon token before
 *  constructing the EventSource): a newer connect/disconnect bumps the epoch,
 *  and a stale in-flight connect aborts instead of opening a duplicate SSE. */
let _sseConnectEpoch = 0;
/** Speak-replies stays silent until this instant — set past each SSE connect
 *  so the replay burst never re-voices history. */
let _speakSuppressUntil = 0;

/** Pull a toolRequest's name/args, tolerating the daemon's tool_result_serde
 *  wrapper `{ status, value:{ name, arguments } }` as well as a flat shape.
 *  (The old code read `.name` off the wrapper, so every tool showed "unknown".) */
function readToolCallInner(toolCall: unknown): { name: string; arguments: Record<string, unknown> } {
  const tc = toolCall as
    | { name?: string; arguments?: Record<string, unknown>; value?: { name?: string; arguments?: Record<string, unknown> } }
    | undefined;
  const inner = tc && typeof tc === 'object' && 'value' in tc && tc.value ? tc.value : tc;
  return { name: inner?.name || 'unknown', arguments: inner?.arguments || {} };
}

/** Turn a toolResponse's `toolResult` into a display string + status.
 *  Shape (tool_result_serde::call_tool_result):
 *    { status:'success', value:{ content:[{type:'text',text}] } | Content[] }
 *    { status:'error',   error: string } */
function readToolResult(toolResult: unknown): { result: string; success: boolean } {
  const tr = toolResult as { status?: string; error?: unknown; value?: unknown } | undefined;
  if (!tr || typeof tr !== 'object') return { result: '', success: true };
  if (tr.status === 'error') {
    return { result: typeof tr.error === 'string' ? tr.error : JSON.stringify(tr.error), success: false };
  }
  const value = tr.value;
  const content = Array.isArray(value) ? value : (value as { content?: unknown } | undefined)?.content;
  if (Array.isArray(content)) {
    const text = content
      .filter((x): x is { type: string; text: string } =>
        !!x && (x as { type?: string }).type === 'text' && typeof (x as { text?: unknown }).text === 'string')
      .map(x => x.text)
      .join('\n');
    return { result: text || JSON.stringify(value), success: true };
  }
  return { result: typeof value === 'string' ? value : JSON.stringify(value ?? ''), success: true };
}

/** Index every toolResponse block in a conversation by its request id, so each
 *  toolRequest can be joined to the result it produced (they arrive in
 *  different messages). */
function indexToolResponses(messages: DaemonMessage[]): Map<string, { result: string; success: boolean }> {
  const map = new Map<string, { result: string; success: boolean }>();
  for (const m of messages) {
    for (const c of m.content) {
      if (c.type === 'toolResponse') {
        const tr = c as { type: string; id: string; toolResult?: unknown };
        if (tr.id) map.set(tr.id, readToolResult(tr.toolResult));
      }
    }
  }
  return map;
}

/** Convert a DaemonMessage to a ChatMessage for display. `responses` (built via
 *  indexToolResponses over the whole conversation) lights up each tool card
 *  with its result + success. */
function daemonMsgToChat(
  msg: DaemonMessage,
  index: number,
  sessionId: string,
  responses?: Map<string, { result: string; success: boolean }>,
): ChatMessage {
  const toolCalls: ToolCall[] = [];
  const images: ChatMessageImage[] = [];
  for (const c of msg.content) {
    if (c.type === 'toolRequest') {
      const tr = c as { type: string; id: string; toolCall?: unknown };
      const { name, arguments: args } = readToolCallInner(tr.toolCall);
      const resp = tr.id ? responses?.get(tr.id) : undefined;
      toolCalls.push({ id: tr.id, name, arguments: args, result: resp?.result, success: resp?.success });
    } else if (c.type === 'image') {
      const img = c as { type: string; data: string; mimeType: string };
      if (img.data && img.mimeType) {
        images.push({ data: img.data, mimeType: img.mimeType });
      }
    }
  }

  const thinking = extractThinking(msg);
  return {
    id: msg.id || `hist-${sessionId}-${index}`,
    role: msg.role,
    content: extractText(msg),
    timestamp: new Date(msg.created * 1000).toISOString(),
    tool_calls: toolCalls.length > 0 ? toolCalls : undefined,
    images: images.length > 0 ? images : undefined,
    thinking: thinking || undefined,
  };
}

/**
 * Surfaces that emit no activity of their own — a `<tool>_opened` engagement
 * signal is emitted when the user navigates to the workspace hosting them, so
 * the onboarding coach knows they've used it. Keyed by the workspace's primary
 * tool (`brain` is the `memory` tool). Tools already instrumented by their own
 * events (build/terminal/browser/projects/etc.) are intentionally absent.
 */
const OPEN_EVENT_BY_TOOL: Partial<Record<ToolType, { event: ActivityEventName; surface: ActivitySourceSurface }>> = {
  world: { event: 'world_view_opened', surface: 'world' },
  memory: { event: 'brain_opened', surface: 'brain' },
  grow: { event: 'grow_opened', surface: 'grow' },
};

/** Extract the primary tool type from a workspace layout tree. */
function primaryToolType(node: LayoutNode): ToolType | null {
  if (node.type === 'panel') return node.tool;
  if (node.type === 'split' && node.children.length > 0) return primaryToolType(node.children[0]);
  return null;
}

/** Deep-search a layout tree for a panel hosting the given tool. */
function layoutHasTool(node: LayoutNode, tool: ToolType): boolean {
  if (node.type === 'panel') return node.tool === tool;
  if (node.type === 'split') return node.children.some(c => layoutHasTool(c, tool));
  return false;
}

/**
 * Switch the main window to the workspace hosting `tool`, closing any overlay.
 * Shared by in-window callers (Settings) and the chat window's cross-window
 * app_navigate handler. Returns false if no workspace hosts the tool.
 */
export function navigateToTool(tool: ToolType): boolean {
  const { workspaces, switchWorkspace, setActivePanel } = useCommandCenter.getState();
  const ws = workspaces.find(w => layoutHasTool(w.layoutJson, tool));
  if (!ws) return false;
  setActivePanel('chat');
  switchWorkspace(ws.id);
  return true;
}

/** Build AppContextPayload from current store state. */
function buildAppContext(state: CommandCenterStore): AppContextPayload | undefined {
  const ws = state.workspaces.find(w => w.id === state.activeWorkspaceId);
  const toolType = ws ? primaryToolType(ws.layoutJson) : null;
  if (!toolType) return undefined;
  const ctx: AppContextPayload = { current_tab: toolType };
  if (state.activePanel !== 'chat') {
    ctx.active_panel = state.activePanel;
  }
  // Decision Inbox deep-link (#303): when a "Discuss with {persona}" click seeded
  // this session, ride the decision id out so the daemon loads + injects its full
  // context on the opening turn. Set only for the seed turn, then cleared.
  if (state.discussSeedDecisionId) {
    ctx.view_state = { discuss_decision_id: state.discussSeedDecisionId };
  }
  return ctx;
}

export const useCommandCenter = create<CommandCenterStore>((set, get) => ({
  // Panel routing
  activePanel: 'chat',
  setActivePanel: (panel) => set({ activePanel: panel }),

  // Agent identity
  agentName: 'Agent',
  setAgentName: (name) => set({ agentName: name }),

  // Workspaces
  workspaces: [],
  activeWorkspaceId: null,
  workspacesLoaded: false,

  loadWorkspaces: async () => {
    try {
      const [workspaces, active] = await Promise.all([
        api.getWorkspaces(),
        api.getActiveWorkspace(),
      ]);
      set({
        workspaces: workspaces.map(w => ({
          id: w.id,
          name: w.name,
          icon: w.icon,
          sortOrder: w.sortOrder,
          layoutJson: w.layoutJson as LayoutNode,
          isDefault: w.isDefault,
        })),
        activeWorkspaceId: active.workspaceId || (workspaces.length > 0 ? workspaces[0].id : null),
        workspacesLoaded: true,
      });
    } catch {
      set({ workspacesLoaded: true });
    }
  },

  refreshWorkspaces: async () => {
    try {
      const workspaces = await api.getWorkspaces();
      set(s => {
        const mapped = workspaces.map(w => ({
          id: w.id,
          name: w.name,
          icon: w.icon,
          sortOrder: w.sortOrder,
          layoutJson: w.layoutJson as LayoutNode,
          isDefault: w.isDefault,
        }));
        const activeStillExists = mapped.some(w => w.id === s.activeWorkspaceId);
        return {
          workspaces: mapped,
          activeWorkspaceId: activeStillExists
            ? s.activeWorkspaceId
            : (mapped.length > 0 ? mapped[0].id : null),
        };
      });
    } catch {
      // Transient refetch failure: keep the current (possibly stale) layouts —
      // never blank a working screen over a liveness refresh.
    }
  },

  switchWorkspace: (workspaceId: string) => {
    // Re-selecting the already-active workspace (a re-click, or a daemon-driven
    // AppNavigate to the tab the user is on) is a no-op — in particular it must
    // not re-emit an "opened" engagement event for a view that never closed.
    if (get().activeWorkspaceId === workspaceId) return;
    set({ activeWorkspaceId: workspaceId });
    // The switch is applied optimistically (the tab must feel instant); the
    // persistence result is surfaced honestly instead of the old silent catch.
    api.setActiveWorkspace(workspaceId)
      .then(() => {
        // A later success supersedes a stale unsaved-active failure.
        if (get().workspaceSaveFailure?.kind === 'active') {
          set({ workspaceSaveFailure: null });
        }
      })
      .catch((err: unknown) => {
        console.error('Failed to persist active workspace:', err);
        set({
          workspaceSaveFailure: {
            kind: 'active',
            workspaceId,
            message: err instanceof Error ? err.message : String(err),
          },
        });
      });
    // Report engagement for surfaces that emit nothing themselves. Boot sets
    // activeWorkspaceId directly (not via this action), so this only fires on a
    // real user/agent navigation, never on initial load.
    const ws = get().workspaces.find(w => w.id === workspaceId);
    const tool = ws ? primaryToolType(ws.layoutJson) : null;
    const open = tool ? OPEN_EVENT_BY_TOOL[tool] : undefined;
    if (open) emitActivity(open.event, open.surface);
  },

  updateWorkspaceLayout: (workspaceId: string, layoutJson: LayoutNode) => {
    set(s => ({
      workspaces: s.workspaces.map(w =>
        w.id === workspaceId ? { ...w, layoutJson } : w
      ),
    }));
    // Optimistic local update above; a failed save must not pass for a saved
    // layout (the old `.catch(() => {})` swallowed 5xx AND the raw fetch never
    // even rejected on one). Failure latches workspaceSaveFailure → retry chip.
    api.updateWorkspaceLayout(workspaceId, layoutJson)
      .then(() => {
        const f = get().workspaceSaveFailure;
        if (f?.kind === 'layout' && f.workspaceId === workspaceId) {
          set({ workspaceSaveFailure: null });
        }
      })
      .catch((err: unknown) => {
        console.error('Failed to persist workspace layout:', err);
        set({
          workspaceSaveFailure: {
            kind: 'layout',
            workspaceId,
            message: err instanceof Error ? err.message : String(err),
          },
        });
      });
  },

  workspaceSaveFailure: null,

  retryWorkspaceSave: async () => {
    const failure = get().workspaceSaveFailure;
    if (!failure) return;
    try {
      if (failure.kind === 'layout') {
        const ws = get().workspaces.find(w => w.id === failure.workspaceId);
        if (ws) await api.updateWorkspaceLayout(failure.workspaceId, ws.layoutJson);
      } else {
        // Persist whichever workspace is active NOW — the user may have
        // switched again since the failed save; current truth wins.
        const active = get().activeWorkspaceId;
        if (active) await api.setActiveWorkspace(active);
      }
      set({ workspaceSaveFailure: null });
    } catch (err) {
      set({
        workspaceSaveFailure: {
          ...failure,
          message: err instanceof Error ? err.message : String(err),
        },
      });
    }
  },

  dismissWorkspaceSaveFailure: () => set({ workspaceSaveFailure: null }),

  // Connection
  connectionStatus: 'disconnected',

  // Providers
  providers: [],
  providersError: false,
  currentModel: null,
  loadProviders: async () => {
    try {
      const configResp = await api.getConfig();
      const cfgMap = ((configResp as Record<string, unknown>)['config'] ?? configResp) as Record<string, unknown>;
      const currentModel = cfgMap['GOOSE_MODEL'] as string | undefined;

      const raw = await api.getProviders();
      set({
        providersError: false,
        currentModel: currentModel || null,
        providers: raw.map(p => ({
          name: p.name,
          displayName: p.metadata.display_name,
          description: p.metadata.description,
          defaultModel: p.metadata.default_model,
          knownModels: p.metadata.known_models.map(m => m.name),
          configKeys: p.metadata.config_keys,
          isConfigured: p.is_configured,
          isDefault: p.is_default ?? false,
          providerType: p.provider_type,
        })),
      });
    } catch {
      set({ providers: [], providersError: true });
    }
  },

  setDefaultProvider: async (name: string, model: string) => {
    try {
      await api.setProvider(name, model);
      set({ currentModel: model });
      await get().loadProviders();
    } catch (e) {
      console.error('Failed to set default provider:', e);
    }
  },

  // State
  events: [],

  // Chat
  chatMessages: [],
  chatSessionId: (() => {
    try { return localStorage.getItem('permagent-chat-session-id'); } catch { return null; }
  })(),
  sessionLoadError: null,
  _streamingMessageId: null,
  _pendingContext: null,
  discussSeedDecisionId: null,

  goalDetail: null,
  openGoalDetail: (projectId, cardId) => set({ goalDetail: { projectId, cardId } }),
  closeGoalDetail: () => set({ goalDetail: null }),
  personDetail: null,
  openPersonDetail: (projectId, person) => set({ personDetail: { projectId, person } }),
  closePersonDetail: () => set({ personDetail: null }),
  peopleRev: 0,
  bumpPeople: () => set(s => ({ peopleRev: s.peopleRev + 1 })),
  projectsRev: 0,
  bumpProjects: () => set(s => ({ projectsRev: s.projectsRev + 1 })),
  identityRev: 0,
  refreshIdentity: async () => {
    try {
      const id = await api.getIdentity();
      set(s => ({ agentName: id.first_name, identityRev: s.identityRev + 1 }));
    } catch {
      // Identity unreachable — keep the current name; consumers stay as-is.
    }
  },

  addChatMessage: (msg) => set(s => ({ chatMessages: [...s.chatMessages, msg] })),

  // Streaming
  isStreaming: false,
  liveTokens: null,
  _activeRequestId: null,

  /**
   * Ensure a session exists. Creates one via POST /api/sessions if needed.
   * Returns the session ID or null on failure.
   */
  ensureSession: async () => {
    let sessionId = get().chatSessionId;
    if (sessionId) return sessionId;

    try {
      const session = await api.createSession();
      sessionId = session.id;
      set({ chatSessionId: sessionId });
      try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* */ }
      get().connectSession(sessionId);
      return sessionId;
    } catch (err) {
      console.error('Failed to create session:', err);
      return null;
    }
  },

  /**
   * Send a message via POST /sessions/{id}/reply (fire-and-forget).
   * Events arrive on the per-session SSE channel and update chat state.
   */
  sendMessage: async (text: string, files?: File[]) => {
    console.log('[send] entry — files:', files?.length ?? 0, 'text length:', text.length);
    const state = get();
    if (state.isStreaming) return;

    const sessionId = await get().ensureSession();
    if (!sessionId) {
      set(s => ({
        chatMessages: [...s.chatMessages, {
          id: `msg-${Date.now()}-err`,
          role: 'system' as const,
          content: 'Failed to create session',
          timestamp: new Date().toISOString(),
        }],
      }));
      return;
    }

    // Route EVERY dropped file through the local Reader (#296) BEFORE it enters
    // the message. The Reader extracts text locally (Vision OCR for images, PDF
    // text layer / UTF-8 for documents) and ingests it into the Brain; Henry
    // receives a compact digest, NOT the raw bytes — the token-leak fix. Only
    // "visual" images (little/no text, e.g. a photo) fall through to base64 so
    // the agent can still SEE them. Documents previously died silently on drop.
    let images: Array<{ data: string; mime_type: string }> | undefined;
    const digests: Array<{ name: string; summary: string; recall_query: string }> = [];
    if (files && files.length > 0) {
      console.log('[send] total files:', files.length,
        'types:', files.map(f => `${f.name}(type="${f.type}")`));
      const visualImages: File[] = [];
      for (const f of files) {
        const isImage = f.type.startsWith('image/');
        try {
          const d = await readerIngest(f);
          if (isImage && d.is_visual) {
            // Sparse/low-confidence text → the agent needs to see the image.
            console.log('[reader] visual image, falling through to vision:', f.name);
            visualImages.push(f);
          } else {
            // Extracted + ingested into the Brain. Digest only — bytes never sent.
            console.log('[reader] ingested', f.name, '→', d.token_count, 'tok kept out of context');
            digests.push({ name: f.name, summary: d.summary, recall_query: d.recall_query });
          }
        } catch (err) {
          if (isImage) {
            // Fail-open: never lose the user's image if the Reader is unavailable.
            console.error('[reader] image ingest failed, falling back to image:', f.name, err);
            visualImages.push(f);
          } else {
            // A document the Reader couldn't read — surface it rather than drop
            // it silently (the old behavior). No base64 path for documents.
            // The daemon's error body carries an honest reason (#468, e.g.
            // "couldn't read this PDF cleanly — … font-encoding issue");
            // show it so neither the user nor the agent mistakes a failed
            // extraction for readable content.
            console.error('[reader] document ingest failed:', f.name, err);
            const reason =
              err instanceof Error && err.message && !err.message.startsWith('reader ingest HTTP')
                ? err.message
                : 'could not extract text from this file';
            digests.push({ name: f.name, summary: `(extraction failed: ${reason})`, recall_query: '' });
          }
        }
      }
      if (visualImages.length > 0) {
        try {
          images = await Promise.all(
            visualImages.map(async f => ({
              data: await fileToBase64(f),
              mime_type: f.type || 'image/png',
            })),
          );
        } catch (err) {
          console.error('[send] fileToBase64 FAILED:', err);
          // The message still sends, but WITHOUT these images — say so in the
          // transcript instead of silently dropping files the user watched
          // themselves attach.
          const n = visualImages.length;
          set(s => ({
            chatMessages: [...s.chatMessages, {
              id: `msg-${Date.now()}-imgerr`,
              role: 'system' as const,
              content: `Couldn't read ${n} attached image${n === 1 ? '' : 's'} — sending the message without ${n === 1 ? 'it' : 'them'}.`,
              timestamp: new Date().toISOString(),
            }],
          }));
        }
      }
    }

    // Fold any Reader digests into the outgoing text so Henry sees the summary
    // + recall handle (and can follow up via search_memory), never the bytes.
    let outgoingText = text;
    if (digests.length > 0) {
      const block = digests
        .map(d => d.recall_query
          ? `📎 ${d.name} — ${d.summary} (recall: "${d.recall_query}")`
          : `📎 ${d.name} — ${d.summary}`)
        .join('\n');
      // Replace the bare "(file upload)" placeholder ChatInput sends for
      // file-only messages; otherwise append.
      outgoingText = !text || text === '(file upload)' ? block : `${text}\n\n${block}`;
    }

    // Add user message to chat — includes inline images for rendering in the bubble
    const userMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      role: 'user',
      content: outgoingText,
      timestamp: new Date().toISOString(),
      images: images?.map(img => ({ data: img.data, mimeType: img.mime_type })),
    };
    set(s => ({ chatMessages: [...s.chatMessages, userMsg] }));

    console.log('[send] before api.sendReply — text length:', outgoingText.length, 'images count:', images?.length ?? 0, 'digests:', digests.length);

    // Create streaming placeholder
    const streamMsgId = `msg-${Date.now()}-stream`;
    set(s => ({
      isStreaming: true,
      _streamingMessageId: streamMsgId,
      chatMessages: [...s.chatMessages, {
        id: streamMsgId,
        role: 'assistant' as const,
        content: '',
        timestamp: new Date().toISOString(),
      }],
    }));

    // Build app_context from current UI state
    const appContext = buildAppContext(get());

    try {
      // Fire-and-forget: the turn streams on the SSE channel. Capture the
      // request_id it returns so the Stop button can cancel THIS turn.
      const { request_id } = await api.sendReply(sessionId, outgoingText, images, appContext);
      set({ _activeRequestId: request_id });
    } catch (err) {
      console.error('[send] api.sendReply FAILED:', err);
      set(s => ({
        isStreaming: false,
        _streamingMessageId: null,
        _activeRequestId: null,
        // Drop the empty assistant placeholder — nothing will ever stream into
        // it, and leaving it renders a blank agent bubble above the failure.
        chatMessages: [...s.chatMessages.filter(m => m.id !== streamMsgId), {
          id: `msg-${Date.now()}-err`,
          role: 'system' as const,
          content: `Failed: ${err instanceof Error ? err.message : 'Unknown error'}`,
          timestamp: new Date().toISOString(),
        }],
      }));
    }
  },

  /**
   * Interrupt the in-flight turn. POSTs /sessions/{id}/cancel with the active
   * request_id and acts on the daemon's HONEST answer:
   *
   * {cancelled:true} — a live request's token was cancelled; the daemon
   * publishes a terminal Finish { reason: "stop" }, so the UI settles on the
   * normal stream path (the Finish handler flips isStreaming off + rehydrates
   * the transcript). That same Finish is when the daemon frees the session's
   * single request slot, so we deliberately keep isStreaming true until it
   * lands — resetting early would let the next send race the slot (400
   * "already has an active request").
   *
   * {cancelled:false} — the daemon knows nothing about that request_id (the
   * turn already ended, or the daemon restarted and the request evaporated).
   * NO terminal frame is ever coming for it, so waiting would wedge the
   * composer + spin the Stop button forever — reconcile to idle right here.
   *
   * If the POST itself throws the agent is still alive, so we propagate rather
   * than lie that it stopped (the caller re-enables Stop to allow a retry).
   * Returns false when nothing was cancelled — including the brief window
   * after a send where isStreaming is already true but the request_id hasn't
   * come back yet — so the caller can drop its "stopping" affordance.
   */
  stopStreaming: async () => {
    const { chatSessionId, _activeRequestId, isStreaming } = get();
    if (!isStreaming || !chatSessionId || !_activeRequestId) return false;
    const { cancelled } = await api.cancelReply(chatSessionId, _activeRequestId);
    if (!cancelled) {
      set({ isStreaming: false, _streamingMessageId: null, _activeRequestId: null });
      return false;
    }
    return true;
  },

  /**
   * Decision Inbox deep-link (#303). Open a FRESH chat session (so the
   * discussion isn't tangled with prior chat), focus the chat panel, then send
   * a seed opener. The decision id rides app_context.view_state (set transiently
   * via discussSeedDecisionId → buildAppContext); the daemon loads the decision
   * authoritatively and injects its full context, so the agent's reply opens
   * already knowing the goal, proposal, and reasoning — not a cold "what's up?".
   */
  discussDecision: async (decisionId: string, headline: string) => {
    let sessionId: string;
    try {
      const session = await api.createSession();
      sessionId = session.id;
    } catch (err) {
      console.error('discussDecision: createSession failed', err);
      return;
    }
    get().disconnectSession();
    set({ chatSessionId: sessionId, chatMessages: [], isStreaming: false, _streamingMessageId: null, _activeRequestId: null });
    try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* */ }
    get().connectSession(sessionId);
    get().setActivePanel('chat');
    get().openChatDock(); // dock-first: the discussion must be visible immediately
    // Seed turn carries the decision id; clear it after so later turns don't re-send
    // (the daemon's injected context persists for the session after the first turn).
    set({ discussSeedDecisionId: decisionId });
    try {
      await get().sendMessage(`I'd like to talk through this decision: "${headline}".`);
    } finally {
      set({ discussSeedDecisionId: null });
    }
  },

  switchToSession: async (sessionId: string) => {
    get().disconnectSession();
    set({ chatSessionId: sessionId, chatMessages: [], isStreaming: false, _streamingMessageId: null, _activeRequestId: null });
    try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* */ }
    await get().loadSessionMessages(sessionId);
    // A 404 inside loadSessionMessages disowns the id (the session is gone);
    // don't open an SSE channel to a session the store no longer owns — the
    // reconnect loop keys off chatSessionId and would die silently (C8).
    // Transient failures KEEP the id (inline error + retry), so we still
    // connect: the turn stream works even while history is unloaded.
    if (get().chatSessionId === sessionId) {
      get().connectSession(sessionId);
    }
  },

  deleteSession: async (sessionId: string) => {
    // api.deleteSession throws on a non-2xx (it used to resolve on a 500, and
    // the old unconditional clear below then blanked the OPEN conversation for
    // a session the daemon never deleted). State clears only after a confirmed
    // delete; failures propagate so the caller can surface them (SessionsList
    // toasts) without losing the user's open chat.
    try {
      await api.deleteSession(sessionId);
    } catch (e) {
      console.error('Failed to delete session:', e);
      throw e;
    }
    if (get().chatSessionId === sessionId) {
      set({ chatSessionId: null, chatMessages: [] });
      try { localStorage.removeItem('permagent-chat-session-id'); } catch { /* */ }
    }
    // loadSessions never throws (it latches sessionsError internally), so a
    // refresh hiccup here cannot masquerade as a failed delete.
    await get().loadSessions();
  },

  renameSession: async (sessionId: string, name: string) => {
    try {
      await apiFetch<unknown>(
        `/api/sessions/${encodeURIComponent(sessionId)}/name`,
        { method: 'PUT', body: JSON.stringify({ name }) },
      );
      await get().loadSessions();
    } catch (e) {
      console.error('Failed to rename session:', e);
    }
  },

  // Skills
  skills: [],
  skillsLoading: false,
  selectedSkillId: null,
  setSelectedSkillId: (id) => set({ selectedSkillId: id }),
  loadSkills: async () => {
    set({ skillsLoading: true });
    try {
      const skills = await api.getSkills();
      set({ skills: skills as SkillState[], skillsLoading: false });
    } catch {
      set({ skills: [], skillsLoading: false });
    }
  },

  deleteSkill: async (id) => {
    try {
      await api.deleteSkill(id);
      set(s => ({
        skills: s.skills.filter(sk => sk.id !== id),
        selectedSkillId: s.selectedSkillId === id ? null : s.selectedSkillId,
      }));
    } catch (e) {
      console.error('Failed to delete skill:', e);
    }
  },

  updateSkill: async (id, updates) => {
    try {
      const updated = await api.updateSkill(id, updates);
      set(s => ({
        skills: s.skills.map(sk => sk.id === id ? { ...sk, ...updated } : sk),
      }));
      return true;
    } catch (e) {
      console.error('Failed to update skill:', e);
      return false;
    }
  },

  // Skill proposals
  pendingSkillProposal: null,
  proposals: [],
  saveSkillProposal: async () => {
    const proposal = get().pendingSkillProposal;
    if (!proposal) return;
    try {
      const saved = await api.createSkill({
        name: proposal.description.slice(0, 64).replace(/\s+/g, '-').toLowerCase(),
        description: proposal.description,
        toolUsed: proposal.tool_used,
        argumentShapeHash: proposal.argument_shape_hash,
        definitionJson: { source_task_ids: proposal.source_task_ids },
        sourceTaskId: proposal.source_task_ids[0] || null,
      });
      set({ pendingSkillProposal: null, selectedSkillId: saved.id, activePanel: 'skills' });
      get().loadSkills();
      get().loadProposals();
    } catch (e) {
      console.error('Failed to save skill:', e);
    }
  },
  dismissSkillProposal: () => {
    const proposal = get().pendingSkillProposal;
    if (proposal) {
      api.dismissSkillProposal(proposal.argument_shape_hash).catch(() => {});
    }
    set({ pendingSkillProposal: null });
  },
  loadProposals: async () => {
    try {
      const data = await api.getSkillProposals();
      const proposals: SkillProposal[] = data.map(p => ({
        description: p.description,
        tool_used: p.toolUsed,
        argument_shape_hash: p.argumentShapeHash,
        occurrence_count: p.occurrenceCount,
        source_task_ids: p.sourceTaskIds,
        timestamp: new Date().toISOString(),
      }));
      set({ proposals });
      // Set the first proposal as the banner if none is currently shown
      if (!get().pendingSkillProposal && proposals.length > 0) {
        set({ pendingSkillProposal: proposals[0] });
      }
    } catch {
      set({ proposals: [] });
    }
  },
  saveProposal: async (proposal: SkillProposal) => {
    try {
      await api.createSkill({
        name: proposal.description.slice(0, 64).replace(/\s+/g, '-').toLowerCase(),
        description: proposal.description,
        toolUsed: proposal.tool_used,
        argumentShapeHash: proposal.argument_shape_hash,
        definitionJson: { source_task_ids: proposal.source_task_ids },
        sourceTaskId: proposal.source_task_ids[0] || null,
      });
      // Auto-dismiss banner if it matches the same hash
      const pending = get().pendingSkillProposal;
      if (pending && pending.argument_shape_hash === proposal.argument_shape_hash) {
        set({ pendingSkillProposal: null });
      }
      get().loadSkills();
      get().loadProposals();
    } catch (e) {
      console.error('Failed to save proposal:', e);
    }
  },
  dismissProposal: (argumentShapeHash: string) => {
    api.dismissSkillProposal(argumentShapeHash).catch(() => {});
    set(s => ({
      proposals: s.proposals.filter(p => p.argument_shape_hash !== argumentShapeHash),
      pendingSkillProposal: s.pendingSkillProposal?.argument_shape_hash === argumentShapeHash
        ? null : s.pendingSkillProposal,
    }));
  },

  // Sessions
  sessions: [],
  sessionsError: false,
  loadSessions: async () => {
    // #341 instrumentation: time fetch+parse vs the map+store-commit (which
    // triggers the React re-render of the session list). The map projects away
    // the heavy JSON columns (extension_data/recipe_json/model_config_json) the
    // list never uses — they cost transfer + parse but are discarded here.
    const t0 = performance.now();
    try {
      const sessions = await api.getSessions();
      const tFetched = performance.now();
      set({
        sessions: sessions.map((s: SessionSummary) => ({
          id: s.id,
          name: s.name,
          created_at: s.created_at,
          updated_at: s.updated_at,
          message_count: s.message_count,
        })),
        sessionsError: false,
      });
      console.info(
        `[session-perf] loadSessions fetch+parse=${(tFetched - t0).toFixed(1)}ms ` +
          `map+set=${(performance.now() - tFetched).toFixed(1)}ms count=${sessions.length}`,
      );
    } catch {
      // Daemon unreachable ≠ "no sessions yet" — flag the failure so the list
      // renders an inline error + retry instead of the empty state (#568).
      set({ sessions: [], sessionsError: true });
    }
  },

  /** Load messages from a session's conversation history.
   *
   *  Failure honesty (C8): only a 404 — the session genuinely no longer
   *  exists — disowns the stored session id. Every other failure (daemon
   *  hiccup, network) is transient: the id is KEPT and the error surfaces
   *  inline via sessionLoadError (MessageList renders it with a Retry —
   *  the #568 lesson), instead of silently discarding a working session and
   *  leaving connectSession running against a disowned id. */
  loadSessionMessages: async (sessionId: string) => {
    set({ sessionLoadError: null });
    try {
      const session = await api.getSession(sessionId);
      if (session.conversation && session.conversation.length > 0) {
        const responses = indexToolResponses(session.conversation);
        const msgs = session.conversation.map((m, i) => daemonMsgToChat(m, i, sessionId, responses));
        set({ chatMessages: msgs });
      }
    } catch (err) {
      if ((err as { status?: number }).status === 404) {
        // Session no longer exists — clear the stale ID and start fresh.
        console.warn('Session not found, will create new on next message');
        set({ chatMessages: [], chatSessionId: null });
        try { localStorage.removeItem('permagent-chat-session-id'); } catch { /* */ }
      } else {
        console.error('Failed to load session messages:', err);
        set({
          sessionLoadError: err instanceof Error && err.message
            ? err.message
            : 'Could not reach the agent',
        });
      }
    }
  },

  // Event filters
  eventTypeFilter: '',
  setEventTypeFilter: (type) => set({ eventTypeFilter: type }),

  clearEvents: () => set({ events: [] }),

  loadEvents: async () => {
    // Events come through per-session SSE; no separate REST endpoint
  },

  /** Handle a per-session SSE event (Message, Error, Finish from reply stream) */
  handleSessionEvent: (data: SSEEvent) => {
    // Every Message/Finish frame carries live token + cost state. Capture it so
    // the Build meter reflects real spend the instant a frame lands. Uses the
    // same extractor the meter test drives, so the SSE→meter path is proven.
    const ts = costFromFrame(data);
    if (ts) set({ liveTokens: ts });

    switch (data.type) {
      case 'ActiveRequests': {
        // On EVERY (re)connect the daemon lists this session's in-flight
        // requests — the truth signal in both directions (C1/C4):
        //
        // Non-empty ⇒ a turn IS live server-side. Adopt the id AND flip
        // isStreaming on, so a window that attached mid-turn (reload, detached
        // dock) shows an honest composer + Stop button instead of an idle
        // input whose send would 400 ("already has an active request").
        // try_register_request enforces a single active request, so the first
        // id is the Stop target. _streamingMessageId is left alone: if this
        // window started the turn its placeholder keeps streaming; a freshly
        // attached window has none and settles on the Finish rehydrate (the
        // StreamingIndicator covers the meantime).
        //
        // EMPTY ⇒ the daemon says nothing is running. If we believe a turn is
        // live, it died without a terminal frame (daemon restart mid-turn:
        // fresh bus, empty replay — no Finish/Error is ever coming) — clear
        // streaming state or the composer stays "Agent is responding…"
        // forever. Sole exception: while our own reply POST is still in
        // flight (isStreaming set, request_id not yet returned) the server
        // may simply not have registered the request yet — don't let a
        // racing reconnect kill a turn that is being born.
        const ids = (data as { type: string; request_ids?: string[] }).request_ids;
        if (ids && ids.length > 0) {
          set({ _activeRequestId: ids[0], isStreaming: true });
        } else {
          const { isStreaming, _activeRequestId } = get();
          const replyPostInFlight = isStreaming && !_activeRequestId;
          if (!replyPostInFlight) {
            set({ isStreaming: false, _streamingMessageId: null, _activeRequestId: null });
          }
        }
        break;
      }
      case 'Message': {
        const msg = (data as { type: string; message: DaemonMessage }).message;
        if (msg.role === 'assistant') {
          const delta = extractText(msg);
          const thinkingDelta = extractThinking(msg);
          const streamMsgId = get()._streamingMessageId;
          if (streamMsgId && (delta || thinkingDelta)) {
            const pending = get()._pendingContext;
            set(s => ({
              _pendingContext: null,
              chatMessages: s.chatMessages.map(m =>
                m.id === streamMsgId
                  ? {
                      ...m,
                      content: m.content + delta,
                      ...(thinkingDelta ? { thinking: (m.thinking ?? '') + thinkingDelta } : {}),
                      ...(pending && !m.context_attached ? { context_attached: pending } : {}),
                    }
                  : m
              ),
            }));
          }
        }

        // Trace recording moved to the connectSession funnel (es.onmessage →
        // sessionFrameToRecord): every frame type is recorded there with its
        // real type (tool_call/Message/Error/Finish), not just Message rows.
        break;
      }
      case 'Error': {
        const errMsg = (data as { type: string; error: string }).error;
        set(s => ({
          isStreaming: false,
          _streamingMessageId: null,
          _activeRequestId: null,
          chatMessages: [...s.chatMessages, {
            id: `msg-${Date.now()}-err`,
            role: 'system' as const,
            content: `Error: ${errMsg}`,
            timestamp: new Date().toISOString(),
          }],
        }));
        break;
      }
      case 'Finish': {
        // Speak-replies (#18): voice the completed reply when enabled — after
        // the state settles, never blocking the text path.
        {
          const lastAssistant = [...get().chatMessages].reverse().find(m => m.role === 'assistant');
          // Content-based dedupe: a replayed Finish (fresh window / redock
          // reconnect) reproduces the same reply text and stays silent; a
          // genuinely new reply has new text and speaks. The connect-burst
          // mute covers voice-pipeline turns the dedupe key never saw.
          if (lastAssistant?.content && Date.now() >= _speakSuppressUntil) {
            void maybeSpeakReply(
              lastAssistant.content,
              undefined,
              replyDedupeKey(get().chatSessionId, lastAssistant.content),
            );
          }
        }
        set({ isStreaming: false, _streamingMessageId: null, _activeRequestId: null });
        // Reload proposals + skills after each reply completes — the agent may
        // have created a skill (save_skill) or a new proposal may have fired.
        get().loadProposals();
        get().loadSkills();
        // Rehydrate the conversation from the daemon now the turn is done: it's
        // the authoritative copy, and only IT carries the tool requests joined
        // to their responses (indexToolResponses, #658). So tool cards light up
        // with real names + typed results the moment the turn finishes — no
        // manual reopen. Runs AFTER streaming ends (isStreaming already false),
        // so it never races the streaming text path. (True mid-turn live tool
        // render needs UpdateConversation reconciliation against the client
        // streaming message — a separate, behaviorally-verified follow-on.)
        {
          const sid = get().chatSessionId;
          if (sid) void get().loadSessionMessages(sid);
        }
        break;
      }
      case 'ContextAttached': {
        // Associate probed/recalled memories with the currently streaming message
        const ctx = data as { type: string; probed_memories: ProbedMemoryRef[]; recalled_memories: RecalledMemoryRef[] };
        const streamId = get()._streamingMessageId;
        if (streamId) {
          set(s => ({
            chatMessages: s.chatMessages.map(m =>
              m.id === streamId ? { ...m, context_attached: { probed_memories: ctx.probed_memories, recalled_memories: ctx.recalled_memories } } : m
            ),
          }));
        } else {
          // Store pending context for the next assistant message
          set({ _pendingContext: { probed_memories: ctx.probed_memories, recalled_memories: ctx.recalled_memories } });
        }
        break;
      }
    }
  },

  // Project navigation (from agent/voice)
  pendingProjectNavigation: null,
  setPendingProjectNavigation: (id) => set({ pendingProjectNavigation: id }),

  // Brain-loop "View in Brain" deep-link. Stash the target, then switch to the
  // Brain workspace; BrainView consumes pendingBrainMemory on mount/refresh, so
  // the focus still resolves if the Brain view was not yet open (mirrors the
  // pendingBrowserUrl / pendingTerminalLaunch seams).
  pendingBrainMemory: null,
  focusBrainMemory: (target) => {
    set({ pendingBrainMemory: target });
    navigateToTool('memory');
  },
  clearPendingBrainMemory: () => set({ pendingBrainMemory: null }),

  pendingSettingsSection: null,
  setPendingSettingsSection: (section) => set({ pendingSettingsSection: section }),

  pendingTerminalLaunch: null,
  setPendingTerminalLaunch: (launch) => set({ pendingTerminalLaunch: launch }),

  // In-app browser navigation: post a URL + focus the Build workspace (which
  // hosts the browser). The Browser consumes pendingBrowserUrl on mount, so the
  // URL still resolves if the workspace was not yet open. Shared by chat-link
  // clicks and the self-knowledge tour (#353).
  pendingBrowserUrl: null,
  openGrowForProject: null,
  growProject: (projectId) => { set({ openGrowForProject: projectId }); navigateToTool('grow'); },
  setOpenGrowForProject: (id) => set({ openGrowForProject: id }),
  buildTerminalHidden: false,
  buildBrowserHidden: false,
  // Never allow both hidden: hiding one re-shows the other.
  toggleBuildTerminal: () =>
    set(s => ({
      buildTerminalHidden: !s.buildTerminalHidden,
      buildBrowserHidden: s.buildTerminalHidden ? s.buildBrowserHidden : false,
    })),
  toggleBuildBrowser: () =>
    set(s => ({
      buildBrowserHidden: !s.buildBrowserHidden,
      buildTerminalHidden: s.buildBrowserHidden ? s.buildTerminalHidden : false,
    })),
  openInBrowser: (url) => {
    set({ pendingBrowserUrl: url });
    navigateToTool('build');
  },
  clearPendingBrowserUrl: () => set({ pendingBrowserUrl: null }),

  // Browser overlay z-order
  overlayBlockingBrowser: 0,
  pushBrowserOverlay: () => set(s => ({ overlayBlockingBrowser: s.overlayBlockingBrowser + 1 })),
  popBrowserOverlay: () => set(s => ({ overlayBlockingBrowser: Math.max(0, s.overlayBlockingBrowser - 1) })),

  // Collapsed chat launcher corner reservation (#553)
  chatLauncherSize: null,
  chatDockOpen: false,
  openChatDock: () => set({ chatDockOpen: true }),
  closeChatDock: () => set({ chatDockOpen: false }),
  chatWindowOpen: false,
  setChatWindowOpen: (open) => set({ chatWindowOpen: open }),
  voiceConversation: null,
  setVoiceConversation: (conv) => set({ voiceConversation: conv }),
  voiceEngine: null,
  setVoiceEngine: (engine) => set({ voiceEngine: engine }),
  setChatLauncherSize: (size) => set(s => {
    const prev = s.chatLauncherSize;
    if (prev === size) return s;
    if (prev && size && prev.width === size.width && prev.height === size.height) return s;
    return { chatLauncherSize: size };
  }),

  // ── Per-session SSE (replaces WebSocket) ──
  _eventSource: null,
  _reconnectTimer: null,
  _reconnectAttempts: 0,
  _lastEventId: null,
  _lastEventSessionId: null,

  connectSession: async (sessionId: string) => {
    const epoch = ++_sseConnectEpoch;
    const state = get();
    // Close existing connection
    if (state._eventSource) {
      state._eventSource.close();
    }
    if (state._reconnectTimer) {
      clearTimeout(state._reconnectTimer);
    }

    // The SSE cursor is per-session (sequence numbers restart on every bus):
    // reset it when this connection targets a different session than the one
    // the cursor was recorded against.
    if (state._lastEventSessionId !== sessionId) {
      set({ _lastEventId: null, _lastEventSessionId: sessionId });
    }

    set({ connectionStatus: 'connecting' });
    startEventPruning();

    // Resume the replay after the last event we processed (P1): EventSource
    // can't set the Last-Event-ID header on this manual reconnect, so the
    // cursor rides a query param. First connect (null cursor) replays all.
    // The daemon token rides the query too (C1/C2 auth) — awaiting it opens
    // an async gap, so re-check the epoch before constructing the stream.
    const url = await api.sessionEventsUrl(sessionId, get()._lastEventId);
    if (epoch !== _sseConnectEpoch) return; // superseded while awaiting the token
    const es = new EventSource(url);

    es.onopen = () => {
      set({ connectionStatus: 'connected', _reconnectAttempts: 0 });
      // The daemon replays its buffered frames immediately after connect —
      // including old Finish frames. Mute speak-replies through that burst so
      // a freshly opened window never re-voices history (voice-pipeline turns
      // never touch the speak dedupe key, so content dedupe alone can't
      // catch them).
      _speakSuppressUntil = Date.now() + 2500;
      get().loadSkills();
      get().loadProposals();
      get().loadWorkspaces();
    };

    es.onmessage = (ev) => {
      // Store Last-Event-ID for reconnection
      if (ev.lastEventId) {
        set({ _lastEventId: ev.lastEventId });
      }

      try {
        const data = JSON.parse(ev.data) as SSEEvent;
        // Trace (C3): record the frame with its REAL type — tool-bearing
        // Message frames become tool_call rows, Error/Finish are the turn's
        // lifecycle signals; streaming text deltas coalesce into one row.
        // This is the single production entry point for session frames.
        const rec = sessionFrameToRecord(data);
        if (rec) set(s => ({ events: appendTraceRecord(s.events, rec, MAX_EVENTS_BUFFER) }));
        get().handleSessionEvent(data);
      } catch {
        // Ignore malformed events
      }
    };

    es.onerror = () => {
      es.close();
      set({ connectionStatus: 'disconnected', _eventSource: null });
      const attempts = get()._reconnectAttempts;
      const delay = Math.min(RECONNECT_BASE_MS * Math.pow(2, attempts), RECONNECT_MAX_MS);
      set({ _reconnectAttempts: attempts + 1 });
      const timer = setTimeout(() => {
        const sid = get().chatSessionId;
        if (sid) get().connectSession(sid);
      }, delay);
      set({ _reconnectTimer: timer });
    };

    set({ _eventSource: es });
  },

  disconnectSession: () => {
    _sseConnectEpoch++; // cancel any connect still awaiting the daemon token
    const { _eventSource, _reconnectTimer } = get();
    if (_reconnectTimer) clearTimeout(_reconnectTimer);
    if (_eventSource) _eventSource.close();
    set({
      _eventSource: null, _reconnectTimer: null,
      connectionStatus: 'disconnected', _reconnectAttempts: 0,
    });
  },
}));
