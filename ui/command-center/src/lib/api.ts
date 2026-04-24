/**
 * Permagent Command Center -- API client
 * Points at permagentd on localhost:3001 (Section D.3)
 */

const API_BASE_URL = (
  (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  `http://${window.location.hostname}:3001`
).replace(/\/$/, '');

// --- Types ---

export interface Session {
  id: string;
  metadata?: Record<string, unknown>;
  created_at?: string;
}

export interface SessionDetail {
  id: string;
  messages: Array<{ role: string; content: string; timestamp?: string }>;
  metadata?: Record<string, unknown>;
}

export interface Skill {
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

export interface PermagentConfig {
  [key: string]: unknown;
}

export interface PermagentEvent {
  id: string;
  type: string;
  timestamp: string;
  payload: Record<string, unknown>;
}

// --- Fetch helper ---

async function apiFetch<T>(endpoint: string, options?: RequestInit): Promise<T> {
  const url = `${API_BASE_URL}${endpoint}`;
  const response = await fetch(url, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(options?.headers ?? {}),
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ error: 'Unknown error' }));
    throw new Error(error.error || `HTTP ${response.status}`);
  }

  return response.json() as Promise<T>;
}

const q = (params: Record<string, string | number | undefined | null>) => {
  const usp = new URLSearchParams();
  Object.entries(params).forEach(([k, v]) => {
    if (v !== undefined && v !== null && `${v}` !== '') usp.set(k, String(v));
  });
  const s = usp.toString();
  return s ? `?${s}` : '';
};

// --- API (Section D.3) ---

export const api = {
  // Health
  getHealth: () => apiFetch<{ status: string }>('/status'),

  // Chat / Reply
  sendReply: (session_id: string, message: string) =>
    apiFetch<{ reply: string; session_id: string }>('/reply', {
      method: 'POST',
      body: JSON.stringify({ session_id, message }),
    }),

  // Sessions
  getSessions: () => apiFetch<Session[]>('/sessions'),

  getSession: (id: string) =>
    apiFetch<SessionDetail>(`/sessions/${encodeURIComponent(id)}`),

  createSession: () =>
    apiFetch<Session>('/sessions', { method: 'POST', body: JSON.stringify({}) }),

  // Config
  getConfig: () => apiFetch<PermagentConfig>('/config'),

  upsertConfig: (key: string, value: unknown) =>
    apiFetch<PermagentConfig>('/config/upsert', {
      method: 'POST',
      body: JSON.stringify({ key, value }),
    }),

  // Skills CRUD (Section D.3)
  getSkills: () => apiFetch<Skill[]>('/permagent/skills').catch(() => {
    console.warn('[api] GET /permagent/skills not implemented yet');
    return [] as Skill[];
  }),

  createSkill: (skill: Partial<Skill>) =>
    apiFetch<Skill>('/permagent/skills', {
      method: 'POST',
      body: JSON.stringify(skill),
    }),

  deleteSkill: (id: string) =>
    apiFetch<{ deleted: boolean }>(`/permagent/skills/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  dismissSkillProposal: () =>
    apiFetch<{ dismissed: boolean }>('/permagent/skills/dismiss', { method: 'POST' }).catch(() => ({ dismissed: true })),

  // Events history
  getEvents: (params?: { type?: string; limit?: number; after?: string; task_id?: string }) =>
    apiFetch<{ events: PermagentEvent[] }>(`/events${q(params ?? {})}`).catch(() => {
      console.warn('[api] GET /events not implemented yet');
      return { events: [] as PermagentEvent[] };
    }),

  // State snapshot (stubbed until daemon implements)
  getStateSnapshot: () => Promise.resolve({
    tasks: [] as Array<{ id: string; title: string | null; status: string; automation_id: string | null; created_at: string | null; updated_at: string }>,
    service_health: [] as Array<{ service: string; status: string; last_check: string; latency_ms: number }>,
    receipts: [] as Array<{ id: string; run_id: string; step_id: string | null; model: string; input_tokens: number; output_tokens: number; cost_usd: number; recorded_at: string }>,
    spend: { today_usd: 0, month_usd: 0 },
  }),
};
