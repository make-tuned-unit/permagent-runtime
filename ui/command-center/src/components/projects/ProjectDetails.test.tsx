/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { listProjectMemories, listProjectNotes, apiFetch } = vi.hoisted(() => ({
  listProjectMemories: vi.fn(),
  listProjectNotes: vi.fn(),
  apiFetch: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { listProjectMemories, listProjectNotes },
  apiFetch,
}));

import { ActivityPanel } from './ProjectDetails';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const project = { id: 'project/a', name: 'Project A' } as Project;
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  listProjectMemories.mockReset().mockResolvedValue([]);
  listProjectNotes.mockReset().mockResolvedValue([]);
  apiFetch.mockReset().mockResolvedValue([]);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('Project details activity', () => {
  it('loads mounted project endpoints and orders their real rows newest first', async () => {
    listProjectMemories.mockResolvedValue([{
      id: 'm1', description: 'Older memory', content: 'memory body', associated_at: '2026-07-20T10:00:00Z',
    }]);
    listProjectNotes.mockResolvedValue([{
      id: 'n1', title: 'Newest note', body: 'note body', created_at: '2026-07-22T10:00:00Z',
    }]);
    apiFetch.mockResolvedValue([{
      id: 'c1', title: 'Middle task', updatedAt: '2026-07-21T10:00:00Z', archivedAt: null,
    }]);

    await act(async () => root.render(<ActivityPanel project={project} />));

    expect(listProjectMemories).toHaveBeenCalledWith('project/a');
    expect(listProjectNotes).toHaveBeenCalledWith('project/a');
    expect(apiFetch).toHaveBeenCalledWith('/api/projects/project%2Fa/cards');
    expect(container.textContent).toContain('Newest note');
    expect(container.textContent).toContain('Middle task');
    expect(container.textContent).toContain('Older memory');
    expect(container.textContent!.indexOf('Newest note')).toBeLessThan(container.textContent!.indexOf('Middle task'));
  });

  it('surfaces endpoint failure with a retry action', async () => {
    listProjectMemories.mockRejectedValue(new Error('offline'));
    await act(async () => root.render(<ActivityPanel project={project} />));
    expect(container.textContent).toContain("Couldn't load activity.");
    expect(container.querySelector('button')?.textContent).toBe('Retry');
  });
});
