/**
 * Workspace metadata (#472 residue) — logic + wire-contract tests.
 *
 * Pins two things:
 *  1. The parsing/merge logic is tolerant of a shared, agent-writable
 *     metadata bag (malformed entries dropped, foreign keys preserved).
 *  2. saveProjectSummary's ACTUAL wire behavior against the existing
 *     PATCH /api/projects/:id: fresh-GET-before-metadata-merge (the bag is
 *     full-replacement on the wire), camelCase body keys, explicit null to
 *     clear siteUrl/repoUrl (double-Option semantics in routes/projects.rs).
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import {
  BRIEF_KEY,
  LINKS_KEY,
  mergeWorkspaceMeta,
  normalizeUrl,
  readBrief,
  readLinks,
  saveProjectSummary,
} from './workspaceMeta';

const apiFetchMock = vi.mocked(apiFetch);

beforeEach(() => {
  apiFetchMock.mockReset();
});

// ── readBrief ───────────────────────────────────────────────────────────────

describe('readBrief', () => {
  it('returns the brief string when present', () => {
    expect(readBrief({ [BRIEF_KEY]: 'the thesis' })).toBe('the thesis');
  });

  it('returns empty string for missing, non-string, or non-object bags', () => {
    expect(readBrief({})).toBe('');
    expect(readBrief({ [BRIEF_KEY]: 42 })).toBe('');
    expect(readBrief(null)).toBe('');
    expect(readBrief(undefined)).toBe('');
    expect(readBrief('not-an-object')).toBe('');
    expect(readBrief([])).toBe('');
  });
});

// ── readLinks ───────────────────────────────────────────────────────────────

describe('readLinks', () => {
  it('returns well-formed links', () => {
    const meta = { [LINKS_KEY]: [{ label: 'X', url: 'https://x.com/me' }] };
    expect(readLinks(meta)).toEqual([{ label: 'X', url: 'https://x.com/me' }]);
  });

  it('drops malformed entries instead of throwing (shared agent-writable bag)', () => {
    const meta = {
      [LINKS_KEY]: [
        { label: 'ok', url: 'https://a.com' },
        { label: 'no url' },
        { url: 'https://no-label.com' }, // label missing entirely → dropped
        { label: 'empty url', url: '   ' },
        'not-an-object',
        null,
        { label: 7, url: 'https://b.com' },
      ],
    };
    expect(readLinks(meta)).toEqual([{ label: 'ok', url: 'https://a.com' }]);
  });

  it('returns [] for non-array links or non-object bags', () => {
    expect(readLinks({ [LINKS_KEY]: 'nope' })).toEqual([]);
    expect(readLinks({})).toEqual([]);
    expect(readLinks(null)).toEqual([]);
  });
});

// ── normalizeUrl ────────────────────────────────────────────────────────────

describe('normalizeUrl', () => {
  it('maps empty/whitespace to null (clears the field)', () => {
    expect(normalizeUrl('')).toBeNull();
    expect(normalizeUrl('   ')).toBeNull();
  });

  it('prefixes https:// onto scheme-less input', () => {
    expect(normalizeUrl('example.com')).toBe('https://example.com');
    expect(normalizeUrl('  github.com/a/b  ')).toBe('https://github.com/a/b');
  });

  it('keeps existing schemes untouched', () => {
    expect(normalizeUrl('https://a.com')).toBe('https://a.com');
    expect(normalizeUrl('http://a.com')).toBe('http://a.com');
    expect(normalizeUrl('mailto:me@a.com')).toBe('mailto:me@a.com');
  });
});

// ── mergeWorkspaceMeta ──────────────────────────────────────────────────────

describe('mergeWorkspaceMeta', () => {
  it('preserves foreign keys (build_command et al.) untouched', () => {
    const meta = { build_command: 'cargo test', build_timeout_secs: 300 };
    const merged = mergeWorkspaceMeta(meta, { brief: 'hello', links: [{ label: 'a', url: 'https://a.com' }] });
    expect(merged).toEqual({
      build_command: 'cargo test',
      build_timeout_secs: 300,
      [BRIEF_KEY]: 'hello',
      [LINKS_KEY]: [{ label: 'a', url: 'https://a.com' }],
    });
  });

  it('does not mutate the input bag', () => {
    const meta = { build_command: 'x' };
    mergeWorkspaceMeta(meta, { brief: 'b' });
    expect(meta).toEqual({ build_command: 'x' });
  });

  it('removes the key on empty brief / empty links (no dead entries)', () => {
    const meta = { [BRIEF_KEY]: 'old', [LINKS_KEY]: [{ label: 'a', url: 'https://a.com' }], keep: 1 };
    expect(mergeWorkspaceMeta(meta, { brief: '   ', links: [] })).toEqual({ keep: 1 });
  });

  it('drops empty-url link rows and trims labels/urls', () => {
    const merged = mergeWorkspaceMeta({}, {
      links: [
        { label: '  X  ', url: ' https://x.com ' },
        { label: 'blank', url: '   ' },
      ],
    });
    expect(merged).toEqual({ [LINKS_KEY]: [{ label: 'X', url: 'https://x.com' }] });
  });

  it('leaves keys alone when a change is not passed at all', () => {
    const meta = { [BRIEF_KEY]: 'stays', [LINKS_KEY]: [{ label: 'a', url: 'https://a.com' }] };
    expect(mergeWorkspaceMeta(meta, {})).toEqual(meta);
  });

  it('starts from an empty object when the bag is malformed', () => {
    expect(mergeWorkspaceMeta('garbage', { brief: 'b' })).toEqual({ [BRIEF_KEY]: 'b' });
  });
});

// ── saveProjectSummary (wire contract) ──────────────────────────────────────

describe('saveProjectSummary', () => {
  it('re-fetches the project and PATCHes a merged metadataJson (full-replacement bag)', async () => {
    // Fresh copy on the daemon has a foreign key the (possibly stale) UI prop
    // wouldn't know about — the save must carry it through.
    apiFetchMock.mockResolvedValueOnce({ id: 'p1', metadataJson: { build_command: 'npm test' } });
    apiFetchMock.mockResolvedValueOnce({ id: 'p1' });

    await saveProjectSummary('p1', { brief: 'the plan', description: 'short' });

    expect(apiFetchMock).toHaveBeenCalledTimes(2);
    const [getUrl, getInit] = apiFetchMock.mock.calls[0];
    expect(getUrl).toBe('/api/projects/p1');
    expect(getInit).toBeUndefined(); // plain GET

    const [patchUrl, patchInit] = apiFetchMock.mock.calls[1];
    expect(patchUrl).toBe('/api/projects/p1');
    expect(patchInit?.method).toBe('PATCH');
    expect(JSON.parse(String(patchInit?.body))).toEqual({
      metadataJson: { build_command: 'npm test', [BRIEF_KEY]: 'the plan' },
      description: 'short',
    });
  });

  it('skips the fresh GET when only non-metadata fields change', async () => {
    apiFetchMock.mockResolvedValueOnce({ id: 'p1' });

    await saveProjectSummary('p1', { description: 'just this' });

    expect(apiFetchMock).toHaveBeenCalledTimes(1);
    const [, init] = apiFetchMock.mock.calls[0];
    expect(init?.method).toBe('PATCH');
    expect(JSON.parse(String(init?.body))).toEqual({ description: 'just this' });
  });

  it('sends explicit null to clear siteUrl/repoUrl (double-Option wire semantics)', async () => {
    apiFetchMock.mockResolvedValueOnce({ id: 'p1', metadataJson: {} });
    apiFetchMock.mockResolvedValueOnce({ id: 'p1' });

    await saveProjectSummary('p1', { siteUrl: null, repoUrl: 'https://github.com/a/b', links: [] });

    const [, patchInit] = apiFetchMock.mock.calls[1];
    const body = JSON.parse(String(patchInit?.body));
    expect(body.siteUrl).toBeNull(); // present AND null — clears the column
    expect('siteUrl' in body).toBe(true);
    expect(body.repoUrl).toBe('https://github.com/a/b');
    expect(body.metadataJson).toEqual({});
  });

  it('escapes the project id in both URLs', async () => {
    apiFetchMock.mockResolvedValueOnce({ id: 'weird', metadataJson: {} });
    apiFetchMock.mockResolvedValueOnce({ id: 'weird' });

    await saveProjectSummary('a/b c', { brief: 'x' });

    expect(apiFetchMock.mock.calls[0][0]).toBe('/api/projects/a%2Fb%20c');
    expect(apiFetchMock.mock.calls[1][0]).toBe('/api/projects/a%2Fb%20c');
  });

  it('propagates a failed PATCH (caller keeps the draft + shows the error)', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('daemon down'));
    await expect(saveProjectSummary('p1', { description: 'x' })).rejects.toThrow('daemon down');
  });
});
