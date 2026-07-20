// Publish sequence (#457) — reader tolerance, foreign-key-preserving merge,
// and the GET-then-PATCH wire contract of savePublishSequence.

import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  PUBLISH_SEQUENCE_KEY,
  readPublishSequence,
  mergePublishSequence,
  savePublishSequence,
} from './publishSequence';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));
import { apiFetch } from '../../lib/api';
const apiFetchMock = vi.mocked(apiFetch);

beforeEach(() => {
  apiFetchMock.mockReset();
});

// ── readPublishSequence ─────────────────────────────────────────────────────

describe('readPublishSequence', () => {
  it('reads [] from absent key, non-object bags, and non-array values', () => {
    expect(readPublishSequence({})).toEqual([]);
    expect(readPublishSequence(null)).toEqual([]);
    expect(readPublishSequence(undefined)).toEqual([]);
    expect(readPublishSequence('nope')).toEqual([]);
    expect(readPublishSequence({ [PUBLISH_SEQUENCE_KEY]: 'vercel --prod' })).toEqual([]);
    expect(readPublishSequence({ [PUBLISH_SEQUENCE_KEY]: { command: 'x' } })).toEqual([]);
  });

  it('parses canonical objects and sorts by explicit order', () => {
    const meta = {
      [PUBLISH_SEQUENCE_KEY]: [
        { order: 2, command: 'vercel --prod', timeout_secs: 600 },
        { order: 1, command: 'npx tsx scripts/reseed-threads.ts', timeout_secs: 300 },
      ],
    };
    expect(readPublishSequence(meta)).toEqual([
      { command: 'npx tsx scripts/reseed-threads.ts', timeoutSecs: 300 },
      { command: 'vercel --prod', timeoutSecs: 600 },
    ]);
  });

  it('accepts bare strings in array order and trims commands', () => {
    const meta = { [PUBLISH_SEQUENCE_KEY]: ['  seed.sh  ', 'vercel --prod'] };
    expect(readPublishSequence(meta)).toEqual([
      { command: 'seed.sh', timeoutSecs: undefined },
      { command: 'vercel --prod', timeoutSecs: undefined },
    ]);
  });

  it('drops malformed/blank entries and invalid timeouts, never throws', () => {
    const meta = {
      [PUBLISH_SEQUENCE_KEY]: [
        42,
        '',
        '   ',
        null,
        { order: 1 },
        { command: '   ' },
        { command: 'real-step', timeout_secs: -5 },
        { command: 'timed', timeout_secs: 30 },
      ],
    };
    expect(readPublishSequence(meta)).toEqual([
      { command: 'real-step', timeoutSecs: undefined },
      { command: 'timed', timeoutSecs: 30 },
    ]);
  });

  it('breaks order ties by array position (matches the Rust parser)', () => {
    const meta = {
      [PUBLISH_SEQUENCE_KEY]: [
        { order: 5, command: 'b' },
        { order: 5, command: 'c' },
        { order: 0, command: 'a' },
      ],
    };
    expect(readPublishSequence(meta).map(s => s.command)).toEqual(['a', 'b', 'c']);
  });
});

// ── mergePublishSequence ────────────────────────────────────────────────────

describe('mergePublishSequence', () => {
  it('preserves foreign keys (build_command, brief, …)', () => {
    const meta = {
      build_command: 'npm run build',
      brief: 'the thesis',
      [PUBLISH_SEQUENCE_KEY]: [{ order: 1, command: 'old' }],
    };
    const merged = mergePublishSequence(meta, [{ command: 'vercel --prod' }]);
    expect(merged.build_command).toBe('npm run build');
    expect(merged.brief).toBe('the thesis');
    expect(merged[PUBLISH_SEQUENCE_KEY]).toEqual([{ order: 1, command: 'vercel --prod' }]);
  });

  it('does not mutate the input bag (pure)', () => {
    const meta = { build_command: 'make', [PUBLISH_SEQUENCE_KEY]: [{ order: 1, command: 'old' }] };
    const snapshot = JSON.parse(JSON.stringify(meta));
    mergePublishSequence(meta, [{ command: 'new' }]);
    mergePublishSequence(meta, []);
    expect(meta).toEqual(snapshot);
  });

  it('normalizes to canonical shape with order 1..N and snake_case timeout', () => {
    const merged = mergePublishSequence({}, [
      { command: '  seed  ', timeoutSecs: 300 },
      { command: 'deploy' },
    ]);
    expect(merged[PUBLISH_SEQUENCE_KEY]).toEqual([
      { order: 1, command: 'seed', timeout_secs: 300 },
      { order: 2, command: 'deploy' },
    ]);
  });

  it('drops blank commands and invalid timeouts', () => {
    const merged = mergePublishSequence({}, [
      { command: '   ' },
      { command: 'ok', timeoutSecs: 0 },
    ]);
    expect(merged[PUBLISH_SEQUENCE_KEY]).toEqual([{ order: 1, command: 'ok' }]);
  });

  it('empty sequence removes the key entirely, leaving siblings intact', () => {
    const merged = mergePublishSequence(
      { build_command: 'make', [PUBLISH_SEQUENCE_KEY]: [{ order: 1, command: 'x' }] },
      [],
    );
    expect(PUBLISH_SEQUENCE_KEY in merged).toBe(false);
    expect(merged).toEqual({ build_command: 'make' });
  });

  it('a non-object bag is replaced by a fresh object', () => {
    const merged = mergePublishSequence(null, [{ command: 'x' }]);
    expect(merged[PUBLISH_SEQUENCE_KEY]).toEqual([{ order: 1, command: 'x' }]);
  });
});

