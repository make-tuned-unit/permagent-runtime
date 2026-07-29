// Cross-window voice conversation signaling.
//
// ARCHITECTURE (2026-07-29, replaces the handoff design): the voice ENGINE —
// mic, VAD, playback, socket — lives in the MAIN window permanently and never
// migrates. The main window always exists, so the conversation physically
// cannot be interrupted by opening/closing the chat window. Every other
// surface (the popped-out chat window's orb) is a MIRROR: it renders from the
// live feed below and sends commands back.
//
// Transport is localStorage polling, NOT `storage` events — WKWebView does
// not reliably deliver storage events across separate webviews (the original
// handoff design died on exactly that), and a 150-400ms poll of a single key
// is free.

const LIVE_KEY = 'permagent-voice-live';
const END_KEY = 'permagent-voice-end';
const START_KEY = 'permagent-voice-start';
const INTERRUPT_KEY = 'permagent-voice-interrupt';
const CMD_FRESH_MS = 10_000;
/** The owner heartbeats every ~150ms; 2s staleness = owner gone. */
const LIVE_FRESH_MS = 2_000;

// ── Live feed (owner → mirrors) ─────────────────────────────────────────────

export interface LiveConversation {
  state: string;
  /** Smoothed 0..~1 audio level so mirror orbs dance with the real audio. */
  level: number;
}

export function publishLiveConversation(state: string, level: number): void {
  try {
    localStorage.setItem(LIVE_KEY, JSON.stringify({ state, level, at: Date.now() }));
  } catch { /* private mode */ }
}

export function clearLiveConversation(): void {
  try { localStorage.removeItem(LIVE_KEY); } catch { /* ignore */ }
}

export function readLiveConversation(): LiveConversation | null {
  try {
    const raw = localStorage.getItem(LIVE_KEY);
    if (!raw) return null;
    const t = JSON.parse(raw) as { state?: string; level?: number; at?: number };
    if (typeof t.at !== 'number' || Date.now() - t.at > LIVE_FRESH_MS) return null;
    if (typeof t.state !== 'string') return null;
    return { state: t.state, level: typeof t.level === 'number' ? t.level : 0 };
  } catch {
    return null;
  }
}

// ── Commands (mirrors → owner, polled by the owner) ─────────────────────────

function requestCmd(key: string): void {
  try { localStorage.setItem(key, String(Date.now())); } catch { /* ignore */ }
}

function consumeCmd(key: string): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return false;
    localStorage.removeItem(key);
    const at = Number(raw);
    return Number.isFinite(at) && Date.now() - at <= CMD_FRESH_MS;
  } catch {
    return false;
  }
}

/** Mirror-orb click: ask the owner to end the conversation. */
export function requestVoiceEnd(): void { requestCmd(END_KEY); }
export function consumeVoiceEnd(): boolean { return consumeCmd(END_KEY); }

/** Chat-window mic button: ask the owner to start a hands-free conversation. */
export function requestVoiceStart(): void { requestCmd(START_KEY); }
export function consumeVoiceStart(): boolean { return consumeCmd(START_KEY); }

/** Barge-in from a mirror surface: stop Henry mid-reply without ending the
 *  conversation. Without this, spacebar in the popped-out window swallowed the
 *  keystroke and did nothing while the tooltip promised an interrupt. */
export function requestVoiceInterrupt(): void { requestCmd(INTERRUPT_KEY); }
export function consumeVoiceInterrupt(): boolean { return consumeCmd(INTERRUPT_KEY); }
