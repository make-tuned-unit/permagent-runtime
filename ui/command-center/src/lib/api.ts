/**
 * Permagent Command Center -- API client
 * Aligned with the actual permagentd endpoints.
 */

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const API_BASE_URL = (
  (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  (isTauri ? 'http://127.0.0.1:3001' : '')
).replace(/\/$/, '');

export function getApiBaseUrl(): string { return API_BASE_URL; }

// Daemon Bearer token — loaded at runtime from Tauri IPC (not baked into the build).
let _daemonToken: string | null = null;
let _daemonTokenPromise: Promise<string | null> | null = null;

export function loadDaemonToken(): Promise<string | null> {
  if (_daemonToken) return Promise.resolve(_daemonToken);
  if (!isTauri) return Promise.resolve(null);
  if (!_daemonTokenPromise) {
    _daemonTokenPromise = import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<string>('get_daemon_token'))
      .then(token => { _daemonToken = token; return token; })
      .catch(() => null);
  }
  return _daemonTokenPromise;
}

// Kick off token loading immediately so it's ready when needed.
if (isTauri) loadDaemonToken();

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

/** UI state snapshot sent with each chat message. */
export interface AppContextPayload {
  current_tab: string;
  active_panel?: string;
  selected_id?: string;
  view_state?: unknown;
}

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

/** Lean session projection returned by the LIST path (GET /api/sessions).
 *  Excludes the heavy fields (extension_data, recipe, model_config, conversation)
 *  that the full Session carries — those come from single-session GET. See #341/#371. */
export interface SessionSummary {
  id: string;
  name: string;
  user_set_name: boolean;
  session_type: string;
  created_at: string;
  updated_at: string;
  message_count: number;
}

export interface SessionListResponse {
  sessions: SessionSummary[];
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
  usageCount?: number;
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
  if (_daemonToken) h['Authorization'] = `Bearer ${_daemonToken}`;
  return h;
}

export async function apiFetch<T>(endpoint: string, options?: RequestInit): Promise<T> {
  // Ensure the daemon token is loaded before making any authenticated request.
  if (!_daemonToken && isTauri) await loadDaemonToken();
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

/** Compact result of routing a dropped file to the local Reader (#296). */
export interface ReaderDigest {
  summary: string;
  recall_query: string;
  source: string;
  token_count: number;
  char_count: number;
  is_visual: boolean;
  memory_key: string;
  already_ingested: boolean;
}

/**
 * Route a dropped file to the local Reader for OCR/extraction + Brain ingest.
 * Returns a compact digest instead of the raw bytes. When `is_visual` is true
 * the Reader found too little text to ingest — the caller should fall back to
 * sending the image to the agent so visual Q&A still works.
 */
export async function readerIngest(file: File): Promise<ReaderDigest> {
  if (!_daemonToken && isTauri) await loadDaemonToken();
  const form = new FormData();
  form.append('file', file, file.name);
  // Do NOT set Content-Type — the browser adds the multipart boundary.
  const headers: Record<string, string> = {};
  if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;
  const resp = await fetch(`${API_BASE_URL}/api/reader/ingest`, {
    method: 'POST',
    headers,
    body: form,
  });
  if (!resp.ok) throw new Error(`reader ingest HTTP ${resp.status}`);
  return resp.json() as Promise<ReaderDigest>;
}

/** One selectable voice from the loaded Kokoro pack (GET /api/voices). */
export interface VoiceInfo {
  id: string;        // pack key persisted to persona.voice_id, e.g. "bf_emma"
  label: string;     // "British English Female — Emma"
  language: string;  // "British English"
  gender: string;    // "Female" | "Male" | ""
}

/** Voice-asset availability (GET /voice/models). */
export interface VoiceModelStatus {
  models_present: boolean;
  tts_loaded: boolean;
  downloading: boolean;
}

/** In-flight download progress (GET /voice/models/download). */
export interface VoiceDownloadProgress {
  model_id: string;
  status: 'downloading' | 'completed' | 'failed' | 'cancelled';
  bytes_downloaded: number;
  total_bytes: number;
  progress_percent: number;
  error: string | null;
}

/**
 * Synthesize `text` in `voiceId` and return playable WAV audio.
 * Returns the Blob (not JSON) — used for per-voice preview, the picker
 * audition, and the spoken opening greeting. Throws (503) when the voice
 * assets aren't downloaded yet — callers gate on getVoiceModelStatus first.
 */
export async function synthesizeVoice(text: string, voiceId?: string | null): Promise<Blob> {
  if (!_daemonToken && isTauri) await loadDaemonToken();
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;
  const resp = await fetch(`${API_BASE_URL}/voice/synthesize`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ text, voice_id: voiceId ?? null }),
  });
  if (!resp.ok) {
    const err = await resp.json().catch(() => ({ message: `HTTP ${resp.status}` }));
    throw new Error(err.message || `HTTP ${resp.status}`);
  }
  return resp.blob();
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

