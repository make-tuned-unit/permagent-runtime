/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('../../lib/api', () => ({ api: { getHarnessRunHistory: vi.fn() } }));
import { api } from '../../lib/api';
import { RecentRunsPanel } from './RecentRunsPanel';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.mocked(api.getHarnessRunHistory).mockResolvedValue(Array.from({ length: 100 }, (_, i) => ({
    runId: `run-${i}`, sessionId: `session-${i}`, status: 'succeeded',
    project: 'Fixture', promptTitle: `Inspect fixture number ${i}`, promptDigest: '',
    elapsedMs: 1200, spendUsd: 0, startedAt: '2026-10-07T12:00:00Z',
    evidence: `Evidence for ${i}`,
  })) as never);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });
const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === text)!;
const transcripts = () => [...host.querySelectorAll('button')].filter(b => b.textContent === 'Open transcript');

it('previews three of 100 runs and keeps all history/evidence inside a bounded scroll area', async () => {
  await act(async () => { root.render(<RecentRunsPanel />); });
  expect(transcripts()).toHaveLength(3);
  const panel = host.querySelector<HTMLElement>('[data-testid="recent-runs-panel"]')!;
  expect(panel.style.maxHeight).toBe('min(35%, 240px)');
  expect(panel.style.minHeight).toBe('0');
  const all = button('View all 100');
  const scroller = document.getElementById(all.getAttribute('aria-controls')!)!;
  expect(scroller.style.overflowY).toBe('auto');
  expect(scroller.style.minHeight).toBe('0');
  expect(all.getAttribute('aria-expanded')).toBe('false');
  act(() => all.click());
  expect(transcripts()).toHaveLength(100);
  expect(button('Show fewer').getAttribute('aria-expanded')).toBe('true');
  act(() => button('Evidence').click());
  expect(scroller.contains(host.querySelector('pre'))).toBe(true);
  expect(panel.style.maxHeight).toBe('min(35%, 240px)');
  act(() => button('Show fewer').click());
  expect(transcripts()).toHaveLength(3);
  expect(vi.mocked(api.getHarnessRunHistory)).toHaveBeenCalledTimes(1);
});

it('folds to a one-line header with the count, is keyboard-operable, and remembers it across a reload', async () => {
  localStorage.removeItem('permagent.build.recentRuns.collapsed');
  await act(async () => { root.render(<RecentRunsPanel />); });
  const toggle = () => host.querySelector<HTMLButtonElement>('[data-testid="recent-runs-toggle"]')!;
  expect(toggle().tagName).toBe('BUTTON'); // a real button: Enter/Space work natively
  expect(toggle().getAttribute('aria-expanded')).toBe('true'); // default expanded
  expect(toggle().textContent).toContain('Recent runs · 100');
  expect(transcripts()).toHaveLength(3);

  act(() => toggle().click());
  expect(toggle().getAttribute('aria-expanded')).toBe('false');
  expect(toggle().textContent).toContain('Recent runs · 100');
  const list = document.getElementById(toggle().getAttribute('aria-controls')!)!;
  expect(list.hidden).toBe(true);
  expect(host.textContent).not.toContain('View all');
  expect(localStorage.getItem('permagent.build.recentRuns.collapsed')).toBe('1');

  // A reload: a fresh mount reads the stored choice.
  act(() => root.unmount());
  root = createRoot(host);
  await act(async () => { root.render(<RecentRunsPanel />); });
  expect(toggle().getAttribute('aria-expanded')).toBe('false');

  act(() => toggle().click());
  expect(toggle().getAttribute('aria-expanded')).toBe('true');
  expect(localStorage.getItem('permagent.build.recentRuns.collapsed')).toBe('0');
  localStorage.removeItem('permagent.build.recentRuns.collapsed');
});

it('G10: a real pointer sequence on the header text, on the row, and on the count folds and unfolds; View all does not fold', async () => {
  localStorage.removeItem('permagent.build.recentRuns.collapsed');
  await act(async () => { root.render(<RecentRunsPanel />); });
  const toggle = () => host.querySelector<HTMLButtonElement>('[data-testid="recent-runs-toggle"]')!;
  const header = host.querySelector<HTMLElement>('[data-testid="recent-runs-header"]')!;
  const press = (el: Element) => act(() => {
    for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
      const Ctor = type.startsWith('pointer') && typeof PointerEvent !== 'undefined' ? PointerEvent : MouseEvent;
      el.dispatchEvent(new Ctor(type, { bubbles: true, cancelable: true, clientX: 625, clientY: 141, button: 0 }));
    }
  });
  const label = toggle().querySelector('span')!; // the visible header text
  press(label);
  expect(toggle().getAttribute('aria-expanded')).toBe('false');
  press(header); // the row padding / empty space
  expect(toggle().getAttribute('aria-expanded')).toBe('true');
  press(label);
  expect(toggle().getAttribute('aria-expanded')).toBe('false');
  expect(localStorage.getItem('permagent.build.recentRuns.collapsed')).toBe('1');
  press(header);
  expect(toggle().getAttribute('aria-expanded')).toBe('true');
  // View all is its own control and must not fold the panel.
  press(button('View all 100'));
  expect(toggle().getAttribute('aria-expanded')).toBe('true');
  expect(toggle().style.transform).toBe('none');
  localStorage.removeItem('permagent.build.recentRuns.collapsed');
});
