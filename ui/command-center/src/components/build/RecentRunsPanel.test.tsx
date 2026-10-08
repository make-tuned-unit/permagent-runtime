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