/** Mirrors the backend ExtensionQuery (POST /config/extensions). The config is
 *  a tagged ExtensionConfig — `type` selects the transport. v1 search uses the
 *  `stdio` variant; the API key is read from the keychain via `env_keys`. */
export interface ExtensionQuery {
  name: string;
  enabled: boolean;
  config: {
    type: 'stdio' | 'streamable_http' | 'builtin' | 'platform';
    name: string;
    description: string;
    cmd?: string;
    args?: string[];
    uri?: string;
    headers?: Record<string, string>;
    env_keys?: string[];
    timeout?: number;
    [key: string]: unknown;
  };
}

export const api = {
  // Health
  getHealth: () => apiFetch<{ status: string }>('/status'),

  // Sessions — GET /api/sessions returns { sessions: [...] } (lean SessionSummary)
  // #341 instrumentation: split the client-perceived cost into round-trip
  // (network + backend), body-download, and JSON.parse, plus payload size. The
  // lean projection (#341b) drops the heavy JSON blobs the consumer discarded —
  // watch `bytes` fall from ~700KB toward ~55KB for the same session count.
  getSessions: async (): Promise<SessionSummary[]> => {
    const t0 = performance.now();
    if (!_daemonToken && isTauri) await loadDaemonToken();
    const res = await fetch(`${API_BASE_URL}/api/sessions`, { headers: authHeaders() });
    const tResp = performance.now();
    if (!res.ok) {
      const err = await res.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(err.message || err.error || `HTTP ${res.status}`);
    }
    const text = await res.text();
    const tBody = performance.now();
    const parsed = JSON.parse(text) as SessionListResponse;
    const tParse = performance.now();
    const count = parsed.sessions?.length ?? 0;
    console.info(
      `[session-perf] GET /api/sessions roundtrip=${(tResp - t0).toFixed(1)}ms ` +
        `body=${(tBody - tResp).toFixed(1)}ms parse=${(tParse - tBody).toFixed(1)}ms ` +
        `total=${(tParse - t0).toFixed(1)}ms bytes=${text.length} count=${count}`,
    );
    return parsed.sessions;
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
    appContext?: AppContextPayload,
  ): Promise<{ request_id: string }> => {
    const requestId = crypto.randomUUID();
    const userMessage = buildUserMessage(text, images);
    const contentTypes = userMessage.content.map(c => (c as { type: string }).type);
    console.log('[api-reply] POST /sessions/' + sessionId + '/reply',
      '— content blocks:', contentTypes,
      'images:', images?.length ?? 0,
      'text length:', text.length);
    const body: Record<string, unknown> = {
      request_id: requestId,
      user_message: userMessage,
    };
    if (appContext) {
      body.app_context = appContext;
    }
    const result = await apiFetch<{ request_id: string }>(
      `/sessions/${encodeURIComponent(sessionId)}/reply`,
      {
        method: 'POST',
        body: JSON.stringify(body),
      },
    );
    console.log('[api-reply] response:', result);
    return result;
  },

  /** Build the SSE URL for per-session events. */
  sessionEventsUrl: (sessionId: string): string =>
    `${API_BASE_URL}/sessions/${encodeURIComponent(sessionId)}/events`,

  // Identity
  getIdentity: () => apiFetch<{
    first_name: string; last_name: string | null; nickname: string | null;
    display_name: string; traits: string[]; tone: string;
    opening_greeting: string; voice_id: string | null;
  }>('/api/agent/identity'),

  putIdentity: (update: {
    first_name: string; last_name?: string | null; nickname?: string | null;
    traits: string[]; tone: string; opening_greeting: string; voice_id?: string | null;
  }) => apiFetch<{ first_name: string; display_name: string }>('/api/agent/identity', {
    method: 'PUT', body: JSON.stringify(update),
  }),

  // Voice
  getVoices: () => apiFetch<VoiceInfo[]>('/api/voices'),

  getVoiceModelStatus: () =>
    apiFetch<VoiceModelStatus>('/voice/models'),

  downloadVoiceModels: () =>
    apiFetch<VoiceModelStatus>('/voice/models/download', { method: 'POST' }),

  getVoiceDownloadProgress: () =>
    apiFetch<VoiceDownloadProgress>('/voice/models/download'),

  // Config
  getConfig: () => apiFetch<PermagentConfig>('/config'),

  readConfig: (key: string, isSecret?: boolean) =>
    apiFetch<{ value?: unknown; masked_value?: string }>('/config/read', {
      method: 'POST', body: JSON.stringify({ key, is_secret: isSecret ?? false }),
    }),

  // Extensions / MCP tools
  getExtensions: () => apiFetch<{
    extensions: Array<{
      enabled: boolean; type: string; name: string;
      description: string; display_name: string;
      bundled: boolean; available_tools: string[];
    }>; warnings: string[];
  }>('/config/extensions'),

  upsertConfig: (key: string, value: unknown, isSecret?: boolean) =>
    apiFetch<unknown>('/config/upsert', {
      method: 'POST',
      body: JSON.stringify({ key, value, is_secret: isSecret ?? false }),
    }),

  // Add or update an MCP extension (persists to config.yaml). Used to register
  // the Brave / Tavily search connectors once their key is stored.
  addExtension: (query: ExtensionQuery) =>
    apiFetch<string>('/config/extensions', {
      method: 'POST', body: JSON.stringify(query),
    }),

  removeExtension: (name: string) =>
    apiFetch<string>(`/config/extensions/${encodeURIComponent(name)}`, {
      method: 'DELETE',
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
      is_default?: boolean;
      provider_type: string;
    }>>('/config/providers'),

  getProviderModels: (name: string) =>
    apiFetch<string[]>(`/config/providers/${encodeURIComponent(name)}/models`).catch(() => []),

  reloadConfig: () =>
    apiFetch<{ provider: string; keyTail: string }>('/config/reload', { method: 'POST' }),

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

  getSkillProposals: () => apiFetch<Array<{
    toolUsed: string;
    argumentShapeHash: string;
    occurrenceCount: number;
    description: string;
    sourceTaskIds: string[];
  }>>('/permagent/skills/proposals').catch(() => []),

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
    if (!_daemonToken && isTauri) await loadDaemonToken();
    const headers: Record<string, string> = {};
    if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;

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

  /** Fetch with daemon Bearer token auth (for /activity/* endpoints). */
  fetchAuthed: async (endpoint: string, options?: RequestInit): Promise<Response> => {
    const token = _daemonToken ?? await loadDaemonToken();
    const url = `${API_BASE_URL}${endpoint}`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    };
    return fetch(url, { ...options, headers: { ...headers, ...(options?.headers as Record<string, string> ?? {}) } });
  },

  // ── Ollama + Librarian ──────────────────────────────────────────

  getOllamaStatus: () =>
    apiFetch<{
      reachable: boolean;
      installed: Array<{ name: string; size: number; digest: string; modified_at: string }>;
      running: Array<{ name: string; size: number; size_vram: number; digest: string; expires_at: string }>;
    }>('/api/ollama/status'),

  getLibrarianSchedule: () =>
    apiFetch<{
      enabled: boolean;
      start_time: string;
      duration_minutes: number;
      model: string;
      run_if_launched_in_window: boolean;
    }>('/api/librarian/schedule'),

  setLibrarianSchedule: async (schedule: {
    enabled: boolean;
    start_time: string;
    duration_minutes: number;
    model: string;
    run_if_launched_in_window: boolean;
  }) => {
    const url = `${API_BASE_URL}/api/librarian/schedule`;
    const resp = await fetch(url, {
      method: 'PUT',
      headers: authHeaders(),
      body: JSON.stringify(schedule),
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return resp.json();
  },

  getLibrarianStatus: () =>
    apiFetch<{
      state: string;
      current_task: string;
      current_memory: { key: string; content_preview: string } | null;
      schedule: { next_window_start: string | null; window_duration_min: number };
      session_stats: {
        batch_started_at: string | null;
        memories_described_this_session: number;
        avg_seconds_per_memory: number | null;
      };
      lifetime_stats: { total_memories: number; described: number; pending: number };
      model: string;
      provider: string;
      error_message: string | null;
    }>('/api/librarian/status'),

  getHenryStatus: () =>
    apiFetch<{
      identity: { name: string; traits: string[]; tone: string };
      current_state: string;
      active_sessions: { id: string; name: string; started_at: string }[];
      current_tool: string | null;
      tasks_in_flight: number;
      recent_tasks: { id: string; description: string; status: string; tool_used: string | null; completed_at: string | null }[];
      today_totals: { messages_sent: number; tasks_dispatched: number; scheduled_fires: number; memories_formed: number };
      lifetime_stats: { total_memories: number; total_sessions: number; days_active: number; first_active: string | null };
      next_scheduled: { id: string; cron: string; currently_running: boolean } | null;
    }>('/api/henry/status'),

  // World View "Carved Cave" depth strata — real memory history sliced by time.
  // N is clamped to 1..6 by the daemon (the depth curve). Honest-empty on read
  // failure (zeros / empty slices), never throws an error wall.
  getWorldStrata: (slices: number) =>
    apiFetch<{
      total_memories: number;
      described_count: number;
      first_memory_at: string | null;
      slices: { start: string; end: string; memory_count: number; described_count: number }[];
    }>(`/api/world/strata?slices=${encodeURIComponent(String(slices))}`),

  getBrainMemories: (params: { q?: string; before?: string; before_id?: string; after?: string; offset?: number; limit?: number }) => {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.before) qs.set('before', params.before);
    if (params.before_id) qs.set('before_id', params.before_id);
    if (params.after) qs.set('after', params.after);
    if (params.offset !== undefined) qs.set('offset', String(params.offset));
    if (params.limit !== undefined) qs.set('limit', String(params.limit));
    return apiFetch<{
      memories: { id: string; key: string | null; text: string; description: string | null; ent: string[]; age: number; weight: number; timestamp: string }[];
      total: number;
      has_more: boolean;
    }>(`/api/brain/memories?${qs.toString()}`);
  },

  getAgents: () =>
    apiFetch<{
      agents: { id: string; name: string; role: string; source: string }[];
    }>('/api/agents'),

  runLibrarianNow: async () => {
    const url = `${API_BASE_URL}/api/librarian/run-now`;
    const resp = await fetch(url, {
      method: 'POST',
      headers: authHeaders(),
    });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(err.message || `HTTP ${resp.status}`);
    }
    return resp.json();
  },
};
