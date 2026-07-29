// Speak-replies (#18) — the agent talks, you can mute.
//
// When enabled, each completed assistant reply is synthesized through the
// local Kokoro primitive (POST /voice/synthesize — the W4 seam) and played.
// Voice-first onboarding turns this ON when a walkthrough starts, with the
// mute toggle in the chat header dropping it back to text; the preference
// persists. Playback is best-effort and never blocks the text path: the reply
// is already on screen before the first audio byte exists.

import { useSyncExternalStore } from 'react';
import { synthesizeVoice } from './api';

const LS_KEY = 'permagent-speak-replies';

let enabled = ((): boolean => {
  try { return localStorage.getItem(LS_KEY) === '1'; } catch { return false; }
})();
const subscribers = new Set<() => void>();
let currentAudio: HTMLAudioElement | null = null;
/** Monotonic token — a mute or a newer reply invalidates in-flight synthesis. */
let playSeq = 0;

export function useSpeakReplies(): boolean {
  return useSyncExternalStore(
    (fn) => { subscribers.add(fn); return () => subscribers.delete(fn); },
    () => enabled,
  );
}

export function setSpeakReplies(on: boolean): void {
  enabled = on;
  try { localStorage.setItem(LS_KEY, on ? '1' : '0'); } catch { /* ephemeral */ }
  if (!on) stopSpeaking();
  subscribers.forEach((fn) => fn());
}

export function stopSpeaking(): void {
  playSeq += 1;
  if (currentAudio) {
    currentAudio.pause();
    // Revoke BEFORE clearing src: blanking src doesn't fire 'ended', so the
    // onended revoke never runs on this path and every superseded/muted reply
    // leaked its blob for the life of the window.
    const url = currentAudio.src;
    currentAudio.src = '';
    if (url.startsWith('blob:')) URL.revokeObjectURL(url);
    currentAudio = null;
  }
}

/** Markdown → speakable prose: drop code (never read code aloud), links to
 *  their labels, heading/emphasis markers, then cap at a sentence boundary. */
export function speakableText(markdown: string, cap = 700): string {
  let t = markdown
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`[^`]*`/g, ' ')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, ' ')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^#{1,6}\s+/gm, '')
    .replace(/[*_>#|-]{2,}/g, ' ')
    .replace(/[*_]/g, '')
    .replace(/https?:\/\/\S+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (t.length > cap) {
    const cut = t.slice(0, cap);
    const lastStop = Math.max(cut.lastIndexOf('. '), cut.lastIndexOf('! '), cut.lastIndexOf('? '));
    t = lastStop > cap * 0.5 ? cut.slice(0, lastStop + 1) : cut;
  }
  return t;
}

const SPOKEN_KEY = 'permagent-last-spoken-key';

/** Content-identity dedupe key. Turn position proved unreliable (a fresh
 *  window's rebuilt message list can count differently than the window that
 *  spoke first) — the reply's own text is the stable identity. */
export function replyDedupeKey(sessionId: string | null, content: string): string {
  let h = 0;
  for (let i = 0; i < content.length; i++) h = (h * 31 + content.charCodeAt(i)) | 0;
  return `${sessionId ?? 'nosession'}:${h}`;
}

/**
 * Record a reply as already voiced WITHOUT synthesizing it — for turns spoken
 * by the /voice pipeline, which bypasses this module entirely.
 *
 * Without this, a voice-spoken reply carried no dedupe key, so a later SSE
 * replay (fresh window, reconnect) would synthesize and speak it again. The
 * previous defense was a blanket 2.5s mute after every stream connect, which
 * also silently swallowed genuinely NEW replies that landed inside the window
 * (session switch, or a reconnect mid-conversation) — audio just vanished with
 * no indication. Marking is precise where the timer was indiscriminate.
 */
export function markReplySpoken(sessionId: string | null, content: string): void {
  if (!content) return;
  try {
    localStorage.setItem(SPOKEN_KEY, replyDedupeKey(sessionId, content));
  } catch { /* private mode — dedupe degrades, playback still correct */ }
}

/** Speak a completed reply. No-op when muted; a newer reply supersedes an
 *  in-flight one. Failures degrade silently to text-only.
 *
 *  `dedupeKey` (session + turn identity) is remembered in localStorage —
 *  SHARED across the main window and the popped-out chat window — so a
 *  replayed `Finish` frame (fresh window connecting with no SSE cursor, or a
 *  redock) never re-speaks a reply the user already heard. */
export async function maybeSpeakReply(
  markdown: string,
  voiceId?: string | null,
  dedupeKey?: string,
): Promise<void> {
  if (!enabled) return;
  if (dedupeKey) {
    try {
      if (localStorage.getItem(SPOKEN_KEY) === dedupeKey) return;
      localStorage.setItem(SPOKEN_KEY, dedupeKey);
    } catch { /* private mode — speak anyway */ }
  }
  const text = speakableText(markdown);
  if (!text) return;
  const seq = ++playSeq;
  try {
    const blob = await synthesizeVoice(text, voiceId);
    if (seq !== playSeq || !enabled) return; // muted or superseded meanwhile
    stopSpeaking();
    playSeq = seq; // stopSpeaking bumped it; this playback stays valid
    const audio = new Audio(URL.createObjectURL(blob));
    currentAudio = audio;
    audio.onended = () => {
      if (currentAudio === audio) currentAudio = null;
      URL.revokeObjectURL(audio.src);
    };
    await audio.play();
  } catch {
    /* synth or playback unavailable — the text is already on screen */
  }
}
