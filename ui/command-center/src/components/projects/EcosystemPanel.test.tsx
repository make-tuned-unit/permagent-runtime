/**
 * @vitest-environment jsdom
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { EcosystemPanel } from './EcosystemPanel';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const writeText = vi.fn();
const project = { id: 'project/1', name: 'Acme' } as Project;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-07-22T14:00:00Z'));
  apiFetch.mockReset().mockResolvedValue({
    competitors: [{
      id: 'c1', kind: 'competitor', name: 'Rival',
      note: 'Competes on automation', source_url: 'https://rival.example',
      created_at: '2026-07-22T12:00:00Z',
    }],
    partners: [{
      id: 'p1', kind: 'partner', name: 'Ally',
      note: null, source_url: 'https://ally.example',
      created_at: '2026-07-21T12:00:00Z',
    }],
    ecosystem: [{
      id: 'a1', kind: 'adjacent', name: 'Neighbor',
      note: 'Adjacent workflow', source_url: 'https://neighbor.example',
      created_at: '2026-07-20T12:00:00Z',
    }],
  });
  writeText.mockReset().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('renders grouped cited items and prepares the refresh prompt', async () => {
  await act(async () => root.render(<EcosystemPanel project={project} />));

  expect(apiFetch).toHaveBeenCalledWith('/api/projects/project%2F1/intel');
  expect(container.textContent).toContain('Competitors');
  expect(container.textContent).toContain('Partners');
  expect(container.textContent).toContain('Ecosystem');
  expect(container.textContent).toContain('Rival');
  expect(container.textContent).toContain('Last researched 2h ago');
  expect(Array.from(container.querySelectorAll('a')).map(a => a.href)).toEqual([
    'https://rival.example/',
    'https://ally.example/',
    'https://neighbor.example/',
  ]);

  const refresh = Array.from(container.querySelectorAll('button'))
    .find(button => button.textContent === 'Refresh intelligence');
  await act(async () => refresh?.click());
  expect(writeText).toHaveBeenCalledWith(
    'Refresh project intelligence for Acme: call research_project_intel with ' +
    'project "Acme", research its competitors, partners, and adjacent ecosystem ' +
    'with your web tools, then call propose_project_intel so I can review the findings in ' +
    'the Decision Inbox.',
  );
});

it('dismisses an item after the DELETE succeeds', async () => {
  await act(async () => root.render(<EcosystemPanel project={project} />));

  const dismiss = container.querySelector<HTMLButtonElement>('button[aria-label="Dismiss Rival"]');
  await act(async () => dismiss?.click());

  expect(apiFetch).toHaveBeenCalledWith(
    '/api/projects/project%2F1/intel/c1',
    { method: 'DELETE' },
  );
  expect(container.textContent).not.toContain('Rival');
  expect(container.textContent).toContain('Ally');
});

it('renders an unsafe source URL as inert text', async () => {
  apiFetch.mockResolvedValue({
    competitors: [{
      id: 'c1', kind: 'competitor', name: 'Unsafe Rival',
      note: null, source_url: 'javascript:alert(1)',
      created_at: '2026-07-22T12:00:00Z',
    }],
  });

  await act(async () => root.render(<EcosystemPanel project={project} />));

  expect(container.textContent).toContain('Source');
  expect(container.querySelector('a')).toBeNull();
});
