/**
 * Permagent Command Center -- API client
 * Aligned with the actual permagentd (goose-server) endpoints.
 */

const API_BASE_URL = (
  (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  ""
).replace(/\/$/, '');

const SECRET_KEY = (import.meta.env.VITE_SECRET_KEY as string | undefined) || '';

// --- Daemon types ---

/** Content block inside a Message */
export interface TextContent {
  type: 'text';
  text: string;
}

export interface ToolRequestContent {
  type: 'toolRequest';
  id: string;
  toolCall: unknown;
}

export interface ToolResponseContent {
  type: 'toolResponse';
  id: string;
  toolResult: unknown;
}

export type MessageContent = TextContent | ToolRequestContent | ToolResponseContent | { type: string; [key: string]: unknown };

/** Message as serialized by the daemon (camelCase via serde) */
export interface DaemonMessage {
  id?: string;
  role: 'user' | 'assistant';
  created: number;
  content: MessageContent[];
  metadata: { userVisible: boolean; agentVisible: boolean };
}

/** Session as returned by the daemon (snake_case, no rename_all) */
export interface Session {
  id: string;
  name: string;
  working_dir: string;
  user_set_name: boolean;
  session_type: string;
  created_at: string;
  updated_at: string;
  message_count: number;
  total_tokens?: number | null;
  input_tokens?: number | null;
  output_tokens?: number | null;
  accumulated_total_tokens?: number | null;
  accumulated_input_tokens?: number | null;
  accumulated_output_tokens?: number | null;
  conversation?: DaemonMessage[] | null;
  schedule_id?: string | null;
  provider_name?: string | null;
}

export interface SessionListResponse {
  sessions: Session[];
}

/** SSE MessageEvent types from the daemon */
export interface SSEMessageEvent {
  type: 'Message';
  message: DaemonMessage;
  token_state: TokenState;
}

export interface SSEErrorEvent {
  type: 'Error';
  error: string;
}

export interface SSEFinishEvent {
  type: 'Finish';
  reason: string;
  token_state: TokenState;
}

export interface SSEPingEvent {
  type: 'Ping';
}

export type SSEEvent = SSEMessageEvent | SSEErrorEvent | SSEFinishEvent | SSEPingEvent | { type: string; [key: string]: unknown };

export interface TokenState {
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  accumulated_input_tokens: number;
  accumulated_output_tokens: number;
  accumulated_total_tokens: number;
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

function authHeaders(): Record<string, string> {
  const h: Record<string, string> = { 'Content-Type': 'application/json' };
  if (SECRET_KEY) h['x-secret-key'] = SECRET_KEY;
  return h;
}

async function apiFetch<T>(endpoint: string, options?: RequestInit): Promise<T> {
  const url = `${API_BASE_URL}${endpoint}`;
  const response = await fetch(url, {
    ...options,
    headers: {
      ...authHeaders(),
      ...(options?.headers ?? {}),
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: 'Unknown error' }));
    throw new Error(error.message || error.error || `HTTP ${response.status}`);
  }

  return response.json() as Promise<T>;
}

/** Build a user Message in the format the daemon expects */
export function buildUserMessage(text: string): DaemonMessage {
  return {
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

/** Extract the first text content from a DaemonMessage */
export function extractText(msg: DaemonMessage): string {
  return msg.content
    .filter((c): c is TextContent => c.type === 'text')
    .map(c => c.text)
    .join('');
}

/**
 * Parse SSE events from a fetch Response body.
 * Calls onEvent for each parsed event, onDone when stream ends.
 */
export async function parseSSEStream(
  response: Response,
  onEvent: (event: SSEEvent) => void,
  onDone?: () => void,
): Promise<void> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const parts = buffer.split('\n');
      buffer = parts.pop() || '';

      for (const line of parts) {
        if (line.startsWith('data: ')) {
          const json = line.slice(6);
          try {
            const parsed = JSON.parse(json) as SSEEvent;
            onEvent(parsed);
          } catch {
            // skip malformed JSON
          }
        }
        // ignore comment lines (: ping ...) and empty lines
      }
    }
  } finally {
    onDone?.();
  }
}

// --- API ---

