/**
 * Permagent Command Center -- API client
 * Aligned with the actual permagentd endpoints.
 */

import type { ProjectDocument, ProjectNote, ProjectMemory } from '../components/projects/types';
import type { ActivityEventName } from './emitActivity';
import { getStreamToken } from './streamToken';
import { BROWSER_UNPAIRED, noteAuthenticatedSuccess, recentlyAuthenticated, setDaemonTokenError } from './daemonAuth';

/** How recently an authenticated success must have happened for a lone 401 to
 *  count as one route refusing, rather than the credential being dead. */
const AUTH_HEALTHY_WINDOW_MS = 30_000;

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const API_BASE_URL = (
  (import.meta.env.VITE_DAEMON_URL as string | undefined) ||
  (import.meta.env.VITE_API_BASE_URL as string | undefined) ||
  (isTauri ? 'http://127.0.0.1:3001' : '')
).replace(/\/$/, '');

export function getApiBaseUrl(): string { return API_BASE_URL; }

/** The daemon masks a secret as its first (up to) 8 characters plus asterisks —
 * 8 characters of a random key is a real disclosure on a shared screen. Show at
 * most the first 4 (enough to tell two keys apart, or read a vendor prefix). */
export function clampMaskedSecret<T extends string | null | undefined>(masked: T): T {
  if (typeof masked !== 'string') return masked;
  // Only the daemon's "prefix + asterisks" form; other shapes pass through.
  const m = /^([^*]+)(\*+)$/.exec(masked);
  if (!m) return masked;
  const lead = m[1].length;
  if (lead <= 4) return masked;
  return (masked.slice(0, 4) + '*'.repeat(lead - 4) + masked.slice(lead)) as T;
}

// Daemon Bearer token — loaded at runtime from Tauri IPC (not baked into the build).
let _daemonToken: string | null = null;
let _daemonTokenPromise: Promise<string | null> | null = null;

const BROWSER_TOKEN_KEY = 'permagent-daemon-token';

/**
 * A browser credential the daemon has stopped accepting.
 *
 * "No credential, no request" (see `requireCredential`) stopped the unpaired
 * World tab's 401 flood on 2026-09-19, but it only covers a browser holding NO
 * token. A browser still holding a revoked device token, or one minted under a
 * daemon secret that has since changed, has one — so every poll went out, came
 * back 401, re-read the SAME token from localStorage, retried, and 401'd again:
 * the same flood into daemon.err, doubled. Re-reading localStorage cannot fix a
 * dead token, so once the daemon has refused it `BROWSER_DEAD_TOKEN_401S` times
 * in a row with no success between, it is set aside here and the tab behaves as
 * unpaired until the stored credential CHANGES (a new pairing, here or in
 * another tab). Consecutive, not single: an older daemon 401s the one route it
 * does not have (/api/agent/identity) while every other route answers 200 with
 * the same token, and that must never lock a working tab out.
 */
let _rejectedBrowserToken: string | null = null;
let _browserConsecutive401s = 0;
export const BROWSER_DEAD_TOKEN_401S = 3;

const BROWSER_CREDENTIAL_REJECTED =
  'The daemon no longer accepts the credential this browser holds (401 on every request) — ' +
  'nothing more is sent until it is paired again. Open the app, or pair this device from Settings → Devices.';

/** The browser's credential, unless it is the one the daemon refused. */
function usableBrowserToken(token: string | null): string | null {
  if (token && token === _rejectedBrowserToken) return null;
  if (token && _rejectedBrowserToken) {
    // A different credential arrived — a re-pairing. Give it a clean slate.
    _rejectedBrowserToken = null;
    _browserConsecutive401s = 0;
  }
  return token;
}

/** Tally a browser request's auth outcome; returns true when `sentToken` has
 *  just been set aside as dead. The desktop app never lands here: its token
 *  comes over IPC and its 401 is what raises the credential banner. */
function noteBrowserAuthResult(status: number, sentToken: string | null): boolean {
  if (isTauri || !sentToken) return false;
  if (status !== 401) {
    if (status >= 200 && status < 300) _browserConsecutive401s = 0;
    return false;
  }
  _browserConsecutive401s += 1;
  if (_browserConsecutive401s < BROWSER_DEAD_TOKEN_401S) return false;
  _rejectedBrowserToken = sentToken;
  _daemonToken = null;
  _daemonTokenPromise = null;
  setDaemonTokenError(BROWSER_CREDENTIAL_REJECTED);
  return true;
}

/** Why the last `#claim=` pairing link failed, while no credential has arrived
 *  since. A failed claim used to fall back silently to "not paired", so a user
 *  who had just opened the pairing link Settings → Devices gave them saw the
 *  same message as one who never tried — with nothing saying the link itself
 *  was refused (used, expired) or never reached the daemon (a dev server that
 *  does not forward /pair answers 404). */
let _claimFailure: string | null = null;

function claimFailedMessage(reason: string): string {
  return (
    `The pairing link could not be redeemed (${reason} from POST /pair/claim) — this browser is still not paired, ` +
    'and nothing is sent until it is. Pairing links work once and expire: make a new one in Settings → Devices ' +
    'and open it in this browser.'
  );
}

/** The message for a browser with no usable credential: holding one the daemon
 *  refused, a pairing link that just failed, or never paired. */
function browserUnpairedMessage(): string {
  if (_rejectedBrowserToken) return BROWSER_CREDENTIAL_REJECTED;
  if (_claimFailure) return claimFailedMessage(_claimFailure);
  return BROWSER_UNPAIRED;
}

// Auth failure surface: `daemonAuth.ts` holds the published state, this module
// is what detects the failure. A dead agent must not look like a quiet agent,
// so every token failure is logged, retried, and published — never swallowed.

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
  let resp: Response;
  try {
    resp = await fetch(`${API_BASE_URL}/pair/claim`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code }),
    });
  } catch {
    _claimFailure = 'the daemon could not be reached';
    return storedBrowserToken();
  }
  try {
    if (!resp.ok) {
      _claimFailure = `HTTP ${resp.status}`;
      return localStorage.getItem(BROWSER_TOKEN_KEY);
    }
    const { token } = (await resp.json().catch(() => ({}))) as { token?: string };
    if (!token) {
      _claimFailure = 'no token in the reply';
      return localStorage.getItem(BROWSER_TOKEN_KEY);
    }
    _claimFailure = null;
    localStorage.setItem(BROWSER_TOKEN_KEY, token);
    reportDevicePaired(token);
    return token;
  } catch {
    return storedBrowserToken();
  }
}

function storedBrowserToken(): string | null {
  try {
    return localStorage.getItem(BROWSER_TOKEN_KEY);
  } catch {
    return null;
  }
}

/** What a pasted pairing link / code carries, or null when it carries neither. */
export function parsePairingInput(input: string): { claim: string } | { token: string } | null {
  const text = input.trim();
  if (!text) return null;
  const hashAt = text.indexOf('#');
  const frag = hashAt >= 0 ? text.slice(hashAt + 1) : text.includes('=') ? text : '';
  if (frag) {
    const params = new URLSearchParams(frag);
    const claim = params.get('claim');
    if (claim) return { claim };
    const token = params.get('token');
    if (token) return { token };
    return null;
  }
  // A bare claim code (32 hex characters today); the daemon is the judge.
  return /^[A-Za-z0-9_-]{8,}$/.test(text) ? { claim: text } : null;
}

/** True where this tab pairs itself (a browser); the desktop app's credential
 *  comes over IPC and has nothing to paste. */
export function canPairThisBrowser(): boolean {
  return !isTauri;
}

/**
 * Pair THIS tab from a pasted pairing link, `#claim=` / `#token=` fragment, or
 * bare claim code — redeemed against this page's own origin.
 *
 * Why it exists: Settings → Devices builds the pairing link on the hub origin
 * (`http://<hub>:3001/ui/#claim=…`), and a credential lives in the localStorage
 * of the origin that redeemed it. Opening that link pairs the hub origin, never
 * the localhost:5173 forum tab the user was actually looking at — which kept
 * showing "not paired" with no way out short of hand-editing its URL. Pasting
 * the same link here redeems the code through this origin (the dev server
 * forwards /pair to the daemon), so the credential lands where it is needed.
 * On success the tab's pollers and /events pick the credential up on their
 * next tick; on failure the published auth message says why.
 */
