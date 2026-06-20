import { create } from 'zustand';
import { api, apiFetch, extractText, fileToBase64, readerIngest } from './api';
import type { Session, DaemonMessage, SSEEvent, AppContextPayload } from './api';
import { startEventPruning } from './eventBus';

// --- Types ---

export interface TaskState {
  id: string;
  title: string | null;
  status: string;
  automation_id: string | null;
  created_at: string | null;
  updated_at: string;
}

export interface ServiceHealthState {
  service: string;
  status: string;
  last_check: string;
  latency_ms: number;
}

export interface ReceiptState {
  id: string;
  run_id: string;
  step_id: string | null;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  recorded_at: string;
}

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
  | 'agent_state_changed'
  | 'skill_proposed' | 'skill_saved' | 'skill_triggered'
  | 'message_received' | 'stream_chunk'
  | 'integration_connected' | 'integration_error'
  | 'librarian_describe_started' | 'librarian_describe_token' | 'librarian_describe_retry' | 'librarian_describe_completed';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

export type ActivePanel = 'chat' | 'skills' | 'events' | 'settings' | 'sessions' | 'terminal' | 'browser';

// ── Workspace types ──

export type ToolType = 'chat' | 'skills' | 'trace' | 'world' | 'terminal' | 'browser' | 'memory' | 'dashboard' | 'build' | 'automate' | 'projects';

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
  switchWorkspace: (workspaceId: string) => void;
  updateWorkspaceLayout: (workspaceId: string, layoutJson: LayoutNode) => void;

  // --- Connection state ---
  connectionStatus: ConnectionStatus;

  // --- Provider state ---
  providers: ProviderInfo[];
  currentModel: string | null;
  loadProviders: () => Promise<void>;
  setDefaultProvider: (name: string, model: string) => Promise<void>;

  // --- Operational state ---
  tasks: TaskState[];
  serviceHealth: ServiceHealthState[];
  receipts: ReceiptState[];
  events: EventRecord[];
  spendToday: number;
  spendMonth: number;

  // --- Agent identity ---
  agentName: string;
  setAgentName: (name: string) => void;

  // --- Chat state ---
  chatMessages: ChatMessage[];
  chatSessionId: string | null;
  addChatMessage: (msg: ChatMessage) => void;
  _streamingMessageId: string | null;
  _pendingContext: { probed_memories: ProbedMemoryRef[]; recalled_memories: RecalledMemoryRef[] } | null;

  // --- SSE streaming ---
  isStreaming: boolean;
  sendMessage: (text: string, files?: File[]) => Promise<void>;
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
  updateSkill: (id: string, updates: Partial<SkillState>) => Promise<void>;

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
  loadSessions: () => Promise<void>;

  // --- Event filters ---
  eventTypeFilter: string;
  setEventTypeFilter: (type: string) => void;

  // --- Actions ---
  loadEvents: (params?: { type?: string; limit?: number }) => Promise<void>;
  loadSnapshot: () => Promise<void>;
  loadSessionMessages: (sessionId: string) => Promise<void>;
  handleSessionEvent: (data: SSEEvent) => void;
  clearEvents: () => void;

  // --- Project navigation (from agent/voice) ---
  pendingProjectNavigation: string | null;
  setPendingProjectNavigation: (id: string | null) => void;

  // --- In-app browser navigation (chat links, agent tour #353) ---
  pendingBrowserUrl: string | null;
  openInBrowser: (url: string) => void;
  clearPendingBrowserUrl: () => void;

  // --- Browser overlay z-order ---
  overlayBlockingBrowser: number;
  pushBrowserOverlay: () => void;
  popBrowserOverlay: () => void;

  // --- Per-session SSE ---
  _eventSource: EventSource | null;
  _reconnectTimer: ReturnType<typeof setTimeout> | null;
  _reconnectAttempts: number;
  _lastEventId: string | null;
  connectSession: (sessionId: string) => void;
  disconnectSession: () => void;
  ensureSession: () => Promise<string | null>;
}

const MAX_EVENTS_BUFFER = 1000;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

