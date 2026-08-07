/**
 * @vitest-environment jsdom
 *
 * The crash-recovery stash. Its whole reason to exist is that a meeting's words
 * are never lost, so the regressions worth pinning are the ones that lose them
 * quietly: one recording burying another's unrecovered draft, a stale far side
 * bleeding into the wrong note, and the stash growing until localStorage
 * refuses to hold anything at all.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import {
  MAX_DRAFTS,
  composeDraftBody,
  composeTranscriptBody,
  draftKey,
  farPartsFor,
  migrateLegacyDraft,
  pruneDrafts,
  readDrafts,
  type MeetingDraft,
} from './useMeetingDictation';

const LEGACY_KEY = 'permagent-meeting-draft';

function draft(startedAt: string, over: Partial<MeetingDraft> = {}): MeetingDraft {
  return {
    projectId: 'p1',
    projectName: 'Acme',
    startedAt,
    parts: ['some words'],
    farParts: [],
    ...over,
  };
}

function stash(d: MeetingDraft): void {
  localStorage.setItem(draftKey(d.startedAt), JSON.stringify(d));
}

beforeEach(() => localStorage.clear());

describe('readDrafts', () => {
  it('returns every stashed draft, newest first', () => {
    stash(draft('2026-08-07T09:00:00.000Z'));
    stash(draft('2026-08-07T11:00:00.000Z'));
    stash(draft('2026-08-07T10:00:00.000Z'));
    expect(readDrafts().map(d => d.startedAt)).toEqual([
      '2026-08-07T11:00:00.000Z',
      '2026-08-07T10:00:00.000Z',
      '2026-08-07T09:00:00.000Z',
    ]);
  });

  it('keeps a second recording from burying an unrecovered first', () => {
    // The bug this replaced: one shared slot, so starting another meeting
    // destroyed the transcript the recovery panel had not offered back yet.
    stash(draft('2026-08-07T09:00:00.000Z', { projectName: 'First' }));
    stash(draft('2026-08-07T10:00:00.000Z', { projectName: 'Second' }));
    expect(readDrafts().map(d => d.projectName)).toEqual(['Second', 'First']);
  });

  it('ignores unrelated keys and unparseable slots rather than throwing', () => {
    localStorage.setItem('some-other-app', 'not json');
    localStorage.setItem(draftKey('2026-08-07T09:00:00.000Z'), '{ broken');
    localStorage.setItem(draftKey('2026-08-07T10:00:00.000Z'), JSON.stringify({ projectId: 'p' }));
    stash(draft('2026-08-07T11:00:00.000Z'));
    expect(readDrafts().map(d => d.startedAt)).toEqual(['2026-08-07T11:00:00.000Z']);
  });
});

describe('pruneDrafts', () => {
  it('bounds the stash and never evicts the recording being written', () => {
    // Oldest first so the live one is also the oldest — the case a naive
    // "drop the oldest" would get wrong.
    const live = '2026-08-07T00:00:00.000Z';
    stash(draft(live));
    for (let i = 1; i <= MAX_DRAFTS + 3; i++) {
      stash(draft(`2026-08-07T0${i}:00:00.000Z`));
    }
    pruneDrafts(live);
    const kept = readDrafts().map(d => d.startedAt);
    expect(kept).toHaveLength(MAX_DRAFTS);
    expect(kept).toContain(live);
  });

  it('leaves a stash that is already within bounds alone', () => {
    stash(draft('2026-08-07T09:00:00.000Z'));
    stash(draft('2026-08-07T10:00:00.000Z'));
    pruneDrafts('2026-08-07T10:00:00.000Z');
    expect(readDrafts()).toHaveLength(2);
  });
});

describe('migrateLegacyDraft', () => {
  it('moves the old single slot into a keyed one and retires the old key', () => {
    const d = draft('2026-08-06T22:00:00.000Z');
    localStorage.setItem(LEGACY_KEY, JSON.stringify(d));
    migrateLegacyDraft();
    expect(localStorage.getItem(LEGACY_KEY)).toBeNull();
    expect(readDrafts().map(x => x.startedAt)).toEqual([d.startedAt]);
  });

  it('discards an unreadable legacy slot instead of leaving it forever', () => {
    localStorage.setItem(LEGACY_KEY, 'not json');
    migrateLegacyDraft();
    expect(localStorage.getItem(LEGACY_KEY)).toBeNull();
    expect(readDrafts()).toEqual([]);
  });

  it('is a no-op when there is nothing to migrate', () => {
    migrateLegacyDraft();
    expect(readDrafts()).toEqual([]);
  });
});

describe('farPartsFor', () => {
  it('hands back the far side of the recording that owns it', () => {
    expect(farPartsFor(3, { recordingId: 3, parts: ['they said hello'] })).toEqual([
      'they said hello',
    ]);
  });

  it('hands back nothing once the recording has moved on', () => {
    // The bug: record WITH system audio, stop, record WITHOUT it — the second
    // note was composed two-sided out of the first call's words.
    expect(farPartsFor(4, { recordingId: 3, parts: ['last meeting'] })).toEqual([]);
  });
});

describe('composeTranscriptBody', () => {
  it('stays a single-voice paragraph when the far side said nothing', () => {
    // Labelling a mic-only recording "You:" throughout would imply the other
    // half was captured and silent — a claim about coverage we cannot make.
    expect(composeTranscriptBody(['a', 'b'], [])).toBe('a b');
    expect(composeTranscriptBody(['a'], ['', '  '])).toBe('a');
  });

  it('becomes two-speaker markdown as soon as the far side has words', () => {
    expect(composeTranscriptBody(['hi'], ['welcome'])).toBe('**Others:** welcome\n\n**You:** hi');
  });

  it('is what the recovery path composes too', () => {
    // A recovered transcript must not read differently from one that was
    // never interrupted.
    const d = draft('2026-08-07T09:00:00.000Z', { parts: ['hi'], farParts: ['welcome'] });
    expect(composeDraftBody(d)).toBe(composeTranscriptBody(d.parts, d.farParts));
  });
});
