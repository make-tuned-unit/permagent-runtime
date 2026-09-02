/**
 * Brain memory focus — pure resolution logic for the cross-surface "View in
 * Brain" seam. Covers the graph-preferred / preview-fallback resolution order
 * (the crux: fresh, description-less writes aren't in the Brain graph yet, so the
 * caller's preview must render), the ProjectMemory→preview mapping, age/weight
 * normalization, and the shared memory-title derivation reused by the list.
 */

import { describe, expect, it } from 'vitest';
import {
  resolveFocusedMemory,
  previewToGraphMemory,
  projectMemoryPreview,
  ageFromTimestamp,
  formatMemoryAge,
  parseBrainTimestamp,
  deriveMemoryTitle,
  humanizeMemoryKey,
  type BrainMemoryTarget,
} from './brainMemoryFocus';
import type { GraphMemory } from './useBrainData';
import type { ProjectMemory } from '../projects/types';

function graphMem(over: Partial<GraphMemory> = {}): GraphMemory {
  return {
    id: 'm1', key: 'k1', text: 'body', description: 'A desc', ent: [],
    age: 0.1, weight: 0.5, timestamp: '2026-07-10T00:00:00Z', ...over,
  };
}

function projMem(over: Partial<ProjectMemory> = {}): ProjectMemory {
  return {
    id: 'm1', key: 'note:p:1', content: 'note body', description: 'Enriched.',
    signal_score: 0.8, created_at: '2026-07-10 00:00:00', associated_at: '2026-07-11 00:00:00', ...over,
  };
}

describe('resolveFocusedMemory', () => {
  it('prefers a live graph hit by id (real recency/chips win over preview)', () => {
    const g = graphMem({ id: 'm1', description: 'Graph copy' });
    const target: BrainMemoryTarget = { id: 'm1', key: 'k1', preview: { text: 'stale', description: 'Preview copy' } };
    const r = resolveFocusedMemory(target, [g]);
    expect(r.kind).toBe('graph');
    expect(r.kind === 'graph' && r.memory.description).toBe('Graph copy');
  });

  it('matches by key when id is absent', () => {
    const g = graphMem({ id: 'x', key: 'note:p:1' });
    const r = resolveFocusedMemory({ key: 'note:p:1' }, [g]);
    expect(r.kind).toBe('graph');
    expect(r.kind === 'graph' && r.memory.id).toBe('x');
  });

  it('falls back to a synthesized preview when not in the graph', () => {
    const target: BrainMemoryTarget = { id: 'missing', key: 'note:p:9', preview: { text: 'fresh note', description: null } };
    const r = resolveFocusedMemory(target, [graphMem({ id: 'other', key: 'other' })]);
    expect(r.kind).toBe('preview');
    expect(r.kind === 'preview' && r.memory.text).toBe('fresh note');
    expect(r.kind === 'preview' && r.memory.id).toBe('missing');
  });

  it('returns none when neither a graph hit nor a preview is available', () => {
    const r = resolveFocusedMemory({ key: 'code:p:map' }, []);
    expect(r.kind).toBe('none');
  });
});

describe('previewToGraphMemory', () => {
  it('clamps weight to 0..1 and carries id/key/text/description with empty entities', () => {
    const m = previewToGraphMemory({ id: 'a', key: 'k', preview: { text: 't', description: 'd', weight: 5 } });
    expect(m.weight).toBe(1);
    expect(m.id).toBe('a');
    expect(m.key).toBe('k');
    expect(m.text).toBe('t');
    expect(m.description).toBe('d');
    expect(m.ent).toEqual([]);
  });

  it('defaults weight to 0 and age to the mid bucket when the preview is bare', () => {
    const m = previewToGraphMemory({ key: 'k', preview: { text: 't' } });
    expect(m.weight).toBe(0);
    expect(m.age).toBe(0.5);
  });
});

describe('ageFromTimestamp', () => {
  it('is ~0 for now and clamps to 1 beyond the 90-day window', () => {
    const now = Date.parse('2026-07-10T00:00:00Z');
    expect(ageFromTimestamp('2026-07-10T00:00:00Z', now)).toBeCloseTo(0, 5);
    expect(ageFromTimestamp('2020-01-01T00:00:00Z', now)).toBe(1);
    expect(ageFromTimestamp(null, now)).toBe(0.5);
  });

  it('reads a naive SQLite "YYYY-MM-DD HH:MM:SS" timestamp as UTC (45d → ~half)', () => {
    const now = Date.parse('2026-07-10T00:00:00Z');
    expect(ageFromTimestamp('2026-05-26 00:00:00', now)).toBeCloseTo(0.5, 2);
  });
});

