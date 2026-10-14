import { describe, expect, it } from 'vitest';
import {
  buildHello, createBargeGuard, deriveVoiceSurface, isClippedTransient, isHeadphoneOutputLabel,
  interruptMsg, replyPlaybackCompleteMsg,
} from './voiceClientProtocol';

describe('surface', () => {
  it('is world from the World view, orb from the chat pane', () => {
    expect(deriveVoiceSurface({ worldConversationActive: true, activeTool: 'chat' })).toBe('world');
    expect(deriveVoiceSurface({ worldConversationActive: false, activeTool: 'world' })).toBe('world');
    expect(deriveVoiceSurface({ worldConversationActive: false, activeTool: 'chat' })).toBe('orb');
    expect(deriveVoiceSurface({ worldConversationActive: false, activeTool: null })).toBe('orb');
  });
  it('hello carries surface and build', () => {
    const h = buildHello({ surface: 'world', build: { appVersion: '1.2.3', buildNumber: '45', gitSha: 'abc' }, wireSampleRate: 16000, revision: 1 });
    expect(h).toMatchObject({ type: 'hello', client: 'desktop_voice', surface: 'world', app_version: '1.2.3', build_number: '45', git_sha: 'abc' });
  });
});

describe('clipped transients (V-18)', () => {
  it('discards short hands-free captures, keeps taps, wake turns and real speech', () => {
    expect(isClippedTransient({ trigger: 'vad', voicedMs: 256, peak: 1 })).toBe(true);
    expect(isClippedTransient({ trigger: 'vad', voicedMs: 380, peak: 0.3 })).toBe(true);
    expect(isClippedTransient({ trigger: 'vad', voicedMs: 700, peak: 1 })).toBe(true); // clipped, still short
    expect(isClippedTransient({ trigger: 'vad', voicedMs: 700, peak: 0.4 })).toBe(false);
    expect(isClippedTransient({ trigger: 'barge', voicedMs: 1500, peak: 1 })).toBe(false);
    expect(isClippedTransient({ trigger: 'user_tap', voicedMs: 100, peak: 1 })).toBe(false);
    expect(isClippedTransient({ trigger: 'wake', voicedMs: 100, peak: 1 })).toBe(false);
  });
});

describe('echo guard (D7, mirrors 39ea442b)', () => {
  const feed = (g: ReturnType<typeof createBargeGuard>, t: number, mic: number, speaker: boolean) =>
    g.onMicBuffer({ nowMs: t, micRms: mic, voiceLike: true, speakerRoute: speaker });

  it('speakers: a single loud burst or two is pending, three within 1.2 s confirm', () => {
    const g = createBargeGuard();
    expect(feed(g, 0, 0.2, true)).toBe('pending');
    expect(feed(g, 128, 0.2, true)).toBe('pending');
    expect(feed(g, 256, 0.2, true)).toBe('confirmed');
  });
  it('speakers: confirmations spread over more than 1.2 s never confirm', () => {
    const g = createBargeGuard();
    for (let i = 0; i < 6; i++) expect(feed(g, i * 700, 0.2, true)).toBe('pending');
  });
  it('speakers: the floor follows the 1.5 s playback peak, so a dip between words does not lower it', () => {
    const g = createBargeGuard();
    g.notePlayback(0, 0.4); // loud word
    g.notePlayback(900, 0.0); // gap between words
    expect(feed(g, 1000, 0.15, true)).toBe('ignore'); // below 0.6 * 0.4
    expect(feed(g, 1100, 0.3, true)).toBe('pending');
    // After 1.5 s the held peak lapses and the base floor applies again.
    expect(feed(g, 2000, 0.06, true)).toBe('pending');
  });
  it('speakers: non-voice-shaped bursts never count', () => {
    const g = createBargeGuard();
    for (let i = 0; i < 5; i++) expect(g.onMicBuffer({ nowMs: i * 100, micRms: 0.5, voiceLike: false, speakerRoute: true })).toBe('ignore');
  });
  it('headphones: two consecutive loud buffers barge in; a gap resets', () => {
    const g = createBargeGuard();
    expect(feed(g, 0, 0.1, false)).toBe('pending');
    expect(feed(g, 100, 0.01, false)).toBe('ignore');
    expect(feed(g, 200, 0.1, false)).toBe('pending');
    expect(feed(g, 300, 0.1, false)).toBe('confirmed');
  });
  it('recognises headphone outputs by label and defaults unknown labels to speakers', () => {
    expect(isHeadphoneOutputLabel("Alex's AirPods Pro")).toBe(true);
    expect(isHeadphoneOutputLabel('MacBook Pro Speakers')).toBe(false);
    expect(isHeadphoneOutputLabel('')).toBe(false);
  });
});

describe('wire messages', () => {
  it('interrupt and reply-complete have the documented fields', () => {
    expect(interruptMsg({ turnId: 'web-3', reason: 'barge_in', heardMs: 1234.4, micRms: 0.2, playbackPeak: 0.1, route: 'speaker', clientMs: 1 }))
      .toEqual({ type: 'interrupt', turn_id: 'web-3', reason: 'barge_in', heard_ms: 1234, mic_rms: 0.2, playback_peak: 0.1, route: 'speaker', client_ms: 1 });
    expect(replyPlaybackCompleteMsg({ turnId: 'web-3', segments: 2, heardMs: 900, queuedMs: 1000, reason: 'interrupt', clientMs: 1 }))
      .toMatchObject({ type: 'reply_playback_complete', completed: false, reason: 'interrupt', segments: 2 });
  });
});
