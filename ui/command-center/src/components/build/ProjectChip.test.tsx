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

const hooks = vi.hoisted(() => ({
  useProjects: null as null | (() => Record<string, unknown>),
  navigateToTool: vi.fn(),
}));

vi.mock('./useProjects', () => ({
  useProjects: () => (hooks.useProjects
    ? hooks.useProjects()
    : {
        projects: projects.list,
        loading: false,
        error: false,
        refresh: vi.fn(),
        retry: vi.fn(),
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
  return { useCommandCenter, navigateToTool: hooks.navigateToTool };
});

import { ProjectChip } from './ProjectChip';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  hooks.useProjects = null;
  hooks.navigateToTool.mockReset();
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
    expect(claude).toBeTruthy();
    await act(async () => { claude!.focus(); });
    expect(document.querySelector('[role="tooltip"]')?.textContent ?? '').toContain('subscription');
    const permagent = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Permagent');
    expect(permagent).toBeTruthy();
    await act(async () => { permagent!.focus(); });
    expect(document.querySelector('[role="tooltip"]')?.textContent ?? '').toMatch(/not cheaper than Claude\/Codex/i);
  });
});

// ── Empty ≠ Error ≠ Loading ─────────────────────────────────────────────────
//
// The chip is the only way to launch a coding agent against a project. It used
// to `return null` on a daemon failure and on an empty list alike, so the entry
// point to the feature simply wasn't there and nothing said why.

function chip(): HTMLButtonElement {
  return container.querySelector('[data-testid="project-chip"]') as HTMLButtonElement;
}

function render() {
  return act(async () => {
    root.render(<ProjectChip onLaunch={vi.fn()} onVisitSite={vi.fn()} />);
  });
}

describe('ProjectChip states', () => {
  it('stays on screen and names the failure when the daemon is unreachable', async () => {
    const retry = vi.fn();
    hooks.useProjects = () => ({ projects: [], loading: false, error: true, refresh: vi.fn(), retry, touch: vi.fn() });
    await render();

    expect(chip()).toBeTruthy();
    await act(async () => { chip().click(); });
    expect(container.textContent).toMatch(/Couldn't load your projects/i);

    const retryBtn = Array.from(container.querySelectorAll('button')).find(b => /Retry/i.test(b.textContent ?? ''))!;
    expect(retryBtn).toBeTruthy();
    await act(async () => { retryBtn.click(); });
    expect(retry).toHaveBeenCalled();
  });

  it('an empty list names the action that fills it, and is not an error', async () => {
    hooks.useProjects = () => ({ projects: [], loading: false, error: false, refresh: vi.fn(), retry: vi.fn(), touch: vi.fn() });
    await render();

    expect(chip()).toBeTruthy();
    await act(async () => { chip().click(); });
    expect(container.textContent).toMatch(/No active projects yet/i);
    expect(container.textContent).not.toMatch(/Couldn't load/i);

    const open = Array.from(container.querySelectorAll('button')).find(b => /Open Projects/i.test(b.textContent ?? ''))!;
    await act(async () => { open.click(); });
    expect(hooks.navigateToTool).toHaveBeenCalledWith('projects');
  });

  it('says it is still loading rather than showing nothing', async () => {
    hooks.useProjects = () => ({ projects: [], loading: true, error: false, refresh: vi.fn(), retry: vi.fn(), touch: vi.fn() });
    await render();

    expect(chip()).toBeTruthy();
    await act(async () => { chip().click(); });
    expect(container.textContent).toMatch(/Loading your projects/i);
    expect(container.textContent).not.toMatch(/No active projects yet/i);
    expect(container.textContent).not.toMatch(/Couldn't load/i);
  });
});
