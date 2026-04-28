/**
 * Permagent Command Center -- API client
 * Aligned with the actual permagentd endpoints.
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

/** Read a File as base64 data string (no data-URI prefix). */
export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      // Strip "data:image/png;base64," prefix
      resolve(result.split(',')[1] || result);
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}

/** Build a user Message in the format the daemon expects */
export function buildUserMessage(
  text: string,
  images?: Array<{ data: string; mime_type: string }>,
): DaemonMessage {
  const content: MessageContent[] = [];
  if (images) {
    for (const img of images) {
      content.push({ type: 'image', data: img.data, mimeType: img.mime_type } as unknown as MessageContent);
    }
  }
  content.push({ type: 'text', text });
  return {
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content,
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

  // Sessions — GET /api/sessions returns { sessions: [...] }
  getSessions: async (): Promise<Session[]> => {
    const res = await apiFetch<SessionListResponse>('/api/sessions');
    return res.sessions;
  },

  // Session detail — GET /api/sessions/{id} returns Session with conversation
  getSession: (id: string) =>
    apiFetch<Session>(`/api/sessions/${encodeURIComponent(id)}`),

  // Delete session — DELETE /api/sessions/{id}
  deleteSession: (id: string) =>
    fetch(`${API_BASE_URL}/api/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers: authHeaders(),
    }),

  // Create session — POST /api/sessions
  createSession: (workingDir?: string) =>
    apiFetch<Session>('/api/sessions', {
      method: 'POST',
      body: JSON.stringify(workingDir ? { workingDir } : {}),
    }),

  /**
   * Send a message via POST /sessions/{id}/reply (per-session).
   * Returns { request_id } — events arrive on the SSE channel.
   */
  sendReply: async (
    sessionId: string,
    text: string,
    images?: Array<{ data: string; mime_type: string }>,
  ): Promise<{ request_id: string }> => {
    const requestId = crypto.randomUUID();
    return apiFetch<{ request_id: string }>(
      `/sessions/${encodeURIComponent(sessionId)}/reply`,
      {
        method: 'POST',
        body: JSON.stringify({
          request_id: requestId,
          user_message: buildUserMessage(text, images),
        }),
      },
    );
  },

  /** Build the SSE URL for per-session events. */
  sessionEventsUrl: (sessionId: string): string =>
    `${API_BASE_URL}/sessions/${encodeURIComponent(sessionId)}/events`,

  // Config
  getConfig: () => apiFetch<PermagentConfig>('/config'),

  upsertConfig: (key: string, value: unknown, isSecret?: boolean) =>
    apiFetch<unknown>('/config/upsert', {
      method: 'POST',
      body: JSON.stringify({ key, value, is_secret: isSecret ?? false }),
    }),

  // Providers
  getProviders: () =>
    apiFetch<Array<{
      name: string;
      metadata: {
        name: string;
        display_name: string;
        description: string;
        default_model: string;
        known_models: Array<{ name: string }>;
        config_keys: Array<{ name: string; required: boolean; secret: boolean; description?: string }>;
      };
      is_configured: boolean;
      provider_type: string;
    }>>('/config/providers'),

  getProviderModels: (name: string) =>
    apiFetch<string[]>(`/config/providers/${encodeURIComponent(name)}/models`).catch(() => []),

  setProvider: async (provider: string, model: string): Promise<void> => {
    const url = `${API_BASE_URL}/config/set_provider`;
    const response = await fetch(url, {
      method: 'POST',
      headers: authHeaders(),
      body: JSON.stringify({ provider, model }),
    });
    if (!response.ok) {
      const err = await response.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(err.message || `HTTP ${response.status}`);
    }
  },

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

  // Attachments
  uploadAttachments: async (sessionId: string, files: File[]): Promise<{
    attachments: Array<{ id: string; filename: string; mime_type: string; size_bytes: number; created_at: string }>;
  }> => {
    const form = new FormData();
    for (const file of files) {
      form.append('files', file, file.name);
    }
    const headers: Record<string, string> = {};
    if (SECRET_KEY) headers['x-secret-key'] = SECRET_KEY;

    const response = await fetch(
      `${API_BASE_URL}/api/sessions/${encodeURIComponent(sessionId)}/upload`,
      { method: 'POST', headers, body: form },
    );
    if (!response.ok) {
      const err = await response.json().catch(() => ({ message: 'Upload failed' }));
      throw new Error(err.message || `HTTP ${response.status}`);
    }
    return response.json();
  },

  getAttachmentUrl: (sessionId: string, attachmentId: string): string =>
    `${API_BASE_URL}/api/sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,

  deleteAttachment: (sessionId: string, attachmentId: string) =>
    fetch(
      `${API_BASE_URL}/api/sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,
      { method: 'DELETE', headers: authHeaders() },
    ),

  // State snapshot (stubbed until daemon implements)
  getStateSnapshot: () => Promise.resolve({
    tasks: [] as Array<{ id: string; title: string | null; status: string; automation_id: string | null; created_at: string | null; updated_at: string }>,
    service_health: [] as Array<{ service: string; status: string; last_check: string; latency_ms: number }>,
    receipts: [] as Array<{ id: string; run_id: string; step_id: string | null; model: string; input_tokens: number; output_tokens: number; cost_usd: number; recorded_at: string }>,
    spend: { today_usd: 0, month_usd: 0 },
  }),
};