export const api = {
  // Health
  getHealth: () => apiFetch<{ status: string }>('/status'),

  // Sessions — GET /sessions returns { sessions: [...] }
  getSessions: async (): Promise<Session[]> => {
    const res = await apiFetch<SessionListResponse>('/sessions');
    return res.sessions;
  },

  // Session detail — GET /sessions/{id} returns Session with conversation
  getSession: (id: string) =>
    apiFetch<Session>(`/sessions/${encodeURIComponent(id)}`),

  // Delete session — DELETE /sessions/{id}
  deleteSession: (id: string) =>
    fetch(`${API_BASE_URL}/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers: authHeaders(),
    }),

  // Create session — POST /agent/start with working_dir
  createSession: (workingDir?: string) =>
    apiFetch<Session>('/agent/start', {
      method: 'POST',
      body: JSON.stringify({ working_dir: workingDir || '/tmp' }),
    }),

  /**
   * Send a message via POST /reply — returns a raw Response for SSE streaming.
   * The caller must parse the SSE stream from the response body.
   */
  sendReply: async (sessionId: string, text: string): Promise<Response> => {
    const response = await fetch(`${API_BASE_URL}/reply`, {
      method: 'POST',
      headers: authHeaders(),
      body: JSON.stringify({
        session_id: sessionId,
        user_message: buildUserMessage(text),
      }),
    });
    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(error.message || `HTTP ${response.status}`);
    }
    return response;
  },

  // Config
  getConfig: () => apiFetch<PermagentConfig>('/config'),

  upsertConfig: (key: string, value: unknown) =>
    apiFetch<PermagentConfig>('/config/upsert', {
      method: 'POST',
      body: JSON.stringify({ key, value }),
    }),

  // Skills CRUD
  getSkills: () => apiFetch<Skill[]>('/permagent/skills').catch(() => [] as Skill[]),

  createSkill: (skill: {
    name: string;
    description: string;
    toolUsed: string;
    argumentShapeHash: string;
    definitionJson: unknown;
    sourceTaskId?: string | null;
  }) =>
    apiFetch<{ id: string; name: string }>('/permagent/skills', {
      method: 'POST',
      body: JSON.stringify(skill),
    }),

  updateSkill: (id: string, updates: Partial<Skill>) =>
    apiFetch<Skill>(`/permagent/skills/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify(updates),
    }),

  getSkillExecutions: (skillId: string) =>
    apiFetch<Array<{ id: string; status: string; started_at: string; completed_at?: string; error_message?: string }>>(
      `/permagent/skills/${encodeURIComponent(skillId)}/executions`
    ).catch(() => []),

  deleteSkill: (id: string) =>
    apiFetch<{ deleted: boolean }>(`/permagent/skills/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  dismissSkillProposal: (argumentShapeHash: string) =>
    apiFetch<void>('/permagent/skills/dismiss', {
      method: 'POST',
      body: JSON.stringify({ argumentShapeHash }),
    }).catch(() => {}),

  // Workspaces
  getWorkspaces: () =>
    apiFetch<Array<{
      id: string; name: string; icon: string; sortOrder: number;
      layoutJson: unknown; isDefault: boolean; createdAt: string; updatedAt: string;
    }>>('/api/workspaces'),

  getWorkspace: (id: string) =>
    apiFetch<{
      id: string; name: string; icon: string; sortOrder: number;
      layoutJson: unknown; isDefault: boolean; createdAt: string; updatedAt: string;
    }>(`/api/workspaces/${encodeURIComponent(id)}`),

  getActiveWorkspace: () =>
    apiFetch<{ workspaceId: string | null }>('/api/workspaces/active'),

  setActiveWorkspace: (workspaceId: string) =>
    apiFetch<void>('/api/workspaces/active', {
      method: 'POST',
      body: JSON.stringify({ workspaceId }),
    }),

  updateWorkspaceLayout: (workspaceId: string, layoutJson: unknown) =>
    fetch(`${API_BASE_URL}/api/workspaces/${encodeURIComponent(workspaceId)}/layout`, {
      method: 'PUT',
      headers: authHeaders(),
      body: JSON.stringify({ layoutJson }),
    }),

  // State snapshot (stubbed until daemon implements)
  getStateSnapshot: () => Promise.resolve({
    tasks: [] as Array<{ id: string; title: string | null; status: string; automation_id: string | null; created_at: string | null; updated_at: string }>,
    service_health: [] as Array<{ service: string; status: string; last_check: string; latency_ms: number }>,
    receipts: [] as Array<{ id: string; run_id: string; step_id: string | null; model: string; input_tokens: number; output_tokens: number; cost_usd: number; recorded_at: string }>,
    spend: { today_usd: 0, month_usd: 0 },
  }),
};
