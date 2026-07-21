/**
 * Inbox routing (#395) — wire-contract + pure-logic tests.
 *
 * Pins the client to the daemon's actual `POST /api/inbox/{id}/route` contract
 * (crates/goose-server/src/routes/inbox.rs): snake_case `project_id`, the
 * destination strings the handler matches on, and the response envelope the
 * panel renders from.
 */

import { describe, expect, it, vi } from 'vitest';

// Isolate the daemon fetch — the real ../../lib/api module has load-time side effects.
vi.mock('../../lib/api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from '../../lib/api';
import {
  canRoute,
  describeRouteResult,
  fetchRoutableProjects,
  needsProject,
  routeInboxFile,
  statusLabel,
  type RouteInboxResponse,
} from './inboxRouting';

const mockApiFetch = apiFetch as unknown as ReturnType<typeof vi.fn>;

const wireFile = (status: string) => ({
  id: 'f-1',
  filename: 'flyer.png',
  original_url: null,
  content_type: 'image/png',
  size_bytes: 2048,
  disk_path: 'flyer.png',
  status,
  project_id: null,
  created_at: '2026-07-20T00:00:00Z',
});

const wireResponse = (over: Partial<RouteInboxResponse>): RouteInboxResponse => ({
  file: wireFile('routed'),
  destination: 'project',
  summary: null,
  document_id: null,
  card_id: null,
  ...over,
});

// NOTE: no beforeEach mockReset/mockClear — clearing a vitest module-mock's
// state breaks its settled-promise tracking, so a later rejected implementation
// is reported as an unhandled rejection instead of an assertion pass. Each test
// sets its own implementation (mockResolvedValue overrides) and asserts on
// `lastCall` / a distinct call shape, so cumulative call history is harmless.

describe('routeInboxFile — wire contract', () => {
  it('brain: POSTs the id-scoped route path with destination only (no project_id key)', async () => {
    mockApiFetch.mockResolvedValue(wireResponse({ destination: 'brain', summary: 'A gist' }));
    const resp = await routeInboxFile('f-1', 'brain');
    expect(mockApiFetch.mock.lastCall).toEqual([
      '/api/inbox/f-1/route',
      {
        method: 'POST',
        body: JSON.stringify({ destination: 'brain' }),
      },
    ]);
    expect(resp.summary).toBe('A gist');
  });

  it('project/scheduler: body carries snake_case project_id', async () => {
    mockApiFetch.mockResolvedValue(wireResponse({ destination: 'scheduler', card_id: 'c-9' }));
    await routeInboxFile('f-1', 'scheduler', 'p-7');
    expect(mockApiFetch.mock.lastCall).toEqual([
      '/api/inbox/f-1/route',
      {
        method: 'POST',
        body: JSON.stringify({ destination: 'scheduler', project_id: 'p-7' }),
      },
    ]);
  });

  it('URL-encodes the file id', async () => {
    mockApiFetch.mockResolvedValue(wireResponse({}));
    await routeInboxFile('a/b c', 'brain');
    expect(mockApiFetch.mock.lastCall?.[0]).toBe('/api/inbox/a%2Fb%20c/route');
  });

  it('non-2xx rejects (apiFetch throws) and the rejection propagates', async () => {
    // Lazy rejection: mockRejectedValue creates the rejected promise eagerly,
    // which vitest's unhandled-rejection detector flags before the assertion.
    mockApiFetch.mockImplementation(() =>
      Promise.reject(new Error("destination 'project' requires a project_id")),
    );
    await expect(routeInboxFile('f-1', 'project')).rejects.toThrow('requires a project_id');
  });
});

describe('fetchRoutableProjects — wire contract', () => {
  it('reads the same /api/projects list the app uses', async () => {
    mockApiFetch.mockResolvedValue([]);
    await fetchRoutableProjects();
    expect(mockApiFetch).toHaveBeenCalledWith('/api/projects');
  });
});

describe('needsProject / canRoute', () => {
  it('project + scheduler are project-scoped; brain is not', () => {
    expect(needsProject('brain')).toBe(false);
    expect(needsProject('project')).toBe(true);
    expect(needsProject('scheduler')).toBe(true);
  });

  it('every live status routes; deleted is inert', () => {
    for (const s of ['received', 'ingested', 'routed']) expect(canRoute(s)).toBe(true);
    expect(canRoute('deleted')).toBe(false);
  });
});

describe('statusLabel', () => {
  it('maps the four lifecycle statuses and passes unknowns through', () => {
    expect(statusLabel('received')).toBe('New');
    expect(statusLabel('ingested')).toBe('In Brain');
    expect(statusLabel('routed')).toBe('Routed');
    expect(statusLabel('deleted')).toBe('Deleted');
    expect(statusLabel('weird')).toBe('weird');
  });
});

describe('describeRouteResult — the per-row result line', () => {
  it('brain with a summary quotes the gist, truncated', () => {
    const line = describeRouteResult(
      wireResponse({ destination: 'brain', summary: 'x'.repeat(200) }),
    );
    expect(line).toContain('Read into the Brain');
    expect(line.length).toBeLessThan(130);
    expect(line).toContain('…');
  });

  it('brain with an empty summary says the file was visual (nothing stored)', () => {
    const line = describeRouteResult(wireResponse({ destination: 'brain', summary: '' }));
    expect(line).toContain('visual');
    expect(line).not.toContain('Read into the Brain —');
  });

  it('project names the target project when known', () => {
    expect(
      describeRouteResult(wireResponse({ destination: 'project', document_id: 'd-1' }), 'Launch'),
    ).toBe('Filed as a document in Launch');
    expect(
      describeRouteResult(wireResponse({ destination: 'project', document_id: 'd-1' })),
    ).toBe('Filed as a document in the project');
  });

  it('scheduler describes the social post draft', () => {
    const line = describeRouteResult(
      wireResponse({ destination: 'scheduler', card_id: 'c-1' }),
      'Launch',
    );
    expect(line).toContain('social post');
    expect(line).toContain('Launch');
  });
});
