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
import { Children, isValidElement, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { canvas } = vi.hoisted(() => ({ canvas: vi.fn() }));
vi.mock('@react-three/fiber', () => ({ Canvas: (props: { children: ReactNode }) => { canvas(props); return null; } }));
vi.mock('@react-three/drei', () => ({
  Html: () => null,
  Line: () => null,
  OrbitControls: () => null,
}));

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PeopleGraph, YouMarker } from './PeopleGraphCanvas';
import * as graphLayout from './peopleGraph';
import { faceHalfExtent } from './graphFit';
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

async function mount() {
  await act(async () => root.render(<PeopleGraph />));
}

const legend = () => container.querySelector('[data-testid="canvas-legend"]');
const keyButton = () => container.querySelector<HTMLButtonElement>('[data-testid="canvas-legend-open"]');

/** Mount and open the key with its own "Key" button (it starts collapsed). */
async function render() {
  await mount();
  await act(async () => keyButton()!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

describe('People graph key', () => {
  // QA 2026-09-24: open by default, the key covered ~45% of a 660x560 graph
  // and faces were drawn over its text. It now waits behind its button.
  it('starts collapsed on a first visit, behind its "Key" button', async () => {
    await mount();
    expect(legend()).toBeNull();
    expect(keyButton()!.textContent).toBe('Key');
  });

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
    await mount();
    expect(legend()).toBeNull();
  });
});


it('filters in place without relaying out the network or losing the central You node', async () => {
  const layout = vi.spyOn(graphLayout, 'layoutPeopleGraph');
  try {
    await mount();
    const count = layout.mock.calls.length;
    const input = container.querySelector<HTMLInputElement>('[aria-label="Filter people"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'no match');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(layout.mock.calls.length).toBe(count);
    expect(container.textContent).toContain('No people match');
    const scene = canvas.mock.calls[canvas.mock.calls.length - 1][0].children;
    const nodeKinds = Children.toArray(scene).filter(isValidElement).flatMap(child => {
      const node = (child.props as { node?: { kind: string } }).node;
      return node ? [node.kind] : [];
    });
    expect(nodeKinds).toEqual(['you']);
  } finally { layout.mockRestore(); }
});

it.each([480, 1187, 1600])('keeps the You hub inside its pixel budget at %ipx viewport width', async width => {
  // A one-person staging graph zoomed close enough that the old world-space
  // sphere covered most of the canvas. Faces and fitting use screen pixels.
  container.style.width = `${width}px`;
  await act(async () => root.render(<YouMarker name="You" color="#40c5db" />));
  const hub = container.querySelector<HTMLElement>('[role="img"]')!;
  const budget = faceHalfExtent(graphLayout.layoutPeopleGraph([]).nodes[0]);
  expect(hub.getAttribute('aria-label')).toBe('You, center of your network');
  expect(hub.textContent).toBe('You');
  expect(hub.style.width).toBe('36px');
  expect(hub.style.height).toBe('36px');
  expect(parseFloat(hub.style.width)).toBeLessThanOrEqual(budget.hw * 2);
  expect(parseFloat(hub.style.height)).toBeLessThanOrEqual(budget.hh * 2);
  expect(hub.style.boxSizing).toBe('border-box');
  expect(container.querySelector('button')).toBeNull();
});