export async function pairThisBrowser(input: string): Promise<{ paired: boolean; message: string | null }> {
  if (isTauri) return { paired: false, message: 'The desktop app pairs itself — there is nothing to paste.' };
  const parsed = parsePairingInput(input);
  if (!parsed) {
    return {
      paired: false,
      message: 'That is not a pairing link. Copy the whole link from Settings → Devices (it contains #claim=).',
    };
  }
  let token: string | null;
  if ('token' in parsed) {
    const previous = storedBrowserToken();
    try { localStorage.setItem(BROWSER_TOKEN_KEY, parsed.token); } catch { /* the in-memory copy still pairs this tab */ }
    if (parsed.token !== previous) reportDevicePaired(parsed.token);
    _claimFailure = null;
    token = parsed.token;
  } else {
    const claimed = await exchangeClaim(parsed.claim);
    token = _claimFailure ? null : claimed;
  }
  const usable = usableBrowserToken(token);
  if (!usable) {
    const message = browserUnpairedMessage();
    setDaemonTokenError(message);
    return { paired: false, message };
  }
  _daemonToken = usable;
  _daemonTokenPromise = null;
  _browserConsecutive401s = 0;
  setDaemonTokenError(null);
  return { paired: true, message: null };
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
      _daemonTokenPromise = exchangeClaim(claim).then(claimed => {
        // A failed exchange falls back to the stored credential, which may be
        // the one the daemon already refused.
        const token = usableBrowserToken(claimed);
        _daemonToken = token;
        // A failed exchange must not be cached as "no token, forever": drop the
        // memo so the next caller can retry (or pick up a later pairing).
        if (!token) _daemonTokenPromise = null;
        setDaemonTokenError(token ? null : browserUnpairedMessage());
        return token;
      });
      return _daemonTokenPromise;
    }
    _daemonToken = usableBrowserToken(browserToken());
    setDaemonTokenError(_daemonToken ? null : browserUnpairedMessage());
    return Promise.resolve(_daemonToken);
  }
  if (!_daemonTokenPromise) {
    _daemonTokenPromise = import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<string>('get_daemon_token'))
      .then(token => {
        // An empty string is not a credential; treat it like a failure rather
        // than sending `Authorization: Bearer ` on every request.
        if (!token) throw new Error('get_daemon_token returned an empty token');
        _daemonToken = token;
        setDaemonTokenError(null);
        return token;
      })
      .catch((e: unknown) => {
        // NOT `.catch(() => null)`: clearing the memo is the whole point. A
        // resolved-null promise left in `_daemonTokenPromise` is an indefinite
        // unauthenticated state that no reconnect, poll or retry can escape.
        _daemonTokenPromise = null;
        setDaemonTokenError(
          `Could not read the daemon token — requests are unauthenticated (${
            e instanceof Error ? e.message : String(e)
          })`,
        );
        return null;
      });
  }
  return _daemonTokenPromise;
}

/** Forget the cached token so the next `loadDaemonToken` re-reads it. Used when
 *  the daemon rejects it (rotation, or a token read before the secret existed). */
function invalidateDaemonToken(): void {
  _daemonToken = null;
  _daemonTokenPromise = null;
}

// Kick off token loading immediately so it's ready when needed.
loadDaemonToken();

/** How often an unpaired browser re-reads its (local) credential while a
 *  stream waits for one. Only localStorage / the URL fragment is read — no
 *  request is made — so pairing in another tab is picked up within this. */
export const UNPAIRED_RECHECK_MS = 5_000;
let _browserCredentialWait: Promise<string> | null = null;

/** Resolves with the browser's daemon token, waiting (without any network
 *  traffic) until one exists. One shared wait, however many streams ask, so
 *  sockets whose owners unmounted meanwhile do not each keep a timer alive. */
function browserCredential(): Promise<string> {
  if (_daemonToken) return Promise.resolve(_daemonToken);
  if (!_browserCredentialWait) {
    _browserCredentialWait = new Promise<string>(resolve => {
      const check = () => {
        void loadDaemonToken().then(token => {
          if (token) {
            _browserCredentialWait = null;
            resolve(token);
          } else {
            setTimeout(check, UNPAIRED_RECHECK_MS);
          }
        });
      };
      check();
    });
  }
  return _browserCredentialWait;
}

/** URL for the global `/events` WebSocket, daemon token attached (`?token=`).
 *  The browser WebSocket API cannot set an Authorization header, so the token
 *  rides the query string — same contract as the `/voice` WS, validated
 *  server-side by the same fail-closed constant-time core (C1/C2 auth).
 *  Async: waits for the token (Tauri IPC on first call) so the first
 *  connection attempt is already authenticated.
 *
 *  In an unpaired browser this resolves only once a credential exists. A
 *  tokenless socket is refused by the daemon (401), and every caller's
 *  reconnect loop then retried it forever — the World page left open in
 *  Chrome put a /events 401 in daemon.err every few seconds. Holding the URL
 *  back keeps each caller's socket unopened with no change to the callers:
 *  they all re-check their own disposal after this await. */