describe('formatMemoryAge', () => {
  const now = Date.parse('2026-07-10T00:00:00Z');
  const daysAgo = (d: number) => new Date(now - d * 86_400_000).toISOString();

  // The reason this function exists. Recency was rendered from the same 0..1
  // scalar the 3D scene uses for colour, normalized over 90 days and clamped —
  // so a memory from last quarter and a memory from three years ago both read
  // "~year", and the older one looked no less current than the newer.
  it('separates ages the 90-day window used to collapse into one label', () => {
    const justPast = formatMemoryAge(daysAgo(91), now);
    const ancient = formatMemoryAge(daysAgo(3 * 365), now);
    expect(justPast.label).not.toBe(ancient.label);
    expect(justPast.label).toBe('3 months ago');
    expect(ancient.label).toBe('3 years ago');
  });

  it('reads honestly at every magnitude', () => {
    expect(formatMemoryAge(daysAgo(0), now).label).toBe('today');
    expect(formatMemoryAge(daysAgo(1), now).label).toBe('yesterday');
    expect(formatMemoryAge(daysAgo(4), now).label).toBe('4 days ago');
    expect(formatMemoryAge(daysAgo(9), now).label).toBe('last week');
    expect(formatMemoryAge(daysAgo(21), now).label).toBe('3 weeks ago');
    expect(formatMemoryAge(daysAgo(45), now).label).toBe('last month');
    expect(formatMemoryAge(daysAgo(240), now).label).toBe('8 months ago');
    expect(formatMemoryAge(daysAgo(400), now).label).toBe('last year');
    expect(formatMemoryAge(daysAgo(20 * 365), now).label).toBe('20 years ago');
  });

  it('marks anything past the staleness threshold, and nothing inside it', () => {
    expect(formatMemoryAge(daysAgo(10), now).stale).toBe(false);
    expect(formatMemoryAge(daysAgo(89), now).stale).toBe(false);
    expect(formatMemoryAge(daysAgo(120), now).stale).toBe(true);
    expect(formatMemoryAge(daysAgo(3 * 365), now).stale).toBe(true);
  });

  it('never guesses when there is no usable date', () => {
    expect(formatMemoryAge(null, now)).toEqual({ label: 'date unknown', stale: true });
    expect(formatMemoryAge('not-a-date', now)).toEqual({ label: 'date unknown', stale: true });
  });

  it('reads a naive SQLite timestamp as UTC, like every other Brain surface', () => {
    expect(formatMemoryAge('2026-05-26 00:00:00', now).label).toBe('last month');
  });

  // A clock skew (or an import dated in the future) must not render as a
  // negative age or a fabricated "in 3 days".
  it('treats a future timestamp as now', () => {
    expect(formatMemoryAge(new Date(now + 86_400_000).toISOString(), now).label).toBe('today');
  });
});

describe('parseBrainTimestamp', () => {
  it('parses ISO and naive-UTC forms, rejects garbage', () => {
    expect(parseBrainTimestamp('2026-07-10T00:00:00Z')).toBe(Date.parse('2026-07-10T00:00:00Z'));
    expect(parseBrainTimestamp('2026-07-10 00:00:00')).toBe(Date.parse('2026-07-10T00:00:00Z'));
    expect(parseBrainTimestamp('not-a-date')).toBeNull();
  });
});

describe('projectMemoryPreview', () => {
  it('maps the ProjectMemory wire shape to a focus preview', () => {
    expect(projectMemoryPreview(projMem())).toEqual({
      text: 'note body', description: 'Enriched.', weight: 0.8, timestamp: '2026-07-10 00:00:00',
    });
  });
});

describe('deriveMemoryTitle', () => {
  it('uses the description first sentence when present', () => {
    expect(deriveMemoryTitle(graphMem({ description: 'First thing. Second thing.' }))).toBe('First thing.');
  });
  it('humanizes the key when there is no description', () => {
    expect(deriveMemoryTitle(graphMem({ description: null, key: 'daily-standup' }))).toBe('Daily Standup');
  });
  it('falls back to a content preview with neither', () => {
    expect(deriveMemoryTitle(graphMem({ description: null, key: null, text: 'hello world' }))).toBe('hello world');
  });
});

describe('humanizeMemoryKey', () => {
  it('drops long digit runs and title-cases the rest', () => {
    expect(humanizeMemoryKey('daily_standup_1700000000')).toBe('Daily Standup');
  });
  it('returns the raw key when every part is a dropped hash', () => {
    expect(humanizeMemoryKey('deadbeefcafe')).toBe('deadbeefcafe');
  });
});
