/**
 * @vitest-environment jsdom
 *
 * The People graph's key.
 *
 * This view is built on a premise it never stated: you at the centre, everyone
 * else grouped by the projects you share, and a larger tinted face for the
 * person who bridges two of those groups. Someone meeting it saw a ring of
 * faces and had to infer all of that — or, more likely, not.
 *
 * The Canvas itself is three.js, so it is stubbed here; the key is DOM chrome
 * outside it, which is the point — it is readable, tabbable and testable,
 * unlike everything it explains.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
vi.mock('@react-three/drei', () => ({
  Html: () => null,
  Line: () => null,
  OrbitControls: () => null,
}));

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PeopleGraph } from './PeopleGraphCanvas';
import { QUIET_AFTER_DAYS } from './contactAge';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  apiFetch.mockReset().mockResolvedValue([
    {
      entity_uuid: 'p1',
      display_name: 'Jane Doe',
      role: null,
      company: null,
      photo_url: null,
      last_contact_at: null,
      projects: [{ project_id: 'a', project_name: 'Acme' }],
    },
  ]);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

async function render() {
  await act(async () => root.render(<PeopleGraph />));
}

const legend = () => container.querySelector('[data-testid="canvas-legend"]');

describe('People graph key', () => {
  it('teaches the premise the layout is built on', async () => {
    await render();
    const text = legend()!.textContent ?? '';
    expect(text).toContain('is you');
    expect(text).toContain('grouped by a project you share');
  });

  it('explains what a bridge person is, and what a line means', async () => {
    await render();
    const text = legend()!.textContent ?? '';
    expect(text).toContain('more than one project');
    expect(text).toContain('bridge');
    expect(text).toContain('a project they both work on');
  });

  it('names the dimming convention with its real threshold', async () => {
    await render();
    expect(legend()!.textContent).toContain(`${QUIET_AFTER_DAYS} days`);
  });

  it('teaches the gestures this canvas has', async () => {
    await render();
    const text = legend()!.textContent ?? '';
    expect(text).toContain('Drag');
    expect(text).toContain('Scroll');
    expect(text).toContain('Right-drag');
    expect(text).toContain('Click a face');
  });

  it('goes quiet for good once dismissed', async () => {
    await render();
    const dismiss = container.querySelector<HTMLButtonElement>('[data-testid="canvas-legend-dismiss"]')!;
    act(() => dismiss.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(legend()).toBeNull();
    act(() => root.unmount());
    root = createRoot(container);
    await render();
    expect(legend()).toBeNull();
  });
});
