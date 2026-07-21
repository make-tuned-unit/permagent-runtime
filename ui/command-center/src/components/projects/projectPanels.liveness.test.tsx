/**
 * @vitest-environment jsdom
 *
 * Multi-client liveness (#629) — component-side wiring: the project panels
 * genuinely REFETCH when the store's revision signals bump (the signals the
 * /events handler drives via livenessSync; that seam has its own tests in
 * lib/livenessSync.test.ts). Together the two files cover the full lane:
 * daemon frame → store rev → panel refetch.
 *
 * Covers DocumentsPanel/MemoriesPanel/NotesPanel on `projectsRev` and
 * PeoplePanel on `peopleRev`.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { listProjectDocuments, listProjectMemories, listProjectNotes, apiFetch } = vi.hoisted(() => ({
  listProjectDocuments: vi.fn(),
  listProjectMemories: vi.fn(),
  listProjectNotes: vi.fn(),
  apiFetch: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { listProjectDocuments, listProjectMemories, listProjectNotes },
  apiFetch,
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { DocumentsPanel } from './DocumentsPanel';
import { MemoriesPanel } from './MemoriesPanel';
import { NotesPanel } from './NotesPanel';
import { PeoplePanel } from './PeoplePanel';
import { useCommandCenter } from '../../lib/store';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const project = { id: 'p1', name: 'Proj' } as Project;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  listProjectDocuments.mockReset().mockResolvedValue([]);
  listProjectMemories.mockReset().mockResolvedValue([]);
  listProjectNotes.mockReset().mockResolvedValue([]);
  apiFetch.mockReset().mockResolvedValue([]);
  useCommandCenter.setState({ projectsRev: 0, peopleRev: 0 });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(ui: React.ReactElement) {
  await act(async () => root.render(ui));
}

/** Bump a store rev inside act so the panel's effect re-runs. */
async function bump(field: 'projectsRev' | 'peopleRev') {
  await act(async () => {
    useCommandCenter.setState(s => ({ [field]: s[field] + 1 }) as never);
  });
}

describe('project panels refetch on projectsRev (driven by project_changed)', () => {
  it('DocumentsPanel re-reads the documents list', async () => {
    await render(<DocumentsPanel project={project} />);
    expect(listProjectDocuments).toHaveBeenCalledTimes(1);
    await bump('projectsRev');
    expect(listProjectDocuments).toHaveBeenCalledTimes(2);
  });

  it('MemoriesPanel re-reads the memories list', async () => {
    await render(<MemoriesPanel project={project} />);
    expect(listProjectMemories).toHaveBeenCalledTimes(1);
    await bump('projectsRev');
    expect(listProjectMemories).toHaveBeenCalledTimes(2);
  });

  it('NotesPanel re-reads the notes list', async () => {
    await render(<NotesPanel project={project} />);
    expect(listProjectNotes).toHaveBeenCalledTimes(1);
    await bump('projectsRev');
    expect(listProjectNotes).toHaveBeenCalledTimes(2);
  });
});

describe('PeoplePanel refetches on peopleRev (driven by person_changed)', () => {
  it('re-reads the people list when the rev bumps', async () => {
    await render(<PeoplePanel project={project} />);
    const before = apiFetch.mock.calls.filter(c => String(c[0]).includes('/people')).length;
    expect(before).toBeGreaterThan(0);
    await bump('peopleRev');
    const after = apiFetch.mock.calls.filter(c => String(c[0]).includes('/people')).length;
    expect(after).toBe(before + 1);
  });
});
