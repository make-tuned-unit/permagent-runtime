import { beforeEach, describe, expect, it } from 'vitest';
import { applyVoiceConfirmEvent, getVoiceConfirms, resetVoiceConfirms, sweepVoiceConfirms } from './voiceConfirm';

const NOW = 1_800_000_000_000;
const req = (over: Record<string, unknown> = {}) => ({
  confirm_id: 'c1', session_id: 's1', tool: 'dispatch_goal', summary: 'Start the CityCircle grow run',
  args_preview: { goal: 'grow' }, expires_at: new Date(NOW + 30_000).toISOString(), ...over,
});

describe('voice confirm store', () => {
  beforeEach(() => resetVoiceConfirms());

  it('adds a card on request and removes it on resolved', () => {
    expect(applyVoiceConfirmEvent('voice_confirm_requested', req(), NOW)).toBe(true);
    expect(getVoiceConfirms()).toHaveLength(1);
    expect(getVoiceConfirms()[0]).toMatchObject({ confirmId: 'c1', sessionId: 's1', summary: 'Start the CityCircle grow run', expiresAtMs: NOW + 30_000 });
    expect(applyVoiceConfirmEvent('voice_confirm_resolved', { confirm_id: 'c1', session_id: 's1', outcome: 'approved' }, NOW)).toBe(true);
    expect(getVoiceConfirms()).toHaveLength(0);
  });
  it('ignores malformed or already-expired requests and unrelated events', () => {
    expect(applyVoiceConfirmEvent('voice_confirm_requested', req({ confirm_id: '' }), NOW)).toBe(false);
    expect(applyVoiceConfirmEvent('voice_confirm_requested', req({ expires_at: new Date(NOW - 1).toISOString() }), NOW)).toBe(false);
    expect(applyVoiceConfirmEvent('goal_state_changed', {}, NOW)).toBe(false);
    expect(getVoiceConfirms()).toHaveLength(0);
  });
  it('lapses a card client-side at expires_at', () => {
    applyVoiceConfirmEvent('voice_confirm_requested', req(), NOW);
    expect(sweepVoiceConfirms(NOW + 29_999)).toBe(false);
    expect(sweepVoiceConfirms(NOW + 30_000)).toBe(true);
    expect(getVoiceConfirms()).toHaveLength(0);
  });
  it('replaces a repeated request for the same id', () => {
    applyVoiceConfirmEvent('voice_confirm_requested', req(), NOW);
    applyVoiceConfirmEvent('voice_confirm_requested', req({ summary: 'Updated' }), NOW);
    expect(getVoiceConfirms()).toHaveLength(1);
    expect(getVoiceConfirms()[0].summary).toBe('Updated');
  });
});
