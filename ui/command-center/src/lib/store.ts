import { create } from 'zustand';
import { api, extractText, parseSSEStream } from './api';
import type { Session, DaemonMessage, SSEEvent } from './api';
import { useEventBus, startEventPruning } from './eventBus';
import type { PermagentEvent } from './eventBus';

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

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  task_id?: string;
  tool_calls?: ToolCall[];
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
  usage_count?: number;
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
  | 'memory_added'
  | 'skill_proposed' | 'skill_saved' | 'skill_triggered'
  | 'message_received' | 'stream_chunk'
  | 'integration_connected' | 'integration_error';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

export type ActivePanel = 'chat' | 'skills' | 'events' | 'settings' | 'terminal' | 'browser';

// ── Workspace types ──

export type ToolType = 'chat' | 'skills' | 'trace' | 'world' | 'terminal' | 'browser';

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

  // --- Operational state ---
  tasks: TaskState[];
  serviceHealth: ServiceHealthState[];
  receipts: ReceiptState[];
  events: EventRecord[];
  spendToday: number;
  spendMonth: number;

  // --- Chat state ---
  chatMessages: ChatMessage[];
  chatSessionId: string | null;
  addChatMessage: (msg: ChatMessage) => void;
  _streamingMessageId: string | null;

  // --- SSE streaming ---
  isStreaming: boolean;
  sendMessage: (text: string) => Promise<void>;

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
  saveSkillProposal: () => Promise<void>;
  dismissSkillProposal: () => void;

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
  handleWsEvent: (event: EventRecord) => void;
  clearEvents: () => void;

  // --- WebSocket ---
  _ws: WebSocket | null;
  _reconnectTimer: ReturnType<typeof setTimeout> | null;
  _reconnectAttempts: number;
  _lastEventId: string | null;
  connect: () => void;
  disconnect: () => void;
}

// WebSocket URL — permagentd events endpoint
const WS_URL = (
  (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_WS_URL as string | undefined) ||
  `ws://localhost:3000/events`
);

const MAX_EVENTS_BUFFER = 1000;

// Reconnect: exponential backoff 1s, 2s, 4s, 8s, ... max 30s
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

/** Convert a DaemonMessage to a ChatMessage for display */
function daemonMsgToChat(msg: DaemonMessage, index: number, sessionId: string): ChatMessage {
  const toolCalls: ToolCall[] = [];
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
    }
  }

  return {
    id: msg.id || `hist-${sessionId}-${index}`,
    role: msg.role,
    content: extractText(msg),
    timestamp: new Date(msg.created * 1000).toISOString(),
    tool_calls: toolCalls.length > 0 ? toolCalls : undefined,
  };
}