export async function eventsWsUrl(): Promise<string> {
  if (!isTauri) await browserCredential();
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
  /** Saved model route used when restoring an existing chat session. */
  model_config?: { model_name: string } | null;
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
  // What the list needs to NAME and SORT a session (History's label, its
  // Conversations / Agent runs / System split, the project badge and goal
  // link). Optional: a daemon older than this projection omits them, and the
  // label falls back to the stored name (lib/sessionLabel.ts).
  working_dir?: string | null;
  schedule_id?: string | null;
  recipe_title?: string | null;
  /** The first user-visible user message, cut to a few hundred chars. */
  first_user_text?: string | null;
  goal_id?: string | null;
  goal_title?: string | null;
  goal_project_id?: string | null;
  project_id?: string | null;
  project_name?: string | null;
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

/** Parent + direct-child cost rollup from GET /api/sessions/{id}/cost. */
export interface SessionCostRollup {
  own: number;
  childrenTotal: number;
  perChild: Array<{ sessionId: string; costUsd: number }>;
}

export interface Skill {
  id: string;
  name: string;
  description: string;
  trigger_type?: string;
  trigger_config?: Record<string, unknown>;
  steps?: Array<{ action: string; tool?: string; description?: string }>;
  /** The tool this skill was learned from (`toolUsed` on GET /permagent/skills). */
  toolUsed?: string | null;
  usageCount?: number;
  last_run?: string;
  status?: string;
  version?: string;
  created_at?: string;
  updated_at?: string;
}

export interface PacksRoleRec {
  role: string;
  provider: string;
  model: string;
  display_name?: string;
  blended_cost_per_mtok?: number;
  reason?: string;
}

export interface PacksResponse {
  prompt: boolean;
  configured: Array<{ role: string; provider: string; model: string }>;
  recommendation: {
    recommendations: PacksRoleRec[];
    considered: string[];
  };
}

export interface PermagentConfig {
  /** YAML-file (+ defaults) config values. Env-blind by design — an env var
   *  can override any of these at the daemon without appearing here. */
  config?: Record<string, unknown>;
  /** GOOSE_MODE as the daemon actually resolves it (env var overrides YAML;
   *  backend ConfigResponse.effective_goose_mode). Snake_case mode value.
   *  Absent on daemons that predate the re-enable-gate epic part B. */
  effective_goose_mode?: string;
  /** Effective per-role routes as the daemon resolves them
   *  (backend ConfigResponse.resolved_routes). Keys: `chat`, `voice`, and —
   *  on daemons with the coordinator-gate projection — `harness`, which is the
   *  coordinator a coding-harness launch actually runs, after the gate. */
  resolved_routes?: Record<string, ResolvedModelRoute>;
  [key: string]: unknown;
}

/** One entry of `PermagentConfig.resolved_routes` (backend ResolvedModelRoute). */
export interface ResolvedModelRoute {
  provider: string;
  model: string;
  /** `configured` | `session_model` | `default` | `half_configured` |
   *  `disabled`, or for `harness` `coordinator_fallback`. */
  source: string;
  /** Plain-words sentence when the route is not simply the configured one. */
  notice?: string | null;
}

// ── Secret sources (GET/POST /config/secret-sources) ──
// Wire shapes mirror routes/config_management.rs (rename_all = camelCase).

/** One key's configured source. `resolves` is null for the built-in stores:
 *  listing sources deliberately does NOT open the keychain, because a settings
 *  list is not worth an authorization prompt. */
export interface SecretKeySource {
  key: string;
  /** 'keychain' | 'file' | 'onepassword' | 'bitwarden' | 'invalid' */
  kind: string;
  /** Display label, e.g. "macOS Keychain", "1Password". */
  label: string;
  /** op://… or bw://…, empty for the built-in stores. Never a secret. */
  reference: string;
  resolves?: boolean | null;
  error?: string | null;
}

/** Whether one password manager is present AND usable. Both halves matter: an
 *  installed-but-locked CLI fails every read, and showing it as ready is the
 *  false green light this feature exists to remove. */
export interface SecretBackendStatus {
  id: string;
  displayName: string;
  installed: boolean;
  signedIn: boolean;
  detail?: string | null;
}

export interface SecretSourcesResponse {
  /** Source for keys with no explicit entry. Normally 'keychain'. */
  defaultSource: string;
  /** ONLY keys with an explicit source. */
  keys: SecretKeySource[];
  backends: SecretBackendStatus[];
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

export interface BrowserHistoryEntry {
  url: string;
  title: string;
  lastVisited: string;
  visitCount: number;
}

export interface PermagentEvent {
  id: string;
  type: string;
  timestamp: string;
  payload: Record<string, unknown>;
}

// --- Fetch helper ---

/** Daemon routes that answer without a credential: the `public` router in
 *  crates/goose-server/src/routes/mod.rs (status, version, the loopback
 *  browser/desktop bridges, pairing). An unpaired browser may still call these;
 *  every other route is certain to 401 without a token. */
const PUBLIC_DAEMON_PATHS = [
  '/status',
  '/system_info',
  '/api/system_info',
  '/api/tailnet/status',
  '/api/tailnet/access',
  '/api/version',
  '/api/desktop',
  '/api/browser/navigate',
  '/api/browser/content',
  '/api/browser/snapshot',
  '/api/browser/act',
  '/pair/claim',
];

export function isPublicDaemonPath(endpoint: string): boolean {
  const path = endpoint.split(/[?#]/, 1)[0];
  return PUBLIC_DAEMON_PATHS.some(p => path === p || path.startsWith(`${p}/`));
}

/** The error a request gets when it is NOT sent because this browser holds no
 *  credential. Shaped like a daemon 401 (`status: 401`) so callers that map
 *  statuses keep working, but nothing went over the wire. */
function unpairedError(endpoint: string): Error & { status: number } {
  const err = new Error(`${browserUnpairedMessage()} (${endpoint} was not sent)`) as Error & { status: number };
  err.status = 401;
  return err;
}

/**
 * No credential, no request.
 *
 * An unpaired browser tab (the standalone World page at
 * localhost:5173/ui/forum.html, left open in Chrome) sent a steady ~73
 * requests a minute to the daemon — /api/agent/identity, /api/agents,
 * /permagent/skills, /api/goals/active, /api/henry/status — and every one was
 * a guaranteed 401 filling daemon.err. The token check below is local
 * (localStorage in a browser), so running it on every tick is free and the
 * first tick after pairing sends the real request. The desktop app never
 * short-circuits: its token comes over IPC, and a 401 there is the signal
 * the credential-failure banner is built on.
 */
async function requireCredential(endpoint: string): Promise<void> {
  if (!_daemonToken) await loadDaemonToken();
  if (_daemonToken || isTauri || isPublicDaemonPath(endpoint)) return;
  setDaemonTokenError(browserUnpairedMessage());
  throw unpairedError(endpoint);
}

/** `fetch` for callers that want the raw Response: the daemon token attached,
 *  and — for an unpaired browser — a local 401 Response instead of a request
 *  that could only come back 401 (see `requireCredential`). */
async function sendAuthed(endpoint: string, init?: RequestInit): Promise<Response> {
  try {
    await requireCredential(endpoint);
  } catch (e) {
    return new Response(JSON.stringify({ message: (e as Error).message }), {
      status: 401,
      headers: { 'Content-Type': 'application/json' },
    });
  }
  const sentToken = _daemonToken;
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(sentToken ? { Authorization: `Bearer ${sentToken}` } : {}),
    ...((init?.headers as Record<string, string> | undefined) ?? {}),
  };
  const response = await fetch(`${API_BASE_URL}${endpoint}`, { ...init, headers });
  // Raw-Response callers count toward the dead-credential tally too, or a tab
  // whose only pollers use fetchAuthed would never stop sending a dead token.
  noteBrowserAuthResult(response.status, sentToken);
  return response;
}

function authHeaders(): Record<string, string> {
  const h: Record<string, string> = { 'Content-Type': 'application/json' };
  if (_daemonToken) h['Authorization'] = `Bearer ${_daemonToken}`;
  return h;
}

/** What POST /config/check_provider proved about a provider's key. */
export interface ProviderCheckResult {
  live: boolean;
  model: string;
  models_available: number;
  note: string | null;
}

/** Durable Orchestrator triage of one recorded meeting (`meeting_triage`). */
export type MeetingTriageStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped';

export interface MeetingTriageReceipt {
  id: string;
  projectId: string;
  recordingId: string;
  noteId: string;
  status: MeetingTriageStatus;
  attemptCount: number;
  lastError?: string;
  /** local_only / trusted_pool / cloud_session / skipped */
  privacyRoute?: string;
  /** The dedicated hidden session the triage ran in, once it has one. */
  sessionId?: string;
  /** Kanban cards this triage materialized. */
  cardIds: string[];
  truncated: boolean;
  /** True when this save created the row (false on an idempotent replay). */
  created: boolean;
}

export async function apiFetch<T>(endpoint: string, options?: RequestInit): Promise<T> {
  return apiFetchOnce<T>(endpoint, options, true);
}

async function apiFetchOnce<T>(
  endpoint: string,
  options: RequestInit | undefined,
  mayRetryAuth: boolean,
): Promise<T> {
  // Ensure the daemon token is loaded before making any authenticated request,
  // and send nothing at all when an unpaired browser has none.
  await requireCredential(endpoint);
  const url = `${API_BASE_URL}${endpoint}`;
  const method = (options?.method ?? 'GET').toUpperCase();
  const sentToken = _daemonToken;
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

  // Any authenticated success clears the alarm. This is what keeps the banner
  // honest: an older daemon answers 401 for routes it simply does not have
  // (see ForumView on /api/agent/identity), and one such route must not leave a
  // permanent "you are not connected" sign up while everything else works.
  if (response.ok && _daemonToken) {
    noteAuthenticatedSuccess();
    setDaemonTokenError(null);
  }

  // A 401 is the daemon saying "I don't know you". Left unhandled it is exactly
  // the failure this module used to hide: the UI keeps rendering while nothing
  // it asks for is ever answered. Re-read the token once (it may have rotated,
  // or been read before the secrets file existed) and retry; if it still fails,
  // say so out loud instead of turning it into a per-caller "Unauthorized".
  const credentialDead = noteBrowserAuthResult(response.status, sentToken);
  if (response.status === 401) {
    const hadToken = sentToken !== null;
    invalidateDaemonToken();
    if (mayRetryAuth && hadToken && !credentialDead) {
      const refreshed = await loadDaemonToken();
      // In a browser the re-read is localStorage: the same token back means
      // the retry can only be refused the same way, so it is not sent. The
      // desktop app always retries — its IPC read can return a rotated token.
      if (refreshed && (isTauri || refreshed !== sentToken)) {
        return apiFetchOnce<T>(endpoint, options, false);
      }
    }
    // A credential that worked seconds ago on another route is not a dead
    // credential: this route is refusing it on its own account. Say so in the
    // log, but do not tell the user their agent cannot answer.
    if (isTauri && recentlyAuthenticated(AUTH_HEALTHY_WINDOW_MS)) {
      console.warn(`[api] ${endpoint} returned 401 while other authenticated requests succeed — route-specific, not a credential failure`);
    } else {
      setDaemonTokenError(
        isTauri
          ? `The daemon rejected this app's credentials (401 on ${endpoint}). ` +
            'The agent cannot answer until this is fixed.'
          : browserUnpairedMessage(),
      );
    }
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
 * Image digests carry the full OCR in `summary`. When `is_visual` is true the
 * Reader found too little text to ingest as the sole source — the caller should
 * still send the image so visual Q&A works, and fold any OCR that did land.
 */
export async function readerIngest(file: File): Promise<ReaderDigest> {
  await requireCredential('/api/reader/ingest');
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

/** Drop a bank/card statement onto the Finance tab. CSV/OFX/QFX parse
 *  directly; PDF/screenshots go through Reader OCR first. */
export async function uploadFinanceStatement(file: File): Promise<{
  inserted: number;
  parsed: number;
  sourceFile: string;
  ocrUsed: boolean;
}> {
  await requireCredential('/api/finance/statements');
  const form = new FormData();
  form.append('file', file, file.name);
  const headers: Record<string, string> = {};
  if (_daemonToken) headers['Authorization'] = `Bearer ${_daemonToken}`;
  const resp = await fetch(`${API_BASE_URL}/api/finance/statements`, {
    method: 'POST',
    headers,
    body: form,
  });
  if (!resp.ok) {
    const detail = await resp.text().catch(() => '');
    throw new Error(detail || `statement ingest HTTP ${resp.status}`);
  }
  return resp.json();
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

export interface PronunciationEntry {
  ipa: string;
  sounds_like: string;
}

export interface UnresolvedPronunciation {
  word: string;
  spelled_out_times: number;
}

/** A word the voice guessed at while speaking, waiting for review in Settings. */
export interface UnknownWord {
  word: string;
  display: string;
  count: number;
  first_seen_ms: number;
  last_seen_ms: number;
  sentence: string;
  substitute_ipa: string | null;
}

/**
 * Synthesize `text` in `voiceId` and return playable WAV audio.
 * Returns the Blob (not JSON) — used for per-voice preview, the picker
 * audition, and the spoken opening greeting. Throws (503) when the voice
 * assets aren't downloaded yet — callers gate on getVoiceModelStatus first.
 */
export async function synthesizeVoice(text: string, voiceId?: string | null): Promise<Blob> {
  await requireCredential('/voice/synthesize');
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
 * A message the daemon wrote about itself rather than an answer from the model.
 *
 * These ride as `systemNotification` blocks on an assistant frame and carry no
 * text block, so `extractText` returned "" and the whole frame rendered as
 * nothing. That is the silence the user hit: both cloud providers were out of
 * credit, the daemon said so on every turn, and the UI threw the explanation
 * away — the agent looked broken when it was actually reporting a billing
 * problem. A notice the user cannot see is the same as no notice at all.
 *
 * `thinkingMessage` is deliberately excluded: it is transient status the
 * streaming indicator already covers, and pinning it into the transcript would
 * leave "Thinking…" sitting under a finished answer forever.
 */
export type SystemNotice = { kind: 'inline' | 'creditsExhausted'; text: string };

export function extractSystemNotice(msg: DaemonMessage): SystemNotice | null {
  for (const c of msg.content) {
    if (c.type !== 'systemNotification') continue;
    const block = c as { notificationType?: unknown; msg?: unknown };
    const text = typeof block.msg === 'string' ? block.msg.trim() : '';
    if (!text) continue;
    if (block.notificationType === 'creditsExhausted') return { kind: 'creditsExhausted', text };
    if (block.notificationType === 'inlineMessage') return { kind: 'inline', text };
  }
  return null;
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
/** `GET`/`PUT /api/owner` — the human running this Permagent instance. */
export interface OwnerResponse {
  display_name: string | null;
  email: string | null;
  linked_person_id: string | null;
  /** The linked person's current display name, resolved server-side. `null`
   *  when unlinked, or when the link no longer resolves to a real person. */
  linked_person_name: string | null;
}

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

/** One daemon control-plane auth event (`/api/security/auth-log`) — a refused
 *  request, or an admitted one whose route can execute code, touch secrets,
 *  spend money or write data. See `crates/goose/src/security/auth_audit.rs`. */
export interface AuthEventEntry {
  id: string;
  ts: string;
  outcome: string;
  principal: string | null;
  credential: string;
  class: string;
  method: string;
  path: string;
  status: number | null;
  peer: string | null;
}

// ── Local inference (/local-inference/*) ──────────────────────────────
export type LocalModelDownloadStatus =
  | { state: 'NotDownloaded' }
  | { state: 'Downloading'; progress_percent: number; bytes_downloaded: number; total_bytes: number; speed_bps: number | null }
  | { state: 'Downloaded' };

/** One entry in the local-model registry — mirrors `LocalModelResponse`
 *  (crates/goose-server/src/routes/local_inference.rs). */
export interface LocalModelInfo {
  id: string;
  repo_id: string;
  filename: string;
  quantization: string;
  size_bytes: number;
  status: LocalModelDownloadStatus;
  recommended: boolean;
  vision_capable: boolean;
  mmproj_status: LocalModelDownloadStatus | null;
}

// ── Backups (/api/backups) ────────────────────────────────────────────
export interface SnapshotInfo {
  db: string;
  filename: string;
  timestamp: string;
  tier: 'Daily' | 'Weekly' | 'Monthly';
  size_bytes: number;
  integrity_ok: boolean;
}

export type BackupRunResult =
  | { status: 'ok'; db: string; filename: string; timestamp: string; tier: string; size_bytes: number; integrity_ok: boolean }
  | { status: 'error'; message: string }
  | { status: 'skipped'; reason: string };

export interface BackupRunResponse {
  brain: BackupRunResult;
  brain_graph: BackupRunResult;
  brain_recognition: BackupRunResult;
  spectral: BackupRunResult;
}

// ── Messaging gateways (/gateway/*) and the remote tunnel (/tunnel/*) ──
export interface PairedUserInfo {
  platform: string;
  user_id: string;
  display_name: string | null;
  session_id: string;
  paired_at: number;
}

export interface GatewayStatusInfo {
  gateway_type: string;
  running: boolean;
  configured: boolean;
  paired_users: PairedUserInfo[];
  info: Record<string, string>;
}

export type TunnelState = 'Idle' | 'Starting' | 'Running' | 'Error' | 'Disabled';

export interface TunnelInfo {
  state: TunnelState;
  url: string;
  hostname: string;
  secret: string;
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
/** An open failure incident (wave-1 triage surface; snake_case wire). */
export interface IncidentView {
  id: string;
  created_at: string;
  session_id: string | null;
  surface: string;
  user_goal: string;
  observation: string;
  mechanism: string;
  artifact_kind: string;
  artifact_ref: string;
  status: string;
  resolved_at: string | null;
}

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

/** Versioned read-time projection from Spectral. Numeric optionals are
 * intentionally nullable: null means the canonical source was unavailable or
 * invalid; numeric zero is an authoritative empty aggregate. */
export type ProjectionCompleteness = 'complete' | 'partial' | 'unknown';
export type ProjectionBand = 'ok' | 'soft' | 'gate' | 'hard' | 'unknown';

export interface BudgetCapTriplet {
  softUsd: number | null;
  gateUsd: number | null;
  hardUsd: number | null;
  source: string;
}

export interface BudgetScopeProjection {
  cap: BudgetCapTriplet;
  settledUsd: number | null;
  heldUsd: number | null;
  unknownUsd: number | null;
  effectiveUsedUsd: number | null;
  remainingUsd: number | null;
  band: ProjectionBand | null;
  completeness: ProjectionCompleteness;
  error: string | null;
}

export interface BillingEvidence {
  billingClass: string | null;
  provider: string | null;
  model: string | null;
  callId: string | null;
  isEstimated: boolean | null;
  observedAt: string | null;
  source: string;
}

export interface BudgetProjectionProvenance {
  version: 'budget-projection.v1';
  asOf: string;
  completeness: ProjectionCompleteness;
  sources: string[];
  error: string | null;
}

export interface BudgetProjection {
  taskId: string | null;
  rootSessionId: string;
  task: BudgetScopeProjection;
  session: BudgetScopeProjection;
  taskBilling: BillingEvidence;
  sessionBilling: BillingEvidence;
  provenance: BudgetProjectionProvenance;
}

export interface CouncilSeat {
  provider: string;
  display_name: string;
  model: string;
  configured: boolean;
  excluded: boolean;
  cli_or_acp: boolean;
}

export interface CouncilMembers {
  enabled: boolean;
  exclude: string[];
  seats: CouncilSeat[];
}

export interface CouncilReport {
  id: string;
  session_id: string;
  generated_at: string;
  headline: string;
  markdown: string;
  consensus: string[];
  dissent: unknown[];
  actions: unknown[];
  chair_provider: string | null;
  chair_model: string | null;
}

export interface CouncilSession {
  id: string;
  started_at: string;
  finished_at: string | null;
  trigger: string;
  extra_question: string | null;
  chair_provider: string | null;
  chair_model: string | null;
  brief_json: string;
  status: string;
  error: string | null;
}

export interface CouncilPosition {
  id: string;
  session_id: string;
  round: number;
  provider: string;
  model: string;
  status: string;
  raw_text: string | null;
  parsed_json: unknown;
  error: string | null;
}

export interface CouncilLatest {
  session: CouncilSession | null;
  report: CouncilReport | null;
  positions: CouncilPosition[];
  openActions: number;
}

/** CR1 (dagK): one row of `GET /api/council/reports` — a past Council
 *  session, and its report when the chair wrote one (a `failed` session may
 *  not have one). */
export interface CouncilReportsEntry {
  session: CouncilSession;
  report: CouncilReport | null;
}

export interface CouncilReportsResponse {
  sessions: CouncilReportsEntry[];
}

export interface HarnessCouncilRecommendation {
  recommended: boolean;
  reason: string;
  signals: string[];
}

export interface HarnessVerification {
  command: string;
  verdict: string | null;
}

export interface HarnessPendingGate {
  requestId: string;
  toolName: string;
  tier: string | null;
}

export interface HarnessRunView {
  runId: string;
  sessionId: string;
  project: string;
  promptTitle: string;
  promptDigest: string;
  taskVersion: string | null;
  envelopeId: string | null;
  promptContext: string | null;
  councilRecommendation: HarnessCouncilRecommendation;
  dagNodes: string[];
  dependencies: string[];
  activeNode: string | null;
  worker: string | null;
  provider: string | null;
  model: string | null;
  billingClass: string | null;
  tier: string | null;
  routingReason: string | null;
  status: 'queued' | 'running' | 'verifying' | 'waiting_gate' | 'succeeded' | 'failed' | 'cancelled';
  declaredVerification: HarnessVerification | null;
  lastVerification: HarnessVerification | null;
  verificationAttempts: number | null;
  verificationVerdict: string | null;
  pendingGate: HarnessPendingGate | null;
  retryCount: number | null;
  toolCalls: number | null;
  gateAttempts: number | null;
  evidence: string | null;
  result: string | null;
  parentRunId: string | null;
  /** Durable session identity; distinct from the optional run-id graph link. */
  parentSessionId: string | null;
  startedAt: string;
  updatedAt: string;
  elapsedMs: number;
  tokens: number | null;
  spendUsd: number | null;
  /** Canonical read-time budget projection; legacy scalar fields above are not
   * budget authority and must never fill this when it is unavailable. */
  budget: BudgetProjection;
}

// ── The Guard (per-project security findings) ───────────────────────────────
export type GuardStatus =
  | 'open' | 'reopened' | 'sent' | 'fix_committed' | 'not_seen' | 'fixed' | 'dismissed';

export interface GuardFinding {
  id: string;
  title: string;
  severity: string;
  cwe?: string | null;
  location?: string | null;
  remediation?: string | null;
  found_at: string;
  status?: GuardStatus;
  status_at?: string | null;
  first_seen_at?: string | null;
  last_seen_at?: string | null;
  sent_at?: string | null;
  sent_agent?: string | null;
  fix_commit?: string | null;
  fix_commit_subject?: string | null;
  fix_verified_at?: string | null;
  verify_detail?: string | null;
  dismissed_reason?: string | null;
}

export interface GuardCounts {
  active: number;
  open: number;
  sent: number;
  awaiting_rescan: number;
  unconfirmed: number;
  fixed: number;
  dismissed: number;
}

export interface GuardSummary {
  findings: GuardFinding[];
  counts: GuardCounts;
  last_scan: string | null;
  last_attempt: string | null;
  rescan_requested_at: string | null;
  enabled: boolean;
}

export interface GuardCheck {
  id: string;
  label: string;
  passed: boolean;
  detail: string;
}

export interface GuardVerifyResult {
  verified: boolean;
  finding: GuardFinding;
  detail: string;
  commit: string | null;
  checks: GuardCheck[];
  rescan_requested: boolean;
}

export interface GuardRescanResult {
  queued: boolean;
  requested_at: string | null;
  reason: string | null;
}

const guardPath = (projectId: string, finding?: string, action?: string) =>
  `/api/projects/${encodeURIComponent(projectId)}/guard`
  + (finding ? `/findings/${encodeURIComponent(finding)}/${action}` : '');

export const api = {
  // Health
  /** Does the hub answer? `/status` replies with the plain text `ok`, which
   *  apiFetch's JSON parse rejects with a SyntaxError — an answer, not an
   *  outage, so only a failure to get a response counts as "down". */
  getHealth: async (): Promise<boolean> => {
    try {
      await apiFetch<unknown>('/status');
      return true;
    } catch (e) {
      return e instanceof SyntaxError;
    }
  },

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
  // `?types=user,scheduled,sub_agent`: the daemon's default list (user +
  // scheduled) leaves out SubAgent sessions (goal and coding-harness
  // workers), so History's "Agent runs" tab silently starved without this —
  // classifySession()'s `sub_agent` branch could only ever fire on a `User`
  // session that happened to carry a goalId (REVIEW_CONFIRMED #11).
  getSessions: async (): Promise<SessionSummary[]> => {
    const t0 = performance.now();
    await requireCredential('/api/sessions');
    const res = await fetch(`${API_BASE_URL}/api/sessions?types=user,scheduled,sub_agent`, { headers: authHeaders() });
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

  /** Parent + child cost rollup — GET /api/sessions/{id}/cost */
  getSessionCost: (id: string) =>
    apiFetch<SessionCostRollup>(`/api/sessions/${encodeURIComponent(id)}/cost`),

  // Delete session — DELETE /api/sessions/{id}. apiFetch, not raw fetch: a raw
  // fetch RESOLVES on a 5xx, so the store treated a failed delete as success
  // and blanked the open conversation for a session the daemon still has.
  deleteSession: (id: string) =>
    apiFetch<void>(`/api/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }),

  // The ONE persistent, non-coding voice conversation shared by the World,
  // the Orb, the phone and the watch (decision D1). The daemon creates it on
  // first use and never returns a session that carries a recipe.
  getVoiceHomeSession: () =>
    apiFetch<{ session_id: string; name: string; created: boolean }>('/api/chat/current-session'),

  // The suite-wide CURRENT chat session (same session on the dock, Orb, World
  // voice, phone and watch). Setting/creating one emits
  // `current_chat_session_changed` on /events; other surfaces follow it.
  // set: 404 unknown id, 422 not a plain chat (coding/recipe/sub-agent/...).
  setCurrentChatSession: (sessionId: string, originClient = 'desktop') =>
    apiFetch<{ session_id: string; name: string; created: boolean }>('/api/chat/current-session', {
      method: 'POST',
      body: JSON.stringify({ session_id: sessionId, origin_client: originClient }),
    }),
  newCurrentChatSession: (originClient = 'desktop') =>
    apiFetch<{ session_id: string; name: string; created: boolean }>('/api/chat/current-session/new', {
      method: 'POST',
      body: JSON.stringify({ origin_client: originClient }),
    }),

  // The on-screen tap for a spoken side-effect (voice_confirm_requested card).
  // 404 = the confirm expired or is unknown; the caller should dismiss the card.
  resolveVoiceConfirm: (confirmId: string, sessionId: string, decision: 'approve' | 'deny') =>
    apiFetch<{ confirm_id: string; outcome: 'approved' | 'denied' }>(
      `/api/voice/confirm/${encodeURIComponent(confirmId)}`,
      { method: 'POST', body: JSON.stringify({ decision, session_id: sessionId }) },
    ),

  // Create session — POST /api/sessions. `projectId` is the CANONICAL project
  // id (`project:<slug>`) of the project open/known at creation time, so the
  // daemon can wing-scope memories written in this session. Omitted (not
  // `null`) when no project is open — never invented.
  createSession: (workingDir?: string, projectId?: string) =>
    apiFetch<Session>('/api/sessions', {
      method: 'POST',
      body: JSON.stringify({
        ...(workingDir ? { workingDir } : {}),
        ...(projectId ? { projectId } : {}),
      }),
    }),

  /**
   * Full-transcript search — GET /api/sessions/search. Unlike the client-side
   * filter over `SessionSummary` (label + first message only), this searches
   * every message in every session, so a session found only by something said
   * on turn 40 still turns up. History treats it as an enrichment: a session
   * id this returns is folded into the label-based match, never a replacement
   * for it, so a daemon that 500s (or an offline session) still searches.
   */
  searchSessions: (query: string, limit = 50) =>
    apiFetch<Session[]>(
      `/api/sessions/search?query=${encodeURIComponent(query)}&limit=${limit}`,
    ),

  /** Compact History header — GET /api/sessions/insights: total session count
   *  and total tokens across all of them. */
  getSessionInsights: async (): Promise<{ total_sessions: number; total_tokens: number }> => {
    // The daemon serialises SessionInsights as camelCase (totalSessions /
    // totalTokens); older builds and tests use snake_case. Accept both and
    // never hand the UI an undefined number.
    const r = await apiFetch<Record<string, unknown>>('/api/sessions/insights');
    const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
    return {
      total_sessions: num(r?.totalSessions ?? r?.total_sessions),
      total_tokens: num(r?.totalTokens ?? r?.total_tokens),
    };
  },

  /** Fork a session — POST /api/sessions/{id}/fork. `copy: true` duplicates the
   *  whole conversation into a new session id rather than truncating this one,
   *  which is what History's row-level "Fork" offers (a safe branch, not an
   *  edit of the original). */
  forkSession: (id: string) =>
    apiFetch<{ sessionId: string }>(`/api/sessions/${encodeURIComponent(id)}/fork`, {
      method: 'POST',
      body: JSON.stringify({ copy: true, truncate: false }),
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
    attachmentIds: string[] = [],
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
    if (attachmentIds.length > 0) {
      body.attachment_ids = attachmentIds;
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

  // Owner — the human running this Permagent instance (distinct from the
  // agent's own persona above). See useOwner.ts.
  getOwner: () => apiFetch<OwnerResponse>('/api/owner'),

  // The daemon's PUT response is the full fresh record (including the
  // resolved `linked_person_name`) — callers adopt it directly rather than
  // re-GETting, same contract as `putIdentity`.
  putOwner: (update: {
    display_name?: string | null; email?: string | null; linked_person_id?: string | null;
  }) => apiFetch<OwnerResponse>('/api/owner', {
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

  getPronunciations: () =>
    apiFetch<Record<string, PronunciationEntry>>('/voice/pronunciations'),

  getUnresolvedPronunciations: () =>
    apiFetch<{ unresolved: UnresolvedPronunciation[] }>('/voice/pronunciations/unresolved'),

  savePronunciation: (word: string, soundsLike: string) =>
    apiFetch<{ saved: boolean; total: number; ipa: string }>('/voice/pronunciations', {
      method: 'PUT',
      body: JSON.stringify({ word, sounds_like: soundsLike }),
    }),

  /** Words the voice had to guess at, most recently heard first. */
  getUnknownWords: () =>
    apiFetch<{ words: UnknownWord[]; max_words: number }>('/voice/unknown-words'),

  /** Dismiss one word from the review list. Teaching it (savePronunciation) also clears it. */
  dismissUnknownWord: (word: string) =>
    apiFetch<{ removed: boolean }>(`/voice/unknown-words/${encodeURIComponent(word)}`, {
      method: 'DELETE',
    }),

  deletePronunciation: (word: string) =>
    apiFetch<{ removed: boolean }>(`/voice/pronunciations/${encodeURIComponent(word)}`, {
      method: 'DELETE',
    }),

  // Config
  getConfig: (signal?: AbortSignal) => apiFetch<PermagentConfig>('/config', signal ? { signal } : undefined),

  /** Read-only connection status per integration (`gmail`, `slack`). Reports
   *  whether a token is stored — never the token. The daemon's Gmail connect
   *  flow is CLI-only today (`permagent integrations connect gmail`); the app
   *  only shows the precondition, it does not drive the OAuth dance. */
  getIntegrations: () =>
    apiFetch<Array<{ provider: string; connected: boolean; token_present: boolean }>>('/integrations'),

  /** Start the Gmail OAuth flow: exchanges a Google OAuth client id/secret for
   *  the URL to open in a browser. The daemon runs a short-lived local
   *  callback listener (port 8095) that completes the exchange when the user
   *  finishes consenting — this call only mints the URL. */
  gmailConnect: (clientId: string, clientSecret: string) =>
    apiFetch<{ auth_url: string; callback_port: number }>('/integrations/gmail/connect', {
      method: 'POST',
      body: JSON.stringify({ client_id: clientId, client_secret: clientSecret }),
    }),

  /** Forget the stored Gmail token. */
  gmailDisconnect: () =>
    apiFetch<void>('/integrations/gmail', { method: 'DELETE' }),

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
    }).then(r => (r && typeof r.maskedValue === 'string' ? { ...r, maskedValue: clampMaskedSecret(r.maskedValue) } : r)),

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

  /** Persist a complete role route in one guarded config-file write. */
  setModelRoute: (role: 'chat' | 'voice' | 'harness', provider: string, model: string) =>
    apiFetch<unknown>('/config/model-route', {
      method: 'POST',
      body: JSON.stringify({ role, provider, model }),
    }),

  getCouncilLatest: () =>
    apiFetch<CouncilLatest>('/api/council/latest'),

  /** CR1: recent finished Council sessions (newest first) for a "Past
   *  sessions" list — `/api/council/latest` only ever surfaces the newest. */
  getCouncilReports: (limit?: number) =>
    apiFetch<CouncilReportsResponse>(`/api/council/reports${limit ? `?limit=${limit}` : ''}`),

  getActiveHarnessRuns: () =>
    apiFetch<HarnessRunView[]>('/api/coding-sessions/harness-runs'),

  getHarnessRunHistory: () =>
    apiFetch<HarnessRunView[]>('/api/coding-sessions/harness-runs/history'),

  /** One explicit click approves one Council pass; it does not enable the
   *  recurring weekly Council setting. */
  conveneCouncil: (question: string, project?: string) =>
    apiFetch<{ accepted: boolean; message: string }>('/api/council/convene', {
      method: 'POST',
      body: JSON.stringify({ question, project, approved: true }),
    }),

  getCouncilMembers: () =>
    apiFetch<CouncilMembers>('/api/council/members'),

  putCouncilMembers: (exclude: string[]) =>
    apiFetch<CouncilMembers>('/api/council/members', {
      method: 'PUT',
      body: JSON.stringify({ exclude }),
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

  /** Recent daemon control-plane auth events — every refused request, and
   *  every admitted request whose route can execute code, touch secrets,
   *  spend money or write data. Detection, not prevention (see
   *  auth_audit.rs doc comment) — an empty log is not proof nothing ran. */
  getAuthLog: (limit = 100) =>
    apiFetch<AuthEventEntry[]>(`/api/security/auth-log?limit=${limit}`),

  // ── Briefings ─────────────────────────────────────────────────────────
  /** Mark briefings as seen ("ack" = seen, never approval). */
  ackBriefings: (ids: string[]) =>
    apiFetch<{ acknowledged: number }>('/api/henry/briefings/ack', {
      method: 'POST',
      body: JSON.stringify({ ids }),
    }),

  // ── Incidents (failure-learning triage) ───────────────────────────────
  /** Open incidents, newest first. */
  getIncidents: (limit = 50) =>
    apiFetch<IncidentView[]>(`/api/incidents?limit=${limit}`),

  /** Resolve an incident (idempotent — `changed:false` when already closed). */
  resolveIncident: (id: string) =>
    apiFetch<{ incident: IncidentView | null; changed: boolean }>(
      `/api/incidents/${id}/resolve`,
      { method: 'POST' },
    ),

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
        config_keys: Array<{
          name: string; required: boolean; secret: boolean; description?: string;
          default?: string | null; oauth_flow?: boolean; primary?: boolean;
        }>;
        /** The provider's own key-setup walkthrough (may contain a URL). */
        setup_steps?: string[];
        model_doc_link?: string;
      };
      is_configured: boolean;
      is_default?: boolean;
      provider_type: string;
    }>>('/config/providers'),

  getProviderModels: (name: string) =>
    apiFetch<string[]>(`/config/providers/${encodeURIComponent(name)}/models`),

  reloadConfig: () =>
    apiFetch<{ provider: string; keyTail: string }>('/config/reload', { method: 'POST' }),

  // ── Secret sources ────────────────────────────────────────────────────
  /** Where each secret is READ from, plus whether the password managers on this
   *  machine can actually answer. `keys` lists ONLY keys with an explicit
   *  source; everything else uses `defaultSource`. camelCase on the wire — see
   *  the `maskedValue` regression note above. */
  getSecretSources: () =>
    apiFetch<SecretSourcesResponse>('/config/secret-sources'),

  /** Point a key at a source, or pass `null` to return it to the default
   *  (normally the keychain). */
  setSecretSource: (key: string, source: string | null) =>
    apiFetch<SecretKeySource>('/config/secret-sources', {
      method: 'POST', body: JSON.stringify({ key, source }),
    }),

  /** Actually read a candidate source without saving it. "Reference saved" only
   *  proves a string reached config.yaml; this proves the reference RESOLVES.
   *  Always resolves (a failed test is a result, not an error). */
  testSecretSource: (key: string, source: string) =>
    apiFetch<{ ok: boolean; maskedValue?: string | null; error?: string | null }>(
      '/config/secret-sources/test',
      { method: 'POST', body: JSON.stringify({ key, source }) },
    ).then(r => (r && typeof r.maskedValue === 'string' ? { ...r, maskedValue: clampMaskedSecret(r.maskedValue) } : r)),

  /** Ship a finished coding-harness session's transcript tail; the daemon
   *  summarizes it and the Brain remembers what the user was building. */
  codingSessionSummary: (payload: {
    transcript: string; cwd?: string; command?: string; duration_secs?: number;
  }) =>
    apiFetch<{ stored: boolean; summary: string | null }>('/api/coding-sessions/summary', {
      method: 'POST', body: JSON.stringify(payload),
    }),

  /** A coding harness exited. Without a linked native goal and landed change,
   *  this callback leaves an untracked Grow action unchanged. */
  completeGrowthActionFromHarness: (projectId: string, actionId: string) =>
    apiFetch<{
      implemented: boolean;
      verified: boolean;
      identity: { id: string; status: string };
      reason: string | null;
    }>(
      `/api/projects/${encodeURIComponent(projectId)}/growth-actions/`
      + `${encodeURIComponent(actionId)}/complete-from-harness`,
      { method: 'POST' },
    ),

  setProvider: async (provider: string, model: string): Promise<void> => {
    await requireCredential('/config/set_provider');
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

  /** Validate a provider against stored config, optionally overlaying a typed
   *  key for validate-without-save. The overlay is not persisted; a stored
   *  keychain value still wins. Rejects when the provider cannot be built or
   *  refuses the key. Resolves with what was proved: `live` is true only when
   *  the key answered an authenticated model listing. An older daemon answers
   *  with an empty body, which resolves `undefined` (built, not proved live). */
  checkProvider: (provider: string, apiKey?: string): Promise<ProviderCheckResult | undefined> =>
    apiFetch<ProviderCheckResult | undefined>('/config/check_provider', {
      method: 'POST',
      body: JSON.stringify({
        provider,
        ...(apiKey ? { api_key: apiKey } : {}),
      }),
    }),

  /** Objective per-role recommendation + whether to prompt Apply. */
  getPacks: (): Promise<PacksResponse> =>
    apiFetch<PacksResponse>('/api/packs'),

  /** Persist the recommended role→model map (same triples as `permagent packs apply`). */
  applyPacks: (): Promise<{ applied: Array<{ role: string; provider: string; model: string }> }> =>
    apiFetch('/api/packs/apply', { method: 'POST' }),

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
    await requireCredential('/api/sessions/upload');
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
    sendAuthed(
      `/api/sessions/${encodeURIComponent(sessionId)}/attachments/${encodeURIComponent(attachmentId)}`,
      { method: 'DELETE' },
    ),

  // ── The Guard: per-project findings, fix loop ─────────────────────────────

  getGuard: (projectId: string) =>
    apiFetch<GuardSummary>(guardPath(projectId)),

  markGuardFindingSent: (projectId: string, findingId: string, agent: string) =>
    apiFetch<{ finding: GuardFinding }>(guardPath(projectId, findingId, 'sent'), {
      method: 'POST', body: JSON.stringify({ agent }),
    }),

  verifyGuardFinding: (projectId: string, findingId: string) =>
    apiFetch<GuardVerifyResult>(guardPath(projectId, findingId, 'verify'), { method: 'POST' }),

  setGuardFindingStatus: (
    projectId: string,
    findingId: string,
    body: { status: 'dismissed'; reason: string } | { status: 'open' },
  ) =>
    apiFetch<{ finding: GuardFinding }>(guardPath(projectId, findingId, 'status'), {
      method: 'POST', body: JSON.stringify(body),
    }),

  requestGuardRescan: (projectId: string) =>
    apiFetch<GuardRescanResult>(`${guardPath(projectId)}/rescan`, { method: 'POST' }),

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
    await requireCredential('/api/projects/documents');
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
    await requireCredential('/api/projects/documents');
    const response = await fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/documents/${encodeURIComponent(docId)}`,
      { headers: authHeaders() },
    );
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.blob();
  },

  fetchGrowMediaBlob: async (projectId: string, cardId: string, filename: string): Promise<Blob> => {
    await requireCredential('/api/projects/cards/media');
    const response = await fetch(
      `${API_BASE_URL}/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}/media/${encodeURIComponent(filename)}`,
      { headers: authHeaders() },
    );
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.blob();
  },

  /** Delete a project document (row + on-disk file). */
  deleteProjectDocument: (projectId: string, docId: string) =>
    sendAuthed(
      `/api/projects/${encodeURIComponent(projectId)}/documents/${encodeURIComponent(docId)}`,
      { method: 'DELETE' },
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
   *  (best-effort) and returns the saved note. A meeting note carries the
   *  recording's stable `recordingId`: the save is idempotent on it (a retry
   *  after a lost response returns the same note), and the response adds the
   *  durable triage receipt. Older daemons omit `triage`. */
  createProjectNote: (
    projectId: string,
    note: { title?: string; body: string; kind?: 'meeting'; recordingId?: string },
  ) =>
    apiFetch<ProjectNote & { triage?: MeetingTriageReceipt }>(
      `/api/projects/${encodeURIComponent(projectId)}/notes`,
      { method: 'POST', body: JSON.stringify(note) },
    ),

  /** The Orchestrator triage receipt for one recorded meeting (404 if none). */
  getMeetingTriage: (projectId: string, recordingId: string) =>
    apiFetch<MeetingTriageReceipt>(
      `/api/projects/${encodeURIComponent(projectId)}/recordings/${encodeURIComponent(recordingId)}/triage`,
    ),

  /** Re-queue a failed or skipped meeting triage. */
  retryMeetingTriage: (projectId: string, recordingId: string) =>
    apiFetch<MeetingTriageReceipt>(
      `/api/projects/${encodeURIComponent(projectId)}/recordings/${encodeURIComponent(recordingId)}/triage/retry`,
      { method: 'POST' },
    ),

  /** Delete a project note (and best-effort disassociate its Brain memory). */
  deleteProjectNote: (projectId: string, noteId: string) =>
    sendAuthed(
      `/api/projects/${encodeURIComponent(projectId)}/notes/${encodeURIComponent(noteId)}`,
      { method: 'DELETE' },
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
    await requireCredential('/api/projects/index-code');
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
    await requireCredential('/api/dictation/transcribe');
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

  getBrowserHistory: () =>
    apiFetch<{ entries: BrowserHistoryEntry[] }>('/api/browser/history'),

  recordBrowserHistory: (entry: BrowserHistoryEntry) =>
    apiFetch<{ entries: BrowserHistoryEntry[] }>('/api/browser/history', {
      method: 'POST',
      body: JSON.stringify(entry),
    }),

  // Downloads inbox (#392/#393) — files that landed in ~/.permagent/inbox via the
  // in-app browser download flow. Lists metadata rows newest-first. Recording a
  // row (POST) is driven by the desktop download bridge, not the UI.
  getInbox: () => apiFetch<InboxFile[]>('/api/inbox'),

  /** Fetch with daemon Bearer token auth (for /activity/* endpoints). */
  fetchAuthed: (endpoint: string, options?: RequestInit): Promise<Response> =>
    sendAuthed(endpoint, options),

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

  // ── Dev roots: where THIS user keeps their code ─────────────────

  /** Discovered (evidence-based: a `.git` was actually found) and already-
   *  confirmed code directories. Callers must keep the two apart — overwriting
   *  a confirmed answer with a fresh guess is the bug this endpoint exists to
   *  prevent. Written back via `upsertConfig('dev_roots', [...])`. */
  getDevRoots: () =>
    apiFetch<{ confirmed: string[]; discovered: string[]; home: string }>('/api/dev-roots'),

  /** Validate a hand-typed directory BEFORE confirming it. `dev_roots()` drops
   *  paths that aren't directories, so an unchecked typo would be silently
   *  discarded at read time — the original bug, re-entered through the fix. */
  checkDevRoot: (path: string) =>
    apiFetch<{ resolved: string; exists: boolean; has_repositories: boolean }>(
      `/api/dev-roots/check?path=${encodeURIComponent(path)}`,
    ),

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
      await requireCredential('/api/ollama/pull');
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
      let eventType = '';
      const readLine = (line: string) => {
        const trimmed = line.trim();
        if (!trimmed) { eventType = ''; return; }
        if (trimmed.startsWith('event:')) { eventType = trimmed.slice(6).trim(); return; }
        if (!trimmed.startsWith('data:')) return;
        const body = trimmed.slice(5).trim();
        let data: unknown;
        try { data = JSON.parse(body); }
        catch {
          if (eventType === 'error') throw new Error(body || 'Ollama download failed');
          return; // Ignore malformed progress, not download errors.
        }
        const progress = data && typeof data === 'object' ? data as { status?: unknown; error?: unknown; total?: number; completed?: number } : null;
        if (eventType === 'error' || typeof progress?.error === 'string') {
          throw new Error(typeof progress?.error === 'string' ? progress.error
            : eventType === 'error' && typeof data === 'string' ? data : 'Ollama download failed');
        }
        if (typeof progress?.status === 'string') onProgress(progress as { status: string; total?: number; completed?: number });
      };
      let finished = false;
      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split('\n');
          buffer = lines.pop() || '';
          for (const line of lines) readLine(line);
        }
        buffer += decoder.decode();
        for (const line of buffer.split('\n')) readLine(line);
        finished = true;
      } finally {
        // An error frame or a failing progress handler exits before EOF.
        // Close this client stream without suggesting the server pull stopped.
        if (!finished) { try { await reader.cancel(); } catch { /* preserve the original error */ } }
        try { reader.releaseLock(); } catch { /* preserve the original error */ }
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
    await requireCredential('/api/librarian/schedule');
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
      /** Entities the Librarian described but could not place with confidence
       *  — it needs the user to say who/what one of them is before it can
       *  describe them truthfully. `undefined` on a daemon that predates this
       *  field (treat the same as 0 — nothing to show, not "unknown"). */
      entities_awaiting_context?: number;
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

  searchBrain: (params: { q: string; since?: string; until?: string; source?: string; offset?: number; limit?: number }) => {
    const qs = new URLSearchParams();
    qs.set('q', params.q);
    if (params.since) qs.set('since', params.since);
    if (params.until) qs.set('until', params.until);
    if (params.source) qs.set('source', params.source);
    if (params.offset !== undefined) qs.set('offset', String(params.offset));
    if (params.limit !== undefined) qs.set('limit', String(params.limit));
    return apiFetch<{
      results: { source: string; id: string; preview: string; score: number; timestamp: string; session_id?: string | null }[];
      total: number;
      query: string;
      offset: number;
      limit: number;
      fts_count: number;
      spectral_count: number;
      dedup_count: number;
    }>(`/api/brain/search?${qs.toString()}`);
  },

  getBrainMemories: (params: { q?: string; before?: string; before_id?: string; after?: string; source?: string; offset?: number; limit?: number }) => {
    const qs = new URLSearchParams();
    if (params.q) qs.set('q', params.q);
    if (params.before) qs.set('before', params.before);
    if (params.before_id) qs.set('before_id', params.before_id);
    if (params.after) qs.set('after', params.after);
    if (params.source) qs.set('source', params.source);
    if (params.offset !== undefined) qs.set('offset', String(params.offset));
    if (params.limit !== undefined) qs.set('limit', String(params.limit));
    return apiFetch<{
      memories: { id: string; key: string | null; text: string; description: string | null; ent: string[]; age: number; weight: number; timestamp: string; source?: string | null; last_reinforced_at?: string | null }[];
      total: number;
      has_more: boolean;
    }>(`/api/brain/memories?${qs.toString()}`);
  },

  /** DELETE /api/brain/memory?key= — hard-delete a memory across every
   *  Spectral substrate (R5 "explicit forget"). `existed: false` means the
   *  key named nothing (already gone); `verified: false` with `existed: true`
   *  means the delete ran but a post-delete probe still found residue. The
   *  Brain list/panel's Forget action fires this once its undo toast expires
   *  unactioned — never on the click itself. */
  forgetBrainMemory: (key: string) =>
    apiFetch<{ existed: boolean; verified: boolean }>(`/api/brain/memory?key=${encodeURIComponent(key)}`, {
      method: 'DELETE',
    }),

  /** Per-source Brain-feed status (Settings → Memory's "What Henry learns
   *  from" panel) — count/last-fed/error/pending/dead for every structured
   *  data source, plus the chat-turn outbox's own depth. Never 404s: every
   *  source appears even with zero facts fed. */
  getBrainFeedStatus: () =>
    apiFetch<{
      sources: {
        source: string;
        tag: string;
        label: string;
        count: number;
        last_fed_at: string | null;
        last_error: string | null;
        last_error_at: string | null;
        pending: number;
        dead: number;
        enabled: boolean;
      }[];
      chat_outbox: { pending: number; dead: number };
    }>('/api/brain/feed-status'),

  getAgents: () =>
    apiFetch<{
      agents: { id: string; name: string; role: string; source: string }[];
    }>('/api/agents'),

  runLibrarianNow: async () => {
    await requireCredential('/api/librarian/run-now');
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

  // ── Local inference (/local-inference/*) — UI-less until now; see
  // crates/goose-server/src/routes/local_inference.rs. ─────────────────
  listLocalModels: () => apiFetch<LocalModelInfo[]>('/local-inference/models'),

  downloadLocalModel: (spec: string) =>
    apiFetch<string>('/local-inference/download', {
      method: 'POST',
      body: JSON.stringify({ spec }),
    }),

  cancelLocalModelDownload: (modelId: string) =>
    apiFetch<void>(`/local-inference/models/${encodeURIComponent(modelId)}/download`, {
      method: 'DELETE',
    }),

  deleteLocalModel: (modelId: string) =>
    apiFetch<void>(`/local-inference/models/${encodeURIComponent(modelId)}`, {
      method: 'DELETE',
    }),

  // ── Backups (/api/backups) — UI-less until now; see
  // crates/goose-server/src/routes/backup.rs. ───────────────────────────
  getBackups: () => apiFetch<{ snapshots: SnapshotInfo[] }>('/api/backups'),

  runBackups: () => apiFetch<BackupRunResponse>('/api/backups/run', { method: 'POST' }),

  // ── Messaging gateways (/gateway/*) — UI-less until now; see
  // crates/goose-server/src/routes/gateway.rs. ──────────────────────────
  getGatewayStatus: () => apiFetch<GatewayStatusInfo[]>('/gateway/status'),

  startGateway: (gatewayType: string, platformConfig: unknown, maxSessions = 0) =>
    apiFetch<void>('/gateway/start', {
      method: 'POST',
      body: JSON.stringify({ gateway_type: gatewayType, platform_config: platformConfig, max_sessions: maxSessions }),
    }),

  stopGateway: (gatewayType: string) =>
    apiFetch<void>('/gateway/stop', {
      method: 'POST',
      body: JSON.stringify({ gateway_type: gatewayType }),
    }),

  createGatewayPairingCode: (gatewayType: string) =>
    apiFetch<{ code: string; expires_at: number }>('/gateway/pair', {
      method: 'POST',
      body: JSON.stringify({ gateway_type: gatewayType }),
    }),

  unpairGatewayUser: (platform: string, userId: string) =>
    apiFetch<void>(`/gateway/pair/${encodeURIComponent(platform)}/${encodeURIComponent(userId)}`, {
      method: 'DELETE',
    }),

  // ── Remote tunnel (/tunnel/*) — the alternative to a tailnet for reaching
  // this hub; UI-less until now. See crates/goose-server/src/tunnel/mod.rs. ─
  getTunnelInfo: () => apiFetch<TunnelInfo>('/tunnel/status'),
};
