/**
 * Permagent Command Center -- API client
 * Aligned with the actual permagentd endpoints.
 */

import type { ProjectDocument, ProjectNote, ProjectMemory } from '../components/projects/types';
import type { ActivityEventName } from './emitActivity';
import { getStreamToken } from './streamToken';

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

const BROWSER_TOKEN_KEY = 'permagent-daemon-token';

/** UUID v4 that also works on insecure origins (a plain-HTTP tailnet page is
 *  not a secure context, so `crypto.randomUUID` may be absent there —
 *  `crypto.getRandomValues` is available everywhere). */
function uuidV4(): string {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40; // version 4
  b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
  const hex = Array.from(b, x => x.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Report genuine pairing completion to the daemon's authenticated
 *  `POST /activity/emit` (`devices_paired`, Always tier → a real Brain memory).
 *
 *  This runs on the *companion* device the moment it captures the hub token —
 *  the true completion seam. The token rides the URL's `#` fragment, which the
 *  browser never sends to the server, so the daemon has no server-side way to
 *  observe a pairing; this authenticated self-report is how it learns. It is
 *  self-proving: the request bears the just-captured token, so an invalid or
 *  rotated-away token is rejected by the auth middleware and can never mint
 *  the event. Fire-and-forget (pairing itself has already succeeded), and the
 *  body never includes the token. The hub-side "Copy" button emits only
 *  `pairing_link_copied` (Ephemeral) — intent, not completion. */
function reportDevicePaired(token: string): void {
  const eventType: ActivityEventName = 'devices_paired';
  try {
    fetch(`${API_BASE_URL}/activity/emit`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({
        event_id: uuidV4(),
        event_type: eventType,
        source_surface: 'settings',
        timestamp: new Date().toISOString(),
        payload: {},
        tier: 'always', // canonical; the daemon re-derives the tier server-side anyway
      }),
    }).catch(() => { /* best-effort awareness signal */ });
  } catch {
    /* best-effort awareness signal */
  }
}

/** #366: browser-context pairing. A device opening
 *  http://<hub>:3001/ui/#token=<daemon_token> captures the token once into
 *  localStorage (and scrubs it from the URL so it never lingers in history).
 *  Tauri keeps its IPC path; this is how phones/laptops on the tailnet pair. */
function browserToken(): string | null {
  try {
    const frag = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    const fromUrl = frag.get('token');
    if (fromUrl) {
      const previous = localStorage.getItem(BROWSER_TOKEN_KEY);
      localStorage.setItem(BROWSER_TOKEN_KEY, fromUrl);
      frag.delete('token');
      const rest = frag.toString();
      history.replaceState(null, '', window.location.pathname + window.location.search + (rest ? `#${rest}` : ''));
      // A pairing URL was actually opened here and its credential captured —
      // the real `devices_paired` moment. Only report a credential this device
      // did not already hold, so a history re-open of the same URL never
      // re-claims a pairing.
      if (fromUrl !== previous) reportDevicePaired(fromUrl);
    }
    return localStorage.getItem(BROWSER_TOKEN_KEY);
  } catch {
    return null;
  }
}

/** #628: claim-code pairing. A device opening
 *  http://<hub>:3001/ui/#claim=<code> carries a one-time claim code, not a
 *  token. The code is scrubbed from the URL immediately (same hygiene as
 *  `#token=`) and exchanged for this device's OWN bearer token via the public
 *  `POST /pair/claim` — so the pairing URL stops being a forever-secret: after
 *  one claim (or expiry) it is inert. */
function pendingClaimCode(): string | null {
  try {
    const frag = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    const code = frag.get('claim');
    if (!code) return null;
    frag.delete('claim');
    const rest = frag.toString();
    history.replaceState(null, '', window.location.pathname + window.location.search + (rest ? `#${rest}` : ''));
    return code;
  } catch {
    return null;
  }
}

/** Exchange a one-time claim code for a fresh per-device token. On success the
 *  token is persisted like a captured `#token=` credential and the genuine
 *  `devices_paired` completion is reported (the token is always freshly minted,
 *  so every successful claim IS a new pairing). On failure (expired/used code,
 *  hub unreachable) any previously stored credential is kept. */
async function exchangeClaim(code: string): Promise<string | null> {
  try {
    const resp = await fetch(`${API_BASE_URL}/pair/claim`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code }),
    });
    if (!resp.ok) return localStorage.getItem(BROWSER_TOKEN_KEY);
    const { token } = (await resp.json()) as { token?: string };
    if (!token) return localStorage.getItem(BROWSER_TOKEN_KEY);
    localStorage.setItem(BROWSER_TOKEN_KEY, token);
    reportDevicePaired(token);
    return token;
  } catch {
    try {
      return localStorage.getItem(BROWSER_TOKEN_KEY);
    } catch {
      return null;
    }
  }
}

