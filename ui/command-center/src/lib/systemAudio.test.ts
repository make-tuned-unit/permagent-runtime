import { describe, it, expect } from 'vitest';
import { normalizeAppList, normalizeStopResult } from './systemAudio';

describe('normalizeAppList', () => {
  it('keeps well-formed apps, sorted by name, falling back to the bundle id for a blank name', () => {
    expect(normalizeAppList([
      { bundleId: 'us.zoom.xos', name: 'Zoom' },
      { bundleId: 'ai.permagent.record-practice-20261007', name: 'Record Practice' },
      { bundleId: 'com.example.blank', name: '  ' },
      { bundleId: '', name: 'No id' },
      { name: 'Missing id' },
      null,
      'nope',
    ])).toEqual([
      { bundleId: 'com.example.blank', name: 'com.example.blank' },
      { bundleId: 'ai.permagent.record-practice-20261007', name: 'Record Practice' },
      { bundleId: 'us.zoom.xos', name: 'Zoom' },
    ]);
  });

  it('treats a non-array as no apps', () => {
    expect(normalizeAppList(null)).toEqual([]);
    expect(normalizeAppList({})).toEqual([]);
  });
});

describe('normalizeStopResult', () => {
  it('passes a newer native\'s unread chunk list through', () => {
    expect(normalizeStopResult(['/t/sysaudio_0001.wav', '/t/sysaudio_0002.wav']))
      .toEqual(['/t/sysaudio_0001.wav', '/t/sysaudio_0002.wav']);
  });

  it('reports an older native (null / unit) as null, not as "none left"', () => {
    expect(normalizeStopResult(null)).toBeNull();
    expect(normalizeStopResult(undefined)).toBeNull();
  });

  it('keeps an empty list distinct from null', () => {
    expect(normalizeStopResult([])).toEqual([]);
  });

  it('drops anything that is not a usable path', () => {
    expect(normalizeStopResult(['/t/a.wav', 3, '', '  ', null])).toEqual(['/t/a.wav']);
  });
});