export const useCommandCenter = create<CommandCenterStore>((set, get) => ({
  // Panel routing
  activePanel: 'chat',
  setActivePanel: (panel) => set({ activePanel: panel }),

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
    // Debounced save — fire and forget
    api.updateWorkspaceLayout(workspaceId, layoutJson).catch(() => {});
  },

  // Connection
  connectionStatus: 'disconnected',

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

  addChatMessage: (msg) => set(s => ({ chatMessages: [...s.chatMessages, msg] })),

  // Streaming
  isStreaming: false,

  /**
   * Send a message to the daemon via POST /reply and process the SSE stream.
   * Creates a session if none exists.
   */
  sendMessage: async (text: string) => {
    const state = get();
    if (state.isStreaming) return;

    // Add user message to chat
    const userMsg: ChatMessage = {
      id: `msg-${Date.now()}`,
      role: 'user',
      content: text,
      timestamp: new Date().toISOString(),
    };
    set(s => ({ chatMessages: [...s.chatMessages, userMsg] }));

    // Ensure we have a session
    let sessionId = state.chatSessionId;
    if (!sessionId) {
      try {
        const session = await api.createSession();
        sessionId = session.id;
        set({ chatSessionId: sessionId });
        try { localStorage.setItem('permagent-chat-session-id', sessionId); } catch { /* */ }
      } catch (err) {
        set(s => ({
          chatMessages: [...s.chatMessages, {
            id: `msg-${Date.now()}-err`,
            role: 'system' as const,
            content: `Failed to create session: ${err instanceof Error ? err.message : 'Unknown error'}`,
            timestamp: new Date().toISOString(),
          }],
        }));
        return;
      }
    }

    set({ isStreaming: true, _streamingMessageId: null });

    try {
      const response = await api.sendReply(sessionId, text);

      // Create a placeholder assistant message for streaming
      const streamMsgId = `msg-${Date.now()}-stream`;
      set(s => ({
        _streamingMessageId: streamMsgId,
        chatMessages: [...s.chatMessages, {
          id: streamMsgId,
          role: 'assistant' as const,
          content: '',
          timestamp: new Date().toISOString(),
        }],
      }));

      let lastContent = '';

      await parseSSEStream(
        response,
        (event: SSEEvent) => {
          switch (event.type) {
            case 'Message': {
              const msg = (event as { type: string; message: DaemonMessage }).message;
              if (msg.role === 'assistant') {
                const text = extractText(msg);
                if (text !== lastContent) {
                  lastContent = text;
                  set(s => ({
                    chatMessages: s.chatMessages.map(m =>
                      m.id === streamMsgId ? { ...m, content: text } : m
                    ),
                  }));
                }
              }
              break;
            }
            case 'Error': {
              const errMsg = (event as { type: string; error: string }).error;
              set(s => ({
                chatMessages: [...s.chatMessages, {
                  id: `msg-${Date.now()}-err`,
                  role: 'system' as const,
                  content: `Error: ${errMsg}`,
                  timestamp: new Date().toISOString(),
                }],
              }));
              break;
            }
            case 'Finish':
              // Stream complete
              break;
            case 'Ping':
              break;
          }
        },
        () => {
          set({ isStreaming: false, _streamingMessageId: null });
        },
      );
    } catch (err) {
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

  // Sessions
  sessions: [],
  loadSessions: async () => {
    try {
      const sessions = await api.getSessions();
      set({
        sessions: sessions.map((s: Session) => ({
          id: s.id,
          name: s.name,
          created_at: s.created_at,
          updated_at: s.updated_at,
          message_count: s.message_count,
        })),
      });
    } catch {
      set({ sessions: [] });
    }
  },

  /** Load messages from a session's conversation history */
  loadSessionMessages: async (sessionId: string) => {
    try {
      const session = await api.getSession(sessionId);
      if (session.conversation && session.conversation.length > 0) {
        const msgs = session.conversation.map((m, i) => daemonMsgToChat(m, i, sessionId));
        set({ chatMessages: msgs });
      }
    } catch {
      // Session may not exist on server yet
    }
  },

  // Event filters
  eventTypeFilter: '',
  setEventTypeFilter: (type) => set({ eventTypeFilter: type }),

  clearEvents: () => set({ events: [] }),

  loadEvents: async () => {
    // Events come through WebSocket; no REST endpoint for event history
    // Keep this as a no-op for now
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

  handleWsEvent: (event) => {
    // Push to EventBus for the event log panel
    const busEvent: PermagentEvent = {
      id: event.id,
      type: event.event_type as PermagentEvent['type'],
      timestamp: event.timestamp,
      payload: event.payload,
    };
    useEventBus.getState().addEvent(busEvent);

    set(s => {
      const newEvents = [event, ...s.events].slice(0, MAX_EVENTS_BUFFER);
      const patch: Partial<CommandCenterStore> = { events: newEvents, _lastEventId: event.id };

      switch (event.event_type) {
        case 'task_created': {
          const p = event.payload as { task_id?: string; title?: string; description?: string; status?: string };
          if (p.task_id) {
            patch.tasks = [...s.tasks, {
              id: p.task_id, title: p.title || p.description || null,
              status: p.status || 'pending', automation_id: null,
              created_at: event.timestamp, updated_at: event.timestamp,
            }];
          }
          break;
        }
        case 'task_started':
        case 'task_completed':
        case 'task_failed': {
          const p = event.payload as { task_id?: string; status?: string };
          if (p.task_id) {
            patch.tasks = s.tasks.map(t =>
              t.id === p.task_id
                ? { ...t, status: p.status || event.event_type.replace('task_', ''), updated_at: event.timestamp }
                : t
            );
          }
          break;
        }
        case 'skill_proposed': {
          const p = event.payload as {
            description?: string; tool_used?: string; argument_shape_hash?: string;
            occurrence_count?: number; source_task_ids?: string[];
          };
          if (p.description && p.argument_shape_hash) {
            patch.pendingSkillProposal = {
              description: p.description,
              tool_used: p.tool_used || '',
              argument_shape_hash: p.argument_shape_hash,
              occurrence_count: p.occurrence_count || 0,
              source_task_ids: p.source_task_ids || [],
              timestamp: event.timestamp,
            };
          }
          break;
        }
        case 'skill_saved': {
          get().loadSkills();
          break;
        }
        case 'skill_triggered': {
          const p = event.payload as { skill_id?: string };
          if (p.skill_id) {
            patch.skills = s.skills.map(sk =>
              sk.id === p.skill_id
                ? { ...sk, usage_count: (sk.usage_count || 0) + 1, last_run: event.timestamp }
                : sk
            );
          }
          break;
        }
        case 'memory_added':
        case 'integration_connected':
        case 'integration_error':
        case 'daemon_started':
        case 'daemon_stopped':
          break;
      }

      return patch;
    });
  },

  // WebSocket
  _ws: null,
  _reconnectTimer: null,
  _reconnectAttempts: 0,
  _lastEventId: null,

  connect: () => {
    const state = get();
    if (state._ws && state._ws.readyState <= WebSocket.OPEN) return;

    set({ connectionStatus: 'connecting' });

    // Start event pruning on first connect
    startEventPruning();

    const ws = new WebSocket(WS_URL);

    ws.onopen = () => {
      set({ connectionStatus: 'connected', _reconnectAttempts: 0 });

      const lastId = get()._lastEventId;
      if (lastId) {
        ws.send(JSON.stringify({ resume_from: lastId }));
      }

      get().loadSnapshot();
      get().loadSkills();
      get().loadWorkspaces();
    };

    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        if (data.id && data.type) {
          const record: EventRecord = {
            id: data.id,
            timestamp: data.timestamp,
            source: data.payload?.source || 'permagentd',
            event_type: data.type,
            severity: data.payload?.severity || 'info',
            run_id: data.payload?.run_id || null,
            task_id: data.payload?.task_id || null,
            agent_id: data.payload?.agent_id || null,
            correlation_id: data.payload?.correlation_id || null,
            payload: data.payload || {},
          };
          get().handleWsEvent(record);
        } else if (data.id && data.event_type) {
          get().handleWsEvent(data as EventRecord);
        }
      } catch {
        // Ignore non-JSON messages
      }
    };

    ws.onclose = () => {
      set({ connectionStatus: 'disconnected', _ws: null });
      const attempts = get()._reconnectAttempts;
      const delay = Math.min(RECONNECT_BASE_MS * Math.pow(2, attempts), RECONNECT_MAX_MS);
      set({ _reconnectAttempts: attempts + 1 });
      const timer = setTimeout(() => get().connect(), delay);
      set({ _reconnectTimer: timer });
    };

    ws.onerror = () => {
      set({ connectionStatus: 'error' });
    };

    set({ _ws: ws });
  },

  disconnect: () => {
    const { _ws, _reconnectTimer } = get();
    if (_reconnectTimer) clearTimeout(_reconnectTimer);
    if (_ws) _ws.close();
    set({
      _ws: null, _reconnectTimer: null,
      connectionStatus: 'disconnected', _reconnectAttempts: 0,
    });
  },
}));