/** Convert a DaemonMessage to a ChatMessage for display */
function daemonMsgToChat(msg: DaemonMessage, index: number, sessionId: string): ChatMessage {
  const toolCalls: ToolCall[] = [];
  const images: ChatMessageImage[] = [];
  for (const c of msg.content) {
    if (c.type === 'toolRequest') {
      const tr = c as { type: string; id: string; toolCall?: { name?: string; arguments?: Record<string, unknown> } };
      const call = tr.toolCall;
      if (call) {
        toolCalls.push({
          name: call.name || 'unknown',
          arguments: call.arguments || {},
        });
      }
    } else if (c.type === 'image') {
      const img = c as { type: string; data: string; mimeType: string };
      if (img.data && img.mimeType) {
        images.push({ data: img.data, mimeType: img.mimeType });
      }
    }
  }

  return {
    id: msg.id || `hist-${sessionId}-${index}`,
    role: msg.role,
    content: extractText(msg),
    timestamp: new Date(msg.created * 1000).toISOString(),
    tool_calls: toolCalls.length > 0 ? toolCalls : undefined,
    images: images.length > 0 ? images : undefined,
  };
}

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

  switchWorkspace: (workspaceId: string) => {
    set({ activeWorkspaceId: workspaceId });
    api.setActiveWorkspace(workspaceId).catch(() => {});
  },

  updateWorkspaceLayout: (workspaceId: string, layoutJson: LayoutNode) => {
    set(s => ({
      workspaces: s.workspaces.map(w =>
        w.id === workspaceId ? { ...w, layoutJson } : w
      ),
    }));
    api.updateWorkspaceLayout(workspaceId, layoutJson).catch(() => {});
  },

  // Connection
  connectionStatus: 'disconnected',

  // Providers
  providers: [],
  currentModel: null,
  loadProviders: async () => {
    try {
      const configResp = await api.getConfig();
      const cfgMap = ((configResp as Record<string, unknown>)['config'] ?? configResp) as Record<string, unknown>;
      const currentModel = cfgMap['GOOSE_MODEL'] as string | undefined;

      const raw = await api.getProviders();
      set({
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
        })),
      });
    } catch {
      set({ providers: [] });
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
  tasks: [],
  serviceHealth: [],
  receipts: [],
  events: [],
  spendToday: 0,
  spendMonth: 0,

  // Chat
  chatMessages: [],
  chatSessionId: (() => {
    try { return localStorage.getItem('permagent-chat-session-id'); } catch { return null; }
  })(),
  _streamingMessageId: null,
  _pendingContext: null,

  addChatMessage: (msg) => set(s => ({ chatMessages: [...s.chatMessages, msg] })),

  // Streaming
  isStreaming: false,

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
            console.error('[reader] document ingest failed:', f.name, err);
            digests.push({ name: f.name, summary: '(could not extract text from this file)', recall_query: '' });
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
      // Fire-and-forget: events arrive on SSE channel
      await api.sendReply(sessionId, outgoingText, images, appContext);
    } catch (err) {
      console.error('[send] api.sendReply FAILED:', err);
      set(s => ({
        isStreaming: false,
        _streamingMessageId: null,
        chatMessages: [...s.chatMessages, {
          id: `msg-${Date.now()}-err`,
          role: 'system' as const,
          content: `Failed: ${err instanceof Error ? err.message : 'Unknown error'}`,
          timestamp: new Date().toISOString(),
        }],
      }));
    }
  },

  switchToSession: async (sessionId: string) => {
    get().disconnectSession();
    set({ chatSessionId: sessionId, chatMessages: [], isStreaming: false, _streamingMessageId: null });
    try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* */ }
    await get().loadSessionMessages(sessionId);
    get().connectSession(sessionId);
  },

  deleteSession: async (sessionId: string) => {
    try {
      await api.deleteSession(sessionId);
      const state = get();
      if (state.chatSessionId === sessionId) {
        set({ chatSessionId: null, chatMessages: [] });
        try { localStorage.removeItem('permagent-chat-session-id'); } catch { /* */ }
      }
      await get().loadSessions();
    } catch (e) {
      console.error('Failed to delete session:', e);
    }
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
    } catch (e) {
      console.error('Failed to update skill:', e);
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
        sessions: sessions.map((s: Session) => ({
          id: s.id,
          name: s.name,
          created_at: s.created_at,
          updated_at: s.updated_at,
          message_count: s.message_count,
        })),
      });
      console.info(
        `[session-perf] loadSessions fetch+parse=${(tFetched - t0).toFixed(1)}ms ` +
          `map+set=${(performance.now() - tFetched).toFixed(1)}ms count=${sessions.length}`,
      );
    } catch {
      set({ sessions: [] });
    }
  },

  /** Load messages from a session's conversation history. Handles 404 gracefully. */
  loadSessionMessages: async (sessionId: string) => {
    try {
      const session = await api.getSession(sessionId);
      if (session.conversation && session.conversation.length > 0) {
        const msgs = session.conversation.map((m, i) => daemonMsgToChat(m, i, sessionId));
        set({ chatMessages: msgs });
      }
    } catch {
      // Session may not exist (404) — clear stale ID and start fresh
      console.warn('Session not found, will create new on next message');
      set({ chatMessages: [], chatSessionId: null });
      try { localStorage.removeItem('permagent-chat-session-id'); } catch { /* */ }
    }
  },

  // Event filters
  eventTypeFilter: '',
  setEventTypeFilter: (type) => set({ eventTypeFilter: type }),

  clearEvents: () => set({ events: [] }),

  loadEvents: async () => {
    // Events come through per-session SSE; no separate REST endpoint
  },

  loadSnapshot: async () => {
    try {
      const snapshot = await api.getStateSnapshot();
      const spendToday = snapshot.spend?.today_usd ?? 0;
      const spendMonth = snapshot.spend?.month_usd ?? 0;
      const serviceHealth = (snapshot.service_health || []).map(h => ({ ...h }));

      set({
        tasks: (snapshot.tasks || []).map(t => ({ ...t, title: t.title || null })),
        serviceHealth,
        receipts: (snapshot.receipts || []).map(r => ({ ...r })),
        spendToday,
        spendMonth,
      });
    } catch {
      set({
        tasks: [], serviceHealth: [], receipts: [],
        spendToday: 0, spendMonth: 0,
      });
    }
  },

  /** Handle a per-session SSE event (Message, Error, Finish from reply stream) */
  handleSessionEvent: (data: SSEEvent) => {
    switch (data.type) {
      case 'Message': {
        const msg = (data as { type: string; message: DaemonMessage }).message;
        if (msg.role === 'assistant') {
          const delta = extractText(msg);
          const streamMsgId = get()._streamingMessageId;
          if (streamMsgId && delta) {
            const pending = get()._pendingContext;
            set(s => ({
              _pendingContext: null,
              chatMessages: s.chatMessages.map(m =>
                m.id === streamMsgId
                  ? { ...m, content: m.content + delta, ...(pending && !m.context_attached ? { context_attached: pending } : {}) }
                  : m
              ),
            }));
          }
        }

        // Also push to trace events
        const record: EventRecord = {
          id: `sse-${Date.now()}`,
          timestamp: new Date().toISOString(),
          source: 'permagentd',
          event_type: 'Message',
          severity: 'info',
          run_id: null,
          task_id: null,
          agent_id: null,
          correlation_id: null,
          payload: data as unknown as Record<string, unknown>,
        };
        set(s => ({ events: [record, ...s.events].slice(0, MAX_EVENTS_BUFFER) }));
        break;
      }
      case 'Error': {
        const errMsg = (data as { type: string; error: string }).error;
        set(s => ({
          isStreaming: false,
          _streamingMessageId: null,
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
        set({ isStreaming: false, _streamingMessageId: null });
        // Reload proposals + skills after each reply completes — the agent may
        // have created a skill (save_skill) or a new proposal may have fired.
        get().loadProposals();
        get().loadSkills();
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

  // In-app browser navigation: post a URL + focus the Build workspace (which
  // hosts the browser). The Browser consumes pendingBrowserUrl on mount, so the
  // URL still resolves if the workspace was not yet open. Shared by chat-link
  // clicks and the self-knowledge tour (#353).
  pendingBrowserUrl: null,
  openInBrowser: (url) => {
    set({ pendingBrowserUrl: url });
    navigateToTool('build');
  },
  clearPendingBrowserUrl: () => set({ pendingBrowserUrl: null }),

  // Browser overlay z-order
  overlayBlockingBrowser: 0,
  pushBrowserOverlay: () => set(s => ({ overlayBlockingBrowser: s.overlayBlockingBrowser + 1 })),
  popBrowserOverlay: () => set(s => ({ overlayBlockingBrowser: Math.max(0, s.overlayBlockingBrowser - 1) })),

  // ── Per-session SSE (replaces WebSocket) ──
  _eventSource: null,
  _reconnectTimer: null,
  _reconnectAttempts: 0,
  _lastEventId: null,

  connectSession: (sessionId: string) => {
    const state = get();
    // Close existing connection
    if (state._eventSource) {
      state._eventSource.close();
    }
    if (state._reconnectTimer) {
      clearTimeout(state._reconnectTimer);
    }

    set({ connectionStatus: 'connecting' });
    startEventPruning();

    const url = api.sessionEventsUrl(sessionId);
    const es = new EventSource(url);

    es.onopen = () => {
      set({ connectionStatus: 'connected', _reconnectAttempts: 0 });
      get().loadSnapshot();
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
    const { _eventSource, _reconnectTimer } = get();
    if (_reconnectTimer) clearTimeout(_reconnectTimer);
    if (_eventSource) _eventSource.close();
    set({
      _eventSource: null, _reconnectTimer: null,
      connectionStatus: 'disconnected', _reconnectAttempts: 0,
    });
  },
}));
