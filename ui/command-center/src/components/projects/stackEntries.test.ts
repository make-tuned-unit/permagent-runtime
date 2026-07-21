/**
 * Stack organizer (#512) — wire-contract + grouping tests.
 *
 * Pins the StackPanel's ACTUAL wire behavior against the mounted
 * /api/projects/{id}/stack endpoints (routes/projects.rs): camelCase request
 * keys, snake_case responses, explicit-null clears on PATCH (double-Option
 * semantics), URL-escaped path segments, and error propagation (no silent
 * catch). Also pins the display grouping: STACK_CATEGORIES order, unknown
 * categories tolerated into "other" — never dropped.
 *
 * Reference-only contract note: there is no field for a password/secret in
 * the draft/patch types, and the shape tests here assert exactly which keys
 * go on the wire.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import {
  createStackEntry,
  deleteStackEntry,
  groupByCategory,
  listStackEntries,
  updateStackEntry,
} from './stackEntries';
import type { StackEntry } from './types';

const apiFetchMock = vi.mocked(apiFetch);

beforeEach(() => {
  apiFetchMock.mockReset();
});

function entry(overrides: Partial<StackEntry>): StackEntry {
  return {
    id: 'se-1',
    project_id: 'p-1',
    service_name: 'Vercel',
    category: 'hosting',
    identity: null,
    notes: '',
    dashboard_url: null,
    created_at: '2026-07-20T00:00:00Z',
    updated_at: '2026-07-20T00:00:00Z',
    ...overrides,
  };
}

// ── listStackEntries ────────────────────────────────────────────────────────

describe('listStackEntries', () => {
  it('GETs the project stack path with the id escaped', async () => {
    apiFetchMock.mockResolvedValueOnce([]);
    await listStackEntries('p 1/x');
    expect(apiFetchMock).toHaveBeenCalledWith('/api/projects/p%201%2Fx/stack');
  });

  it('propagates load failures (no silent catch)', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('boom'));
    await expect(listStackEntries('p-1')).rejects.toThrow('boom');
  });
});

// ── createStackEntry ────────────────────────────────────────────────────────

describe('createStackEntry', () => {
  it('POSTs camelCase keys and returns the saved row', async () => {
    const saved = entry({ identity: 'jesse+kinrows@gmail.com' });
    apiFetchMock.mockResolvedValueOnce(saved);

    const result = await createStackEntry('p-1', {
      serviceName: 'Railway',
      category: 'hosting',
      identity: 'jesse+kinrows@gmail.com',
      notes: 'free tier',
      dashboardUrl: 'https://railway.app/dashboard',
    });

    expect(result).toBe(saved);
    const [path, options] = apiFetchMock.mock.calls[0];
    expect(path).toBe('/api/projects/p-1/stack');
    expect(options?.method).toBe('POST');
    expect(JSON.parse(options?.body as string)).toEqual({
      serviceName: 'Railway',
      category: 'hosting',
      identity: 'jesse+kinrows@gmail.com',
      notes: 'free tier',
      dashboardUrl: 'https://railway.app/dashboard',
    });
  });

  it('omits optional keys entirely when not provided (never sends secret-shaped extras)', async () => {
    apiFetchMock.mockResolvedValueOnce(entry({}));
    await createStackEntry('p-1', { serviceName: 'Neon', category: 'database' });
    const body = JSON.parse(apiFetchMock.mock.calls[0][1]?.body as string);
    expect(Object.keys(body).sort()).toEqual(['category', 'serviceName']);
  });

  it('propagates create failures', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('HTTP 400'));
    await expect(
      createStackEntry('p-1', { serviceName: 'X', category: 'social' }),
    ).rejects.toThrow('HTTP 400');
  });
});

// ── updateStackEntry ────────────────────────────────────────────────────────

describe('updateStackEntry', () => {
  it('PATCHes the entry path with both ids escaped', async () => {
    apiFetchMock.mockResolvedValueOnce(entry({}));
    await updateStackEntry('p 1', 'e/2', { notes: 'hi' });
    const [path, options] = apiFetchMock.mock.calls[0];
    expect(path).toBe('/api/projects/p%201/stack/e%2F2');
    expect(options?.method).toBe('PATCH');
  });

  it('sends explicit JSON null to clear identity/dashboardUrl (double-Option)', async () => {
    apiFetchMock.mockResolvedValueOnce(entry({}));
    await updateStackEntry('p-1', 'se-1', {
      identity: null,
      dashboardUrl: null,
      notes: 'cleared',
    });
    const raw = apiFetchMock.mock.calls[0][1]?.body as string;
    const body = JSON.parse(raw);
    expect(body.identity).toBeNull();
    expect(body.dashboardUrl).toBeNull();
    // null must survive serialization (not be dropped like undefined).
    expect(raw).toContain('"identity":null');
    expect(raw).toContain('"dashboardUrl":null');
  });

  it('omits untouched fields so the backend leaves them unchanged', async () => {
    apiFetchMock.mockResolvedValueOnce(entry({}));
    await updateStackEntry('p-1', 'se-1', { notes: 'only notes' });
    const body = JSON.parse(apiFetchMock.mock.calls[0][1]?.body as string);
    expect(Object.keys(body)).toEqual(['notes']);
  });

  it('propagates update failures', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('HTTP 404'));
    await expect(updateStackEntry('p-1', 'gone', { notes: 'x' })).rejects.toThrow('HTTP 404');
  });
});

// ── deleteStackEntry ────────────────────────────────────────────────────────

describe('deleteStackEntry', () => {
  it('DELETEs the entry path', async () => {
    apiFetchMock.mockResolvedValueOnce(undefined);
    await deleteStackEntry('p-1', 'se-1');
    expect(apiFetchMock).toHaveBeenCalledWith('/api/projects/p-1/stack/se-1', {
      method: 'DELETE',
    });
  });

  it('propagates delete failures', async () => {
    apiFetchMock.mockRejectedValueOnce(new Error('HTTP 404'));
    await expect(deleteStackEntry('p-1', 'gone')).rejects.toThrow('HTTP 404');
  });
});

// ── groupByCategory ─────────────────────────────────────────────────────────

describe('groupByCategory', () => {
  it('groups in display order and omits empty categories', () => {
    const groups = groupByCategory([
      entry({ id: 'a', category: 'social', service_name: 'X' }),
      entry({ id: 'b', category: 'hosting', service_name: 'Vercel' }),
      entry({ id: 'c', category: 'hosting', service_name: 'Railway' }),
    ]);
    expect(groups.map(g => g.category)).toEqual(['hosting', 'social']);
    expect(groups[0].entries.map(e => e.id)).toEqual(['b', 'c']);
  });

  it('buckets an unknown category under "other" instead of dropping it', () => {
    const groups = groupByCategory([
      entry({ id: 'a', category: 'quantum' as StackEntry['category'] }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].category).toBe('other');
    expect(groups[0].entries[0].id).toBe('a');
  });

  it('returns [] for no entries', () => {
    expect(groupByCategory([])).toEqual([]);
  });
});