// ── savePublishSequence wire contract ───────────────────────────────────────

describe('savePublishSequence', () => {
  it('re-fetches the project, merges over the FRESH bag, and PATCHes metadataJson', async () => {
    // The fresh copy holds a sibling key a stale prop would not know about.
    apiFetchMock
      .mockResolvedValueOnce({
        id: 'p1',
        metadataJson: { build_command: 'npm run build', brief: 'fresh-only key' },
        updatedAt: '2026-07-20T00:00:00Z',
      })
      .mockResolvedValueOnce({ id: 'p1', metadataJson: {}, updatedAt: '2026-07-20T00:00:01Z' });

    await savePublishSequence('p1', [{ command: 'vercel --prod', timeoutSecs: 600 }]);

    expect(apiFetchMock).toHaveBeenCalledTimes(2);
    const [getUrl, getOpts] = apiFetchMock.mock.calls[0];
    expect(getUrl).toBe('/api/projects/p1');
    expect(getOpts).toBeUndefined();

    const [patchUrl, patchOpts] = apiFetchMock.mock.calls[1];
    expect(patchUrl).toBe('/api/projects/p1');
    expect(patchOpts?.method).toBe('PATCH');
    const body = JSON.parse(String(patchOpts?.body));
    // ONLY metadataJson rides the PATCH — no other project fields touched.
    expect(Object.keys(body)).toEqual(['metadataJson']);
    // Fresh sibling keys survive; our key is canonical.
    expect(body.metadataJson).toEqual({
      build_command: 'npm run build',
      brief: 'fresh-only key',
      [PUBLISH_SEQUENCE_KEY]: [{ order: 1, command: 'vercel --prod', timeout_secs: 600 }],
    });
  });

  it('URL-encodes the project id', async () => {
    apiFetchMock
      .mockResolvedValueOnce({ id: 'a/b', metadataJson: {} })
      .mockResolvedValueOnce({ id: 'a/b', metadataJson: {} });
    await savePublishSequence('a/b', [{ command: 'x' }]);
    expect(apiFetchMock.mock.calls[0][0]).toBe('/api/projects/a%2Fb');
    expect(apiFetchMock.mock.calls[1][0]).toBe('/api/projects/a%2Fb');
  });

  it('clearing the sequence PATCHes a bag without the key', async () => {
    apiFetchMock
      .mockResolvedValueOnce({
        id: 'p1',
        metadataJson: { build_command: 'make', [PUBLISH_SEQUENCE_KEY]: [{ order: 1, command: 'x' }] },
      })
      .mockResolvedValueOnce({ id: 'p1', metadataJson: { build_command: 'make' } });

    await savePublishSequence('p1', []);
    const body = JSON.parse(String(apiFetchMock.mock.calls[1][1]?.body));
    expect(body.metadataJson).toEqual({ build_command: 'make' });
  });

  it('propagates a failed PATCH (caller keeps the draft)', async () => {
    apiFetchMock
      .mockResolvedValueOnce({ id: 'p1', metadataJson: {} })
      .mockRejectedValueOnce(new Error('daemon down'));
    await expect(savePublishSequence('p1', [{ command: 'x' }])).rejects.toThrow('daemon down');
  });
});