export function loadDaemonToken(): Promise<string | null> {
  if (_daemonToken) return Promise.resolve(_daemonToken);
  if (!isTauri) {
    // A claim exchange already in flight (#628): every caller must await IT —
    // falling through to localStorage here would hand early callers a null
    // while the fresh token is milliseconds away.
    if (_daemonTokenPromise) return _daemonTokenPromise;
    // Claim-code pairing (#628) takes precedence: a `#claim=` fragment means
    // this load IS the pairing moment — exchange before falling back to any
    // stored / legacy `#token=` credential.
    const claim = typeof window !== 'undefined' ? pendingClaimCode() : null;
    if (claim) {
      _daemonTokenPromise = exchangeClaim(claim).then(token => {
        _daemonToken = token;
        return token;
      });
      return _daemonTokenPromise;
    }
    _daemonToken = browserToken();
    return Promise.resolve(_daemonToken);
  }
  if (!_daemonTokenPromise) {
    _daemonTokenPromise = import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<string>('get_daemon_token'))
      .then(token => { _daemonToken = token; return token; })
      .catch(() => null);
  }
  return _daemonTokenPromise;
}

// Kick off token loading immediately so it's ready when needed.
loadDaemonToken();

/** URL for the global `/events` WebSocket, daemon token attached (`?token=`).
 *  The browser WebSocket API cannot set an Authorization header, so the token
 *  rides the query string — same contract as the `/voice` WS, validated
 *  server-side by the same fail-closed constant-time core (C1/C2 auth).
 *  Async: waits for the token (Tauri IPC on first call) so the first
 *  connection attempt is already authenticated. */
export async function eventsWsUrl(): Promise<string> {
  const token = await getStreamToken();
  const base = getApiBaseUrl().replace(/^http/, 'ws');
  return token ? `${base}/events?token=${encodeURIComponent(token)}` : `${base}/events`;
}

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

/** Sent at the start of an SSE stream listing the session's in-flight
 *  request_ids so a (re)connecting client can reattach — the Stop button reads
 *  it to recover the cancel target after an EventSource reconnect mid-turn. */
export interface SSEActiveRequestsEvent {
  type: 'ActiveRequests';
  request_ids: string[];
}

export type SSEEvent = SSEMessageEvent | SSEErrorEvent | SSEFinishEvent | SSEActiveRequestsEvent | SSEPingEvent | { type: string; [key: string]: unknown };

/**
 * Live per-frame token + cost state. Rides every SSE MessageEvent frame
 * (`token_state`). NOTE: the daemon serializes this struct `rename_all =
 * "camelCase"`, so the fields are camelCase here (the enclosing `token_state`
 * key itself stays snake_case). Every cost figure is single-sourced from the
 * per-call cost ledger via the canonical `cost_of`.
 */
export interface TokenState {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  accumulatedInputTokens: number;
  accumulatedOutputTokens: number;
  accumulatedTotalTokens: number;
  /** Cost of the most recent provider turn (USD). */
  costUsd: number;
  /** Running session cost (USD) — the one authoritative number. */
  accumulatedCostUsd: number;
  /** Running dollars saved by cache reads — the "cache saved $X" signal. */
  cacheSavingsUsd: number;
  /** Percent of the context window in use, or null when the limit is unknown. */
  contextPercent: number | null;
  /** Active model name. */
  model: string;
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
  /** YAML-file (+ defaults) config values. Env-blind by design — an env var
   *  can override any of these at the daemon without appearing here. */
  config?: Record<string, unknown>;
  /** GOOSE_MODE as the daemon actually resolves it (env var overrides YAML;
   *  backend ConfigResponse.effective_goose_mode). Snake_case mode value.
   *  Absent on daemons that predate the re-enable-gate epic part B. */
  effective_goose_mode?: string;
  [key: string]: unknown;
}

/** Wire shape for POST/PUT /config/custom-providers (backend
 *  UpdateCustomProviderRequest). A custom provider is a user-defined
 *  OpenAI/Anthropic/Ollama-compatible endpoint not in the built-in catalog. */
export interface CustomProviderPayload {
  engine: 'openai_compatible' | 'anthropic_compatible' | 'ollama_compatible';
  display_name: string;
  api_url: string;
  api_key: string;
  models: string[];
  supports_streaming?: boolean;
  requires_auth?: boolean;
}

// ── Built-in browser bookmarks + saved tab sets (#790) ──
// Wire shapes mirror routes/browser_state.rs (rename_all = camelCase).

export interface BrowserBookmark {
  url: string;
  title: string;
  createdAt: string;
}

export interface BrowserSavedTab {
  url: string;
  title: string;
}

export interface BrowserTabSet {
  name: string;
  tabs: BrowserSavedTab[];
  createdAt: string;
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
  if (!_daemonToken) await loadDaemonToken();
  const url = `${API_BASE_URL}${endpoint}`;
  const method = (options?.method ?? 'GET').toUpperCase();
  let response: Response;
  try {
    response = await fetch(url, {
      ...options,
      headers: {
        ...authHeaders(),
        ...(options?.headers ?? {}),
      },
    });
  } catch (e) {
    // The request never left the client (bad URL, daemon down, CORS). Without
    // this line such failures are indistinguishable from a backend reject.
    console.error(`[api] ${method} ${endpoint} — request failed to send:`, e);
    throw e;
  }

