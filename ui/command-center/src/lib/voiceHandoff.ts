// Voice conversation handoff between the main window and the popped-out chat
// window.
//
// A live conversation's resources (mic stream, audio graph, WS) are bound to
// ONE window's JS context and cannot migrate. What CAN move is the
// conversation itself: the leaving window ends its hands-free session and
// posts a handoff ticket; the receiving window consumes it and starts
// hands-free bound to the SAME chat session — Henry continues where he was.
//
// Transport is localStorage (shared origin across both windows): writable
// synchronously in beforeunload, readable at mount (so a window created
// AFTER the ticket was posted still sees it — Tauri events would be lost),
// and `storage` events push it live to an already-open window.

const KEY = 'permagent-voice-handoff';
const FRESH_MS = 20_000;

export type VoiceHandoffTarget = 'chat' | 'main';

export const VOICE_HANDOFF_KEY = KEY;

export function requestVoiceHandoff(target: VoiceHandoffTarget): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ target, at: Date.now() }));
  } catch { /* private mode — conversation simply ends in the old window */ }
}

/** Consume a pending handoff addressed to `target`. Stale tickets (>20s, e.g.
 *  surviving an app relaunch) are discarded — never resurrect a mic session
 *  the user didn't just ask for. */
export function consumeVoiceHandoff(target: VoiceHandoffTarget): boolean {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return false;
    const t = JSON.parse(raw) as { target?: string; at?: number };
    if (typeof t.at !== 'number' || Date.now() - t.at > FRESH_MS) {
      localStorage.removeItem(KEY);
      return false;
    }
    if (t.target !== target) return false;
    localStorage.removeItem(KEY);
    return true;
  } catch {
    return false;
  }
}

// ── Live-conversation mirror ─────────────────────────────────────────────
// While a hands-free conversation runs anywhere, its owner heartbeats the
// current voice state here. Another window (the freshly popped-out chat)
// renders the orb in MIRROR mode from this — the user sees one continuous
// conversation while the audio finishes in the owning window and the real
// handoff happens underneath at turn end.

const LIVE_KEY = 'permagent-voice-live';
const END_KEY = 'permagent-voice-end';
const LIVE_FRESH_MS = 8_000;

export const VOICE_LIVE_KEY = LIVE_KEY;
export const VOICE_END_KEY = END_KEY;

export function publishLiveConversation(state: string): void {
  try {
    localStorage.setItem(LIVE_KEY, JSON.stringify({ state, at: Date.now() }));
  } catch { /* private mode */ }
}

export function clearLiveConversation(): void {
  try { localStorage.removeItem(LIVE_KEY); } catch { /* ignore */ }
}

export function readLiveConversation(): { state: string } | null {
  try {
    const raw = localStorage.getItem(LIVE_KEY);
    if (!raw) return null;
    const t = JSON.parse(raw) as { state?: string; at?: number };
    if (typeof t.at !== 'number' || Date.now() - t.at > LIVE_FRESH_MS) return null;
    return typeof t.state === 'string' ? { state: t.state } : null;
  } catch {
    return null;
  }
}

/** Ask whichever window owns the conversation to end it (mirror-orb click). */
export function requestVoiceEnd(): void {
  try { localStorage.setItem(END_KEY, String(Date.now())); } catch { /* ignore */ }
}
