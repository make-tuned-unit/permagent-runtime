/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const projects = vi.hoisted(() => ({
  list: [
    {
      id: 'p1',
      slug: 'demo',
      name: 'Demo',
      description: '',
      status: 'active',
      rootPath: '/tmp/demo',
      siteUrl: null,
      repoUrl: null,
      notes: '',
      tags: [] as string[],
      createdAt: '2026-08-01T00:00:00Z',
      updatedAt: '2026-08-01T00:00:00Z',
      lastOpenedAt: '2026-08-01T00:00:00Z',
    },
  ],
}));

vi.mock('./useProjects', () => ({
  useProjects: () => ({
    projects: projects.list,
    loading: false,
    refresh: vi.fn(),
    touch: vi.fn(),
  }),
}));

vi.mock('../../lib/store', () => {
  const state = {
    pushBrowserOverlay: vi.fn(),
    popBrowserOverlay: vi.fn(),
  };
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});

import { ProjectChip } from './ProjectChip';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ProjectChip', () => {
  it('teaches subscription-first on the Projects launch row', async () => {
    const onLaunch = vi.fn();
    await act(async () => {
      root.render(<ProjectChip onLaunch={onLaunch} onVisitSite={vi.fn()} />);
    });

    const chip = Array.from(container.querySelectorAll('button')).find(b =>
      (b.textContent ?? '').includes('Projects'),
    );
    expect(chip).toBeTruthy();
    await act(async () => { chip!.click(); });

    const demo = Array.from(container.querySelectorAll('button')).find(b =>
      (b.textContent ?? '').includes('Demo'),
    );
    expect(demo).toBeTruthy();
    await act(async () => { demo!.click(); });

    expect(container.textContent).toContain('nothing extra');
    const claude = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Claude');
    expect(claude?.getAttribute('title') ?? '').toContain('subscription');
    const permagent = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Permagent');
    expect(permagent?.getAttribute('title') ?? '').toMatch(/not cheaper than Claude\/Codex/i);
  });
});
