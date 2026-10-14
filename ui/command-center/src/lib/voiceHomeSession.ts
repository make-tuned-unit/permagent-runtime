// The voice-home session: the single persistent, non-coding conversation every
// voice surface (World, Orb, phone, watch) speaks into. Voice must never ride
// whatever chat session is sticky in the dock — that was how a spoken question
// landed in an Auto-mode coding coordinator (V-10).
//
// Resolution: ask the daemon (`GET /api/voice/home-session`). If the daemon
// can't answer, fall back to a fresh PLAIN chat session (never a coding one),
// remembered locally so the fallback is stable until the daemon answers again.

import { api } from './api';

const FALLBACK_KEY = 'permagent-voice-fallback-session-id';

let cached: string | null = null;
let inflight: Promise<string> | null = null;

function readFallback(): string | null {
  try { return localStorage.getItem(FALLBACK_KEY); } catch { return null; }
}
function writeFallback(id: string | null): void {
  try {
    if (id) localStorage.setItem(FALLBACK_KEY, id);
    else localStorage.removeItem(FALLBACK_KEY);
  } catch { /* storage unavailable: the in-memory cache still holds */ }
}

async function resolveOnce(): Promise<string> {
  try {
    const home = await api.getVoiceHomeSession();
    if (home?.session_id) {
      writeFallback(null);
      return home.session_id;
    }
  } catch { /* fall through to the plain-chat fallback */ }
  const remembered = readFallback();
  if (remembered) return remembered;
  const fresh = await api.createSession();
  writeFallback(fresh.id);
  return fresh.id;
}

/** Resolve (and cache) the voice-home session id. Rejects only if both the
 *  daemon route and the fresh-chat fallback fail; callers retry later. */
export function resolveVoiceHomeSession(): Promise<string> {
  if (cached) return Promise.resolve(cached);
  if (!inflight) {
    inflight = resolveOnce()
      .then(id => { cached = id; return id; })
      .finally(() => { inflight = null; });
  }
  return inflight;
}

/** Forget the cache (a deleted voice session, or tests). */
export function resetVoiceHomeSession(): void {
  cached = null;
  inflight = null;
}

/** The suite-wide current chat session (voice follows it too). Same resolver:
 *  daemon first, fresh plain chat as the fallback. */
export const resolveCurrentChatSession = resolveVoiceHomeSession;

/** A `current_chat_session_changed` frame (or a local change) updates the cache
 *  so the next resolve returns the new id without a round trip. */
export function setCachedCurrentChatSession(id: string): void {
  cached = id;
}