  // Mutations are rare enough to log unconditionally; the daemon keeps no HTTP
  // access log, so this console line is the only request-level trace.
  if (method !== 'GET') {
    console.info(`[api] ${method} ${endpoint} → ${response.status}`);
  }

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: 'Unknown error' }));
    // Attach the HTTP status so callers can map specific failures (e.g. a 409
    // Conflict on goal cancel) to clear messages. Non-breaking: existing callers
    // read only `.message`.
    const err = new Error(
      error.message || error.error || `HTTP ${response.status}`,
    ) as Error & { status?: number };
    err.status = response.status;
    throw err;
  }

  // Some mutation routes (e.g. the #530 project-association POST/DELETE) return
  // a bare status code with an empty body; response.json() rejects on empty
  // input, which turned those 2xx successes into client-side "failures" (#561).
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
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
  if (!_daemonToken) await loadDaemonToken();
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
  if (!resp.ok) {
    // The daemon puts an honest, user-facing reason in the error body (#468):
    // e.g. "couldn't read this PDF cleanly — text extraction returned
    // unreadable text (likely a font-encoding issue)". Surface it so the
    // caller can show WHY the file was refused instead of a bare status code.
    let detail = '';
    try {
      detail = (await resp.text()).trim();
    } catch {
      /* body unavailable — fall through to the generic message */
    }
    throw new Error(detail || `reader ingest HTTP ${resp.status}`);
  }
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
  if (!_daemonToken) await loadDaemonToken();
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

/** Extract the text content from a DaemonMessage.
 *
 * Distinct text blocks within one stored message are distinct SEGMENTS — the
 * model spoke, used a tool, spoke again — so they join with a paragraph
 * break, never glued ("…works.Let me dig deeper…"). Streamed delta frames
 * carry a single block, so the separator is inert on the live path. */
export function extractText(msg: DaemonMessage): string {
  return msg.content
    .filter((c): c is TextContent => c.type === 'text')
    .map(c => c.text)
    .filter(t => t.length > 0)
    .join('\n\n');
}

/**
 * Extract the model's reasoning ("thinking") content. Extended-thinking models
 * stream `{ type: 'thinking', thinking: '…' }` blocks before the answer; the
 * daemon passes them through (message.rs MessageContent::Thinking). This is what
 * the reasoning-disclosure UI renders.
 */
export function extractThinking(msg: DaemonMessage): string {
  return msg.content
    .filter((c): c is { type: 'thinking'; thinking: string } =>
      c.type === 'thinking' && typeof (c as { thinking?: unknown }).thinking === 'string')
    .map(c => c.thinking)
    .filter(t => t.length > 0)
    .join('\n\n');
}

/** True when a frame carries tool activity — the boundary between one text
 *  segment and the next. Requests ride assistant frames; results come back on
 *  user-role frames. The streaming path uses this to owe the next text delta
 *  a paragraph break, so the live render matches the settled transcript
 *  instead of snapping into shape on Finish. */
