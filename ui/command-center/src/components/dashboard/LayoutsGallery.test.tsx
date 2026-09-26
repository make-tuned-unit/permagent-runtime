/** @vitest-environment jsdom */
/**
 * dagK home-templates lane — the Layouts gallery: 5 templates + "Custom
 * (yours)", "Try" opens the current-vs-proposed overlay (P9: never a silent
 * write), and Apply calls the persist callback with the REALIZED layout
 * (comingSoon cards dropped, per §3's graceful-degrade rule).
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const apiFetch = vi.fn();
vi.mock('../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

import { LayoutsGallery } from './LayoutsGallery';
import { DASHBOARD_TEMPLATES, realizedLayout } from './templates';
import type { DashboardLayoutData } from './useLayout';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  apiFetch.mockRejectedValue(new Error('offline')); // gallery falls back to the static mirror
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = '';
});

const currentLayout: DashboardLayoutData = {
  cards: [{ id: 'timeline', type: 'timeline', position: { x: 0, y: 0 }, size: { w: 12, h: 6 }, visible: true }],
};

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

it('lists all five templates by name plus Custom (yours)', async () => {
  await act(async () => { root.render(<LayoutsGallery currentLayout={currentLayout} onApply={() => {}} onClose={() => {}} />); });
  await flush();
  for (const t of DASHBOARD_TEMPLATES) {
    expect(document.body.textContent).toContain(t.name);
  }
  expect(document.body.textContent).toContain('Custom (yours)');
});

it('Try opens the current-vs-proposed overlay without applying anything', async () => {
  const onApply = vi.fn();
  await act(async () => { root.render(<LayoutsGallery currentLayout={currentLayout} onApply={onApply} onClose={() => {}} />); });
  await flush();
  const tryButtons = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Try');
  await act(async () => { tryButtons[0].click(); });
  expect(document.body.textContent).toContain('Proposed');
  expect(onApply).not.toHaveBeenCalled();
});

it('Apply persists the REALIZED layout (comingSoon cards dropped) and closes', async () => {
  const onApply = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  await act(async () => { root.render(<LayoutsGallery currentLayout={currentLayout} onApply={onApply} onClose={onClose} />); });
  await flush();
  const tryButtons = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Try');
  // Weekly Story is the 3rd template and has comingSoon cards.
  const weeklyIndex = DASHBOARD_TEMPLATES.findIndex(t => t.id === 'weekly_story');
  await act(async () => { tryButtons[weeklyIndex].click(); });
  const applyButton = Array.from(document.querySelectorAll('button')).find(b => b.textContent?.includes('Apply'));
  await act(async () => { applyButton?.click(); });
  await flush();
  const weeklyStory = DASHBOARD_TEMPLATES.find(t => t.id === 'weekly_story')!;
  expect(onApply).toHaveBeenCalledWith(realizedLayout(weeklyStory));
  expect(onClose).toHaveBeenCalled();
});

it('Keep current dismisses the overlay with no apply call', async () => {
  const onApply = vi.fn();
  await act(async () => { root.render(<LayoutsGallery currentLayout={currentLayout} onApply={onApply} onClose={() => {}} />); });
  await flush();
  const tryButtons = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Try');
  await act(async () => { tryButtons[0].click(); });
  const keepButton = Array.from(document.querySelectorAll('button')).find(b => b.textContent === 'Keep current');
  await act(async () => { keepButton?.click(); });
  expect(onApply).not.toHaveBeenCalled();
  expect(document.body.textContent).not.toContain('Proposed');
});
