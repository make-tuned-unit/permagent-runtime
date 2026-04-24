import { create } from 'zustand';
import { api } from './api';

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

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  task_id?: string;
}

export interface SessionState {
  id: string;
  created_at?: string;
  metadata?: Record<string, unknown>;
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

export type ActivePanel = 'chat' | 'skills' | 'events';

interface CommandCenterStore {
  // --- Panel routing ---
  activePanel: ActivePanel;
  setActivePanel: (panel: ActivePanel) => void;

  // --- Connection state ---
  wsConnected: boolean;
  wsReconnecting: boolean;

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

  // --- Skills state ---
  skills: SkillState[];
  skillsLoading: boolean;
  loadSkills: () => Promise<void>;
  deleteSkill: (id: string) => Promise<void>;

  // --- Sessions state ---
  sessions: SessionState[];
  loadSessions: () => Promise<void>;

  // --- Event filters ---
  eventTypeFilter: string;
  setEventTypeFilter: (type: string) => void;

  // --- Actions ---
  loadEvents: (params?: { type?: string; limit?: number }) => Promise<void>;
  loadSnapshot: () => Promise<void>;
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

// WebSocket URL — permagentd events endpoint (Section C.2)
const WS_URL = (
  (import.meta.env.VITE_WS_URL as string | undefined) ||
  `ws://${window.location.hostname}:3001/events`
);

const MAX_EVENTS_BUFFER = 500;

export const useCommandCenter = create<CommandCenterStore>((set, get) => ({
  // Panel routing
  activePanel: 'chat',
  setActivePanel: (panel) => set({ activePanel: panel }),

  // Connection
  wsConnected: false,
  wsReconnecting: false,

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

  addChatMessage: (msg) => set(s => ({ chatMessages: [...s.chatMessages, msg] })),

  // Skills
  skills: [],
  skillsLoading: false,
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
      set(s => ({ skills: s.skills.filter(sk => sk.id !== id) }));
    } catch (e) {
      console.error('Failed to delete skill:', e);
    }
  },

  // Sessions
  sessions: [],
  loadSessions: async () => {
    try {
      const sessions = await api.getSessions();
      set({ sessions: sessions as SessionState[] });
    } catch {
      set({ sessions: [] });
    }
  },

  // Event filters
  eventTypeFilter: '',
  setEventTypeFilter: (type) => set({ eventTypeFilter: type }),

  clearEvents: () => set({ events: [] }),

  loadEvents: async (params) => {
    try {
      const res = await api.getEvents({ limit: 200, ...params });
      const events: EventRecord[] = (res.events ?? []).map(e => ({
        id: e.id,
        timestamp: e.timestamp,
        source: (e.payload?.source as string) || 'permagentd',
        event_type: e.type,
        severity: (e.payload?.severity as string) || 'info',
        run_id: (e.payload?.run_id as string) || null,
        task_id: (e.payload?.task_id as string) || null,
        agent_id: (e.payload?.agent_id as string) || null,
        correlation_id: (e.payload?.correlation_id as string) || null,
        payload: e.payload,
      }));
      set({ events });
    } catch { set({ events: [] }); }
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
    set(s => {
      const newEvents = [event, ...s.events].slice(0, MAX_EVENTS_BUFFER);
      const patch: Partial<CommandCenterStore> = { events: newEvents, _lastEventId: event.id };

      switch (event.event_type) {
        case 'task_created': {
          const p = event.payload as { task_id?: string; title?: string; status?: string };
          if (p.task_id) {
            patch.tasks = [...s.tasks, {
              id: p.task_id, title: p.title || null,
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
        case 'message_received': {
          const p = event.payload as { content?: string; role?: string };
          if (p.content) {
            const msg: ChatMessage = {
              id: event.id,
              role: (p.role as 'user' | 'assistant') || 'assistant',
              content: p.content,
              timestamp: event.timestamp,
            };
            patch.chatMessages = [...s.chatMessages, msg];
          }
          break;
        }
        case 'skill_saved': {
          // Reload skills list when a new skill is saved
          get().loadSkills();
          break;
        }
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

    const ws = new WebSocket(WS_URL);

    ws.onopen = () => {
      set({ wsConnected: true, wsReconnecting: false, _reconnectAttempts: 0 });

      const lastId = get()._lastEventId;
      if (lastId) {
        ws.send(JSON.stringify({ resume_from: lastId }));
      }

      get().loadSnapshot();
      get().loadEvents();
      get().loadSkills();
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
      set({ wsConnected: false, _ws: null });
      const attempts = get()._reconnectAttempts;
      const delay = Math.min(1000 * Math.pow(2, attempts), 30000);
      set({ wsReconnecting: true, _reconnectAttempts: attempts + 1 });
      const timer = setTimeout(() => get().connect(), delay);
      set({ _reconnectTimer: timer });
    };

    ws.onerror = () => {};

    set({ _ws: ws });
  },

  disconnect: () => {
    const { _ws, _reconnectTimer } = get();
    if (_reconnectTimer) clearTimeout(_reconnectTimer);
    if (_ws) _ws.close();
    set({ _ws: null, _reconnectTimer: null, wsConnected: false, wsReconnecting: false, _reconnectAttempts: 0 });
  },
}));