export function hasToolActivity(msg: DaemonMessage): boolean {
  return msg.content.some(c =>
    c.type === 'toolRequest' || c.type === 'toolResponse' || c.type === 'toolConfirmationRequest');
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

/** One file in the Downloads inbox (metadata row; bytes live on disk).
 *  Mirrors `permagent::inbox::InboxFile` — serde serializes `Option<T>` as null. */
export interface InboxFile {
  id: string;
  filename: string;
  original_url: string | null;
  content_type: string | null;
  size_bytes: number | null;
  disk_path: string;
  status: string;
  project_id: string | null;
  created_at: string;
}

/** Sovereign-mode status (GET/POST /api/security/sovereignty). */
export interface SovereigntyStatus {
  /** When on, all cloud inference is blocked (fail-closed) and audited. */
  enabled: boolean;
  /** Whether the egress log captures full prompts (vs a hash only). */
  capturePrompts: boolean;
  /** Whether a local provider (`local`/`ollama`) is registered to serve work. */
  localProviderAvailable: boolean;
}

/**
 * Diagnostics consent (GET/POST /telemetry/consent) — two INDEPENDENT opt-ins
 * (#327 split), both off by default, explicit opt-in:
 *   - crashReportsConsented — crash-report sharing gate
 *     (crash_capture::crash_reports_consented, key `crash_reports_consent`)
 *   - analyticsConsented — product-analytics telemetry (`GOOSE_TELEMETRY_ENABLED`)
 * The Settings toggles render from these, never a hardcoded default (#845).
 */
export interface ConsentStatus {
  /** Whether the user has consented to sharing crash reports. */
  crashReportsConsented: boolean;
  /** Whether the user has consented to product-analytics telemetry. */
  analyticsConsented: boolean;
}

/** A locally-written, redacted crash-report export (#327 MVP). */
export interface CrashExportResponse {
  /** Absolute path of the redacted bundle written to local disk. */
  path: string;
  /** Number of captured crash reports included. */
  reportCount: number;
  /** The exact redacted bundle text (what would be shared), for preview. */
  content: string;
}

/** One row of the cloud-egress audit log (GET /api/security/egress-log). */
export interface EgressLogEntry {
  id: string;
  ts: string;
  provider: string;
  model: string;
  sessionId: string | null;
  projectId: string | null;
  kind: string;
  /** True if this cloud call was blocked by sovereign mode. */
  blocked: boolean;
  contentHash: string;
  prompt: string | null;
}

/** One paired companion device (#628). Mirrors the daemon's DeviceView —
 *  deliberately NO token material: tokens are shown once at claim time only. */
export interface DeviceInfo {
  id: string;
  name: string;
  /** RFC 3339 creation timestamp. */
  created: string;
  /** RFC 3339 timestamp of the last authenticated request, or null if never. */
  last_seen: string | null;
  revoked: boolean;
}

// ── Governance / spend ──────────────────────────────────────────────────
/** A budget ceiling triplet (soft warn ≤ gate/pause ≤ hard stop), USD. */
export interface Ceilings {
  soft: number;
  gate: number;
  hard: number;
}

/** The optional budget the cost-router enforces via the Decision Inbox. */
export interface BudgetView {
  /** Per-session ceiling — the "spend cap for this machine" the user sets. */
  session: Ceilings;
  /** Per-task ceiling (tighter, per unit of work). */
  task: Ceilings;
}

/** One session's spend (GET /api/governance/spend). */
export interface SessionSpend {
  id: string;
  name: string;
  workingDir: string;
  sessionType: string;
  costUsd: number;
  tokens: number;
  updatedAt: string;
  /** 'ok' | 'soft' | 'gate' | 'hard' — spend vs the session ceiling. */
  band: string;
}

/** Spend rolled up to a project (grouped by working directory). */
export interface ProjectSpend {
  path: string;
  label: string;
  costUsd: number;
  tokens: number;
  sessionCount: number;
}

/** The Spend panel's snapshot (GET /api/governance/spend). */
export interface SpendSnapshot {
  runningTotalUsd: number;
  totalTokens: number;
  sessionCount: number;
  budget: BudgetView;
  sessions: SessionSpend[];
  projects: ProjectSpend[];
}

/** A budget patch — omit a scope or a ceiling to leave it untouched. */
export interface BudgetPatch {
  session?: Partial<Ceilings>;
  task?: Partial<Ceilings>;
}

/** One worker in the roster (GET /api/agent/workers). */
export interface WorkerInfo {
  key: string;
  display_name: string;
  role: string;
  /** "internal_subagent" | "external_cli" | "pending". */
  engine: string;
  available: boolean;
  reason: string | null;
}

export const api = {
  // Health
  getHealth: () => apiFetch<{ status: string }>('/status'),

  // ── Device registry (#628): per-device pairing tokens ──────────────

  /** All paired devices (revoked included), newest first. Never contains
   *  token values. */
  listDevices: () => apiFetch<DeviceInfo[]>('/api/devices'),

  /** Mint a one-time claim code for a device named `name`. The pairing URL is
   *  `http://<hub>:3001/ui/#claim=<claim_code>`; the code is single-use and
   *  expires at `expires_at`. */
  pairDevice: (name: string) =>
    apiFetch<{ claim_code: string; expires_at: string }>('/api/devices/pair', {
      method: 'POST',
      body: JSON.stringify({ name }),
    }),

  /** Rename a paired device. */
  renameDevice: (id: string, name: string) =>
    apiFetch<DeviceInfo>(`/api/devices/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    }),

  /** Revoke a device's token — it stops authenticating immediately. */
  revokeDevice: (id: string) =>
    apiFetch<DeviceInfo>(`/api/devices/${encodeURIComponent(id)}/revoke`, {
      method: 'POST',
    }),

  // Sessions — GET /api/sessions returns { sessions: [...] } (lean SessionSummary)
  // #341 instrumentation: split the client-perceived cost into round-trip
  // (network + backend), body-download, and JSON.parse, plus payload size. The
  // lean projection (#341b) drops the heavy JSON blobs the consumer discarded —
  // watch `bytes` fall from ~700KB toward ~55KB for the same session count.
  getSessions: async (): Promise<SessionSummary[]> => {
    const t0 = performance.now();
    if (!_daemonToken) await loadDaemonToken();
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

  // Delete session — DELETE /api/sessions/{id}. apiFetch, not raw fetch: a raw
  // fetch RESOLVES on a 5xx, so the store treated a failed delete as success
  // and blanked the open conversation for a session the daemon still has.
  deleteSession: (id: string) =>
    apiFetch<void>(`/api/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
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
    // uuidV4, not crypto.randomUUID: companion devices reach the hub over
    // plain-HTTP tailnet URLs (insecure origin), where randomUUID is undefined
    // and a raw call would make every chat send throw.
    const requestId = uuidV4();
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

  /**
   * Interrupt an in-flight turn via POST /sessions/{id}/cancel. Always 200
   * with an HONEST body: `{cancelled:true}` means a live request's token was
   * cancelled and a terminal Finish { reason: "stop" } will follow on the SSE
   * channel (the UI settles on the normal stream path); `{cancelled:false}`
   * means there was nothing to cancel (stale/unknown request_id — the turn
   * already ended, or the daemon restarted), so NO terminal frame is coming
   * and the caller must reconcile streaming state itself. apiFetch still
   * throws on transport/server errors (agent may be alive — don't pretend it
   * stopped).
   */
  cancelReply: (sessionId: string, requestId: string) =>
    apiFetch<{ cancelled: boolean }>(`/sessions/${encodeURIComponent(sessionId)}/cancel`, {
      method: 'POST',
      body: JSON.stringify({ request_id: requestId }),
    }),

  /** Build the SSE URL for per-session events. `lastEventId` resumes the
   *  server replay after that sequence number (P1): EventSource cannot set the
   *  Last-Event-ID header on the manual close-and-reconnect the store's
   *  backoff loop performs, so the cursor rides a query param the daemon
   *  reads as its header-equivalent. The daemon token rides the query too
   *  (EventSource cannot set Authorization either — C1/C2 auth); async so the
   *  first connect is already authenticated once the token loads. */
  sessionEventsUrl: async (sessionId: string, lastEventId?: string | null): Promise<string> => {
    const token = await getStreamToken();
    const base = `${API_BASE_URL}/sessions/${encodeURIComponent(sessionId)}/events`;
    const params = new URLSearchParams();
    if (lastEventId) params.set('last_event_id', lastEventId);
    if (token) params.set('token', token);
    const qs = params.toString();
    return qs ? `${base}?${qs}` : base;
  },

  // Identity
  getIdentity: () => apiFetch<{
    first_name: string; last_name: string | null; nickname: string | null;
    display_name: string; traits: string[]; tone: string;
    opening_greeting: string; voice_id: string | null;
  }>('/api/agent/identity'),

  // The daemon's PUT response is the full fresh persona (IdentityResponse) —
  // typed fully so callers can adopt it directly instead of re-GETting (#167).
  putIdentity: (update: {
    first_name: string; last_name?: string | null; nickname?: string | null;
    traits: string[]; tone: string; opening_greeting: string; voice_id?: string | null;
  }) => apiFetch<{
    first_name: string; last_name: string | null; nickname: string | null;
    display_name: string; traits: string[]; tone: string;
    opening_greeting: string; voice_id: string | null;
  }>('/api/agent/identity', {
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

  /**
   * Read a NON-SECRET config value. The daemon answers with the **bare JSON
   * value** — `true`, `24`, `"anthropic"` — or `null` when the key is unset.
   *
   * There is no envelope. `ConfigValueResponse` is an untagged enum, so the
   * only shape carrying a `value` key is one nobody sends. Every call site that
   * reached for `.value` read `undefined`: that is why the Guard toggle saved
   * correctly and then read back as OFF forever, whatever the config said.
   * Secrets answer differently and have their own reader below — the two
   * shapes are separate functions precisely so this cannot be confused again.
   */
  readConfig: (key: string) =>
    apiFetch<unknown>('/config/read', {
      method: 'POST', body: JSON.stringify({ key, is_secret: false }),
    }),

  /** Read a SECRET config key. Answers `{"maskedValue": "…"}` — camelCase on
   *  the wire (`MaskedSecret` is `rename_all = "camelCase"`) — or `null` when
   *  unset. Never returns the secret itself. */
  readSecretConfig: (key: string) =>
    apiFetch<{ maskedValue?: string } | null>('/config/read', {
      method: 'POST', body: JSON.stringify({ key, is_secret: true }),
    }),

  // Extensions / MCP tools
  /**
   * Extension list for the Tools & MCPs panel.
   *
   * These fields are OPTIONAL on the wire and the types must say so. The Rust
   * side flattens ExtensionConfig, whose variants differ: `display_name` is an
   * `Option<String>` that the `stdio` variant omits entirely, and `bundled`
   * serializes as null. Declaring them required is what crashed the panel —
   * `ext.display_name[0]` on an absent field threw and took the app down, for
   * exactly the two stdio servers (Brave, Tavily) the user came to look at.
   */
  getExtensions: () => apiFetch<{
    extensions: Array<{
      enabled: boolean; type: string; name: string;
      description?: string; display_name?: string | null;
      bundled?: boolean | null; available_tools?: string[];
      /** Names (never values) of env vars the server needs, e.g. BRAVE_API_KEY. */
      env_keys?: string[];
    }>; warnings?: string[];
  }>('/config/extensions'),

  /** Actually start a configured extension and ask it for its tools. "Key
   *  saved" only proves a string reached the keychain; this proves the key
   *  WORKS. Always resolves (a failed probe is a result, not an error).
   *  camelCase on the wire — see the `maskedValue` regression. */
  probeExtension: (name: string) =>
    apiFetch<{ ok: boolean; toolCount: number; tools: string[]; error?: string | null }>(
      '/config/extensions/probe',
      { method: 'POST', body: JSON.stringify({ name }) },
    ),

  upsertConfig: (key: string, value: unknown, isSecret?: boolean) =>
    apiFetch<unknown>('/config/upsert', {
      method: 'POST',
      body: JSON.stringify({ key, value, is_secret: isSecret ?? false }),
    }),

  /** Delete a config key (secret keys are removed from the keychain). The
   *  upsert-empty-string trick this replaces left providers looking
   *  'Connected' with a dead key. */
  removeConfig: (key: string, isSecret = false) =>
    apiFetch<string>('/config/remove', {
      method: 'POST',
      body: JSON.stringify({ key, is_secret: isSecret }),
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

  // ── Sovereignty / data boundary ──────────────────────────────────────
  /** Current sovereign-mode status. */
  getSovereignty: () =>
    apiFetch<SovereigntyStatus>('/api/security/sovereignty'),

  /** Toggle sovereign mode and/or full-prompt capture; returns new status. */
  setSovereignty: (body: { enabled?: boolean; capturePrompts?: boolean }) =>
    apiFetch<SovereigntyStatus>('/api/security/sovereignty', {
      method: 'POST',
      body: JSON.stringify(body),
    }),

  /** Recent cloud-egress audit entries (everything that has left this machine). */
  getEgressLog: (limit = 100) =>
    apiFetch<EgressLogEntry[]>(`/api/security/egress-log?limit=${limit}`),

  // ── Governance / spend ────────────────────────────────────────────────
  /** Per-session + per-project spend, running total, and the current budget. */
  getSpend: (limit = 100) =>
    apiFetch<SpendSnapshot>(`/api/governance/spend?limit=${limit}`),

  /** The current optional budget ceilings. */
  getBudget: () => apiFetch<BudgetView>('/api/governance/budget'),

  /** Set the budget ceilings; returns the enforced (sanitized) budget. */
  setBudget: (patch: BudgetPatch) =>
    apiFetch<BudgetView>('/api/governance/budget', {
      method: 'POST',
      body: JSON.stringify(patch),
    }),

  /** The worker roster (per-role model/engine + live availability). */
  getWorkers: () =>
    apiFetch<Record<string, WorkerInfo>>('/api/agent/workers'),

  // ── Diagnostics consent (two independent opt-ins, #327) ───────────────
  /** Current crash-report + analytics consent (both off by default). */
  getCrashConsent: () =>
    apiFetch<ConsentStatus>('/telemetry/consent'),

  /** Set crash-report sharing consent; returns the authoritative values. */
  setCrashConsent: (consented: boolean) =>
    apiFetch<ConsentStatus>('/telemetry/consent', {
      method: 'POST',
      body: JSON.stringify({ crashReports: consented }),
    }),

  /** Set product-analytics telemetry consent; returns authoritative values. */
  setAnalyticsConsent: (consented: boolean) =>
    apiFetch<ConsentStatus>('/telemetry/consent', {
      method: 'POST',
      body: JSON.stringify({ analytics: consented }),
    }),

  /**
   * Produce a REDACTED crash-report bundle on local disk and return its path +
   * content (#327 MVP). No network upload — the user decides whether to attach
   * the file. Sovereign-safe (a local write is not egress).
   */
  exportCrashReport: () =>
    apiFetch<CrashExportResponse>('/telemetry/crash-report/export', {
      method: 'POST',
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

  /** Ship a finished coding-harness session's transcript tail; the daemon
   *  summarizes it and the Brain remembers what the user was building. */
  codingSessionSummary: (payload: {
    transcript: string; cwd?: string; command?: string; duration_secs?: number;
  }) =>
    apiFetch<{ stored: boolean; summary: string | null }>('/api/coding-sessions/summary', {
      method: 'POST', body: JSON.stringify(payload),
    }),

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

  /** Validate a provider against its current stored config: confirms the daemon
   *  can construct it (registered + required secrets present + well-formed URL /
   *  headers). Resolves on success; rejects with the daemon's reason on failure.
   *  Note: this is a config/constructibility check, not a guaranteed live API
   *  round-trip for every provider. POST /config/check_provider returns an empty
   *  200 body on success — apiFetch handles that (returns undefined). */
  checkProvider: (provider: string): Promise<void> =>
    apiFetch<void>('/config/check_provider', {
      method: 'POST',
      body: JSON.stringify({ provider }),
    }),

  /** Create a user-defined custom provider (writes a declarative provider
   *  definition + stores its API key, then hot-registers it). The new provider
   *  then appears in getProviders() tagged provider_type "Custom". Returns the
   *  generated provider name (id). */
  createCustomProvider: (payload: CustomProviderPayload): Promise<{ provider_name: string }> =>
    apiFetch<{ provider_name: string }>('/config/custom-providers', {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  /** Remove a user-defined custom provider by its id (the provider `name`). */
  removeCustomProvider: (id: string): Promise<string> =>
    apiFetch<string>(`/config/custom-providers/${encodeURIComponent(id)}`, {
      method: 'DELETE',
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

  // No silent `.catch(() => [])` here: a backend failure must reach the
  // component as an ERROR, not render as an empty "No executions yet" history.
  getSkillExecutions: (skillId: string) =>
    apiFetch<Array<{ id: string; status: string; started_at: string; completed_at?: string; error_message?: string }>>(
      `/permagent/skills/${encodeURIComponent(skillId)}/executions`
    ),

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

  // apiFetch, not raw fetch: a raw fetch resolved on a 5xx, so a failed layout
  // save looked identical to a successful one and the UI silently lied.
  updateWorkspaceLayout: (workspaceId: string, layoutJson: unknown) =>
    apiFetch<void>(`/api/workspaces/${encodeURIComponent(workspaceId)}/layout`, {
      method: 'PUT',
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
    if (!_daemonToken) await loadDaemonToken();
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

  // ── Project documents (#471 Layer 2) ──────────────────────────────────────

  /** List the documents attached to a project. */
  listProjectDocuments: (projectId: string) =>
    apiFetch<ProjectDocument[]>(`/api/projects/${encodeURIComponent(projectId)}/documents`),

  /** Upload one or more files into a project's document hub (multipart). */
  uploadProjectDocuments: async (
    projectId: string,
    files: File[],
  ): Promise<{ documents: ProjectDocument[] }> => {
    const form = new FormData();
    for (const file of files) form.append('files', file, file.name);
    if (!_daemonToken) await loadDaemonToken();
    const headers: Record<string, string> = {};
    if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;
    const response = await fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/documents`,
      { method: 'POST', headers, body: form },
    );
    if (!response.ok) {
      const err = await response.json().catch(() => ({ message: 'Upload failed' }));
      throw new Error(err.message || `HTTP ${response.status}`);
    }
    return response.json();
  },

  /** Fetch a document's bytes (authed) as a Blob — the viewer wraps it in an
   *  object URL so `<img>`/`<iframe>` render it without a token in the URL. */
  fetchProjectDocumentBlob: async (projectId: string, docId: string): Promise<Blob> => {
    if (!_daemonToken) await loadDaemonToken();
    const response = await fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/documents/${encodeURIComponent(docId)}`,
      { headers: authHeaders() },
    );
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.blob();
  },

  /** Delete a project document (row + on-disk file). */
  deleteProjectDocument: (projectId: string, docId: string) =>
    fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/documents/${encodeURIComponent(docId)}`,
      { method: 'DELETE', headers: authHeaders() },
    ),

  // ── Project memories (the Brain-loop read side, #587-adjacent) ─────────────

  /** List the Brain memories associated with a project, resolved from the LIVE
   *  Brain, newest association first. This is the read-back the Memories panel
   *  and the panels' "View in Brain" links resolve their keys against — every
   *  note / document / code-index write auto-associates, so they all appear. */
  listProjectMemories: (projectId: string) =>
    apiFetch<ProjectMemory[]>(`/api/projects/${encodeURIComponent(projectId)}/memories`),

  // ── Project notes (freeform notes indexed into the Brain) ──────────────────

  /** List a project's notes, newest first. */
  listProjectNotes: (projectId: string) =>
    apiFetch<ProjectNote[]>(`/api/projects/${encodeURIComponent(projectId)}/notes`),

  /** Create a note on a project. The backend indexes its text into the Brain
   *  (best-effort) and returns the saved note. */
  createProjectNote: (projectId: string, note: { title?: string; body: string; kind?: 'meeting' }) =>
    apiFetch<ProjectNote>(`/api/projects/${encodeURIComponent(projectId)}/notes`, {
      method: 'POST',
      body: JSON.stringify(note),
    }),

  /** Delete a project note (and best-effort disassociate its Brain memory). */
  deleteProjectNote: (projectId: string, noteId: string) =>
    fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/notes/${encodeURIComponent(noteId)}`,
      { method: 'DELETE', headers: authHeaders() },
    ),

  // ── Project code index (parse a codebase into the Brain, #471) ─────────────

  /** Index a project's codebase into the Brain: the backend parses the code at
   *  the project's root_path into a durable, project-scoped code map memory (the
   *  "code understanding" keystone). Returns a small summary. On failure the
   *  backend answers with a plain-text reason (no root_path, unreadable path,
   *  Brain unavailable) — surfaced here as the thrown error message. */
  indexProjectCode: async (
    projectId: string,
  ): Promise<{ indexed: boolean; files: number; memoryKey: string }> => {
    if (!_daemonToken) await loadDaemonToken();
    const response = await fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/index-code`,
      { method: 'POST', headers: authHeaders() },
    );
    if (!response.ok) {
      const msg = await response.text().catch(() => '');
      throw new Error(msg || `HTTP ${response.status}`);
    }
    return response.json();
  },

  /** Install the bundled dictation model on first run: the daemon copies the
   *  bundled Whisper model into the data dir (once) and points
   *  LOCAL_WHISPER_MODEL at it. Idempotent. `status` is "ready" once dictation
   *  works, or "unavailable" when no bundle is present (e.g. a dev build). */
  provisionDictationModel: (bundledPath: string) =>
    apiFetch<{ status: string; model: string | null }>('/api/dictation/provision', {
      method: 'POST',
      body: JSON.stringify({ bundled_path: bundledPath }),
    }),

  /** Transcribe a dictated audio clip (WAV) to text locally, via the on-device
   *  Whisper model. A 503 means dictation isn't set up on this install — the
   *  caller should surface that as a setup prompt, not a hard error. */
  transcribeAudio: async (audio: Blob): Promise<{ text: string }> => {
    if (!_daemonToken) await loadDaemonToken();
    const form = new FormData();
    form.append('file', audio, 'dictation.wav');
    // Only Authorization — NOT Content-Type: the browser must add the multipart
    // boundary itself (mirrors readerIngest).
    const headers: Record<string, string> = {};
    if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;
    const resp = await fetch(`${API_BASE_URL}/api/dictation/transcribe`, {
      method: 'POST',
      headers,
      body: form,
    });
    if (!resp.ok) {
      const detail = await resp.text().catch(() => '');
      if (resp.status === 503) {
        throw new Error(detail || 'Dictation is not set up on this install');
      }
      throw new Error(detail || `transcribe HTTP ${resp.status}`);
    }
    return resp.json() as Promise<{ text: string }>;
  },

  // ── Built-in browser bookmarks + saved tab sets (#790) ──────────────
  // Daemon-persisted (JSON state files, routes/browser_state.rs) so they
  // survive app restarts. Full-replace PUT: the UI owns list edits and
  // persists the whole list each time (mirrors the dashboard-layout PUT).

  getBrowserBookmarks: () =>
    apiFetch<{ bookmarks: BrowserBookmark[] }>('/api/browser/bookmarks'),

  putBrowserBookmarks: (bookmarks: BrowserBookmark[]) =>
    apiFetch<{ bookmarks: BrowserBookmark[] }>('/api/browser/bookmarks', {
      method: 'PUT',
      body: JSON.stringify({ bookmarks }),
    }),

  getBrowserTabSets: () =>
    apiFetch<{ tabSets: BrowserTabSet[] }>('/api/browser/tab-sets'),

  putBrowserTabSets: (tabSets: BrowserTabSet[]) =>
    apiFetch<{ tabSets: BrowserTabSet[] }>('/api/browser/tab-sets', {
      method: 'PUT',
      body: JSON.stringify({ tabSets }),
    }),

  // Downloads inbox (#392/#393) — files that landed in ~/.permagent/inbox via the
  // in-app browser download flow. Lists metadata rows newest-first. Recording a
  // row (POST) is driven by the desktop download bridge, not the UI.
  getInbox: () => apiFetch<InboxFile[]>('/api/inbox'),

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

  // ── System info (#381: wizard hardware scan) ────────────────────

  getSystemInfo: () =>
    apiFetch<{
      app_version: string;
      os: string;
      os_version: string;
      architecture: string;
      provider: string | null;
      model: string | null;
      enabled_extensions: string[];
      total_ram_bytes: number | null;
      cpu_brand: string | null;
      disk_free_bytes: number | null;
    }>('/api/system_info'),

  // ── Ollama + Librarian ──────────────────────────────────────────

  /** Pull an Ollama model with SSE progress streaming (#381). Each event's
   *  data is one NDJSON progress line from Ollama:
   *  {"status":"pulling …","digest":"sha256:…","total":N,"completed":N} */
  pullOllamaModel: (
    model: string,
    onProgress: (data: { status: string; total?: number; completed?: number }) => void,
  ) => {
    const url = `${API_BASE_URL}/api/ollama/pull`;
    const controller = new AbortController();
    const promise = (async () => {
      if (!_daemonToken) await loadDaemonToken();
      const resp = await fetch(url, {
        method: 'POST',
        headers: authHeaders(),
        body: JSON.stringify({ model }),
        signal: controller.signal,
      });
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      const reader = resp.body?.getReader();
      if (!reader) throw new Error('No response body');
      const decoder = new TextDecoder();
      let buffer = '';
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          const trimmed = line.replace(/^data:\s*/, '').trim();
          if (!trimmed) continue;
          try { onProgress(JSON.parse(trimmed)); } catch { /* skip non-JSON */ }
        }
      }
      if (buffer.trim()) {
        const trimmed = buffer.replace(/^data:\s*/, '').trim();
        try { onProgress(JSON.parse(trimmed)); } catch { /* skip non-JSON */ }
      }
    })();
    return { promise, abort: () => controller.abort() };
  },

  /** Best-effort auto-start of a locally installed Ollama (#381 one-click). */
  startOllama: () =>
    apiFetch<{ launched: boolean; method: string | null }>('/api/ollama/start', {
      method: 'POST',
      body: JSON.stringify({}),
    }),

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
      pruning_enabled?: boolean;
    }>('/api/librarian/schedule'),

  setLibrarianSchedule: async (schedule: {
    enabled: boolean;
    start_time: string;
    duration_minutes: number;
    model: string;
    run_if_launched_in_window: boolean;
    pruning_enabled?: boolean;
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

  getLibrarianRunStatus: () =>
    apiFetch<{
      running: boolean;
      started_at: string | null;
      finished_at: string | null;
      described: number | null;
      last_error: string | null;
    }>('/api/librarian/run-status'),

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
    if (resp.status === 409) {
      return { status: 'already_running' as const };
    }
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(err.message || `HTTP ${resp.status}`);
    }
    return resp.json() as Promise<{ status: 'started' }>;
  },
};
