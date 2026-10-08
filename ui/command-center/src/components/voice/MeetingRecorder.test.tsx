/**
 * @vitest-environment jsdom
 *
 * The rail Record button never stops a live meeting — while recording it
 * surfaces the panel — so its tooltip must not promise a stop.
 */
import { describe, it, expect, vi } from 'vitest';

// Never reach for a live daemon token from a unit test.
vi.mock('../../lib/api', () => ({ api: {}, apiFetch: vi.fn() }));

import {
  captureLine, noSpeechFeedback, focusables, pickerDescription, recordButtonTooltip, triageStatusLine,
} from './MeetingRecorder';
import { appAvailability, describeSources, unavailableAppMessage } from '../../hooks/useMeetingDictation';

describe('appAvailability', () => {
  const app = { bundleId: 'x.app', name: 'X' };
  it('is available only when the chosen app is in the running list', () => {
    expect(appAvailability(app, [app])).toBe('available');
    expect(appAvailability(app, [{ bundleId: 'y', name: 'Y' }])).toBe('unavailable');
    expect(appAvailability(app, [])).toBe('unavailable');
    expect(appAvailability(app, null)).toBe('unavailable'); // native can't target apps
    expect(appAvailability(app, undefined)).toBe('checking');
  });

  it('explains which kind of unavailable it is', () => {
    expect(unavailableAppMessage(app, [])).toMatch(/“X” isn't running/);
    expect(unavailableAppMessage(app, null)).toMatch(/can't capture a single app/);
  });
});

const APP = { bundleId: 'ai.permagent.record-practice-20261007', name: 'Record Practice' };

describe('source labels name what is actually recording', () => {
  it('describeSources', () => {
    expect(describeSources({ mic: true, system: false, app: null })).toBe('Your microphone');
    expect(describeSources({ mic: true, system: true, app: null })).toBe('Your microphone + all system audio');
    expect(describeSources({ mic: true, system: true, app: APP })).toBe('Your microphone + Record Practice audio');
    expect(describeSources({ mic: false, system: true, app: null })).toBe('All system audio only');
    expect(describeSources({ mic: false, system: true, app: APP })).toBe('Record Practice audio only');
  });

  it('captureLine never says "you" when the mic is off', () => {
    const line = captureLine({ mic: false, system: true, app: APP }, 0, 1);
    expect(line).toBe('Record Practice audio only — hearing Record Practice ✓');
    expect(line).not.toMatch(/\byou\b/i);
    expect(captureLine({ mic: true, system: false, app: null }, 1, 0)).toBe('Your microphone — hearing you ✓');
    expect(captureLine({ mic: true, system: true, app: null }, 0, 0))
      .toBe('Your microphone + all system audio — mic: listening… · waiting for system audio…');
  });

  it('pickerDescription states the sources and does not overclaim isolation', () => {
    expect(pickerDescription({ mic: false, system: true, app: null })).toMatch(/^Records all of this Mac's audio output .* not your microphone/);
    expect(pickerDescription({ mic: true, system: true, app: APP })).toMatch(/^Records your microphone and Record Practice's audio only\./);
    expect(pickerDescription({ mic: true, system: false, app: null })).toMatch(/^Records your microphone only/);
    expect(pickerDescription({ mic: false, system: false, app: null })).toBe('Nothing is selected to record.');
  });
});

describe('focusables', () => {
  it('lists tabbable controls in order, skipping disabled and tabindex=-1', () => {
    const root = document.createElement('div');
    root.innerHTML = `
      <input id="a"><button id="b" disabled></button><select id="c"></select>
      <div tabindex="-1" id="d"></div><button id="e"></button><div tabindex="0" id="f"></div>`;
    expect(focusables(root).map(el => el.id)).toEqual(['a', 'c', 'e', 'f']);
  });
});
import type { MeetingTriageView } from '../../hooks/useMeetingDictation';
import type { MeetingTriageReceipt } from '../../lib/api';

function view(receipt: Partial<MeetingTriageReceipt> | null, over: Partial<MeetingTriageView> = {}): MeetingTriageView {
  return {
    meeting: { projectId: 'p', projectName: 'P', recordingId: 'r', noteId: 'n', savedAt: '2026-10-07T10:00:00.000Z' },
    receipt: receipt && {
      id: 't', projectId: 'p', recordingId: 'r', noteId: 'n', status: 'pending',
      attemptCount: 0, cardIds: [], truncated: false, created: true, ...receipt,
    },
    unavailable: false, stalled: false, error: null,
    ...over,
  };
}

describe('triageStatusLine', () => {
  it('never claims the triage is done before the receipt says completed', () => {
    for (const status of ['pending', 'running'] as const) {
      expect(triageStatusLine(view({ status })).text).not.toMatch(/done|to-do card/i);
    }
    expect(triageStatusLine(view(null)).text).toMatch(/Checking/);
  });

  it('counts the cards a completed triage created', () => {
    expect(triageStatusLine(view({ status: 'completed', cardIds: ['a', 'b'] })).text)
      .toBe('Triage done — 2 to-do cards on the board.');
    expect(triageStatusLine(view({ status: 'completed', cardIds: ['a'] })).text)
      .toBe('Triage done — 1 to-do card on the board.');
    expect(triageStatusLine(view({ status: 'completed' })).text).toBe('Triage done — no to-dos found.');
  });

  it('surfaces a failure with its reason, and a skip as a settings question', () => {
    const failed = triageStatusLine(view({ status: 'failed', lastError: 'model timeout' }));
    expect(failed).toEqual({ text: 'Triage failed: model timeout', tone: 'danger' });
    expect(triageStatusLine(view({ status: 'skipped' })).text).toMatch(/skipped/);
  });

  it('says so when the daemon gave no triage status at all', () => {
    expect(triageStatusLine(view(null, { unavailable: true })).text).toMatch(/didn't report triage status/);
  });

  it('says "check again" once polling has given up', () => {
    expect(triageStatusLine(view({ status: 'running' }, { stalled: true })).text).toMatch(/check again/);
  });
});

describe('recordButtonTooltip', () => {
  it('while recording, points at the panel instead of claiming to stop', () => {
    const tip = recordButtonTooltip('recording');
    expect(tip).not.toMatch(/^Stop/);
    expect(tip).toMatch(/panel/);
  });

  it('describes the idle, error and saving actions plainly', () => {
    expect(recordButtonTooltip('idle')).toBe('Record a meeting to a project note');
    expect(recordButtonTooltip('error')).toBe('Record a meeting to a project note');
    expect(recordButtonTooltip('finishing')).toBe('Transcribing & saving…');
  });
});

describe('no speech feedback', () => {
  it('waits for the first chunk and names only sources without words, without claiming PCM silence', () => {
    const sources = { mic: true, system: true, app: APP };
    expect(noSpeechFeedback(sources, 59, 0, 0)).toBeNull();
    expect(noSpeechFeedback(sources, 60, 1, 1)).toBeNull();
    const feedback = noSpeechFeedback(sources, 60, 1, 0)!;
    expect(feedback).toContain('No speech has been transcribed from Record Practice yet');
    expect(feedback).toContain('may still be processing');
    expect(feedback).toContain('typed notes are kept');
    expect(noSpeechFeedback({ mic: false, system: true, app: APP }, 90, 0, 1)).toBeNull();
  });
});
