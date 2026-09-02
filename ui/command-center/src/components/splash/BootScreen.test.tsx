/** @vitest-environment jsdom
 *
 * BootScreen — the daemon-connecting state between the logo splash and the
 * running app. Replaces a blank filled `<div>` that silently guessed
 * "wizard" after ~10s of failure; this covers the three honest states
 * (connecting / retrying / failed), that there is no fake progress
 * indicator, and that the Retry button actually re-attempts the connection.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({ api: { getConfig: vi.fn() } }));

import { api } from '../../lib/api';
import { BootScreen } from './BootScreen';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const getConfigMock = vi.mocked(api.getConfig);

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  getConfigMock.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
}

/** Run the connect loop out to its end (success, or exhausted retries). */
async function runLoop() {
  await flush();
  for (let i = 0; i < 11; i += 1) await advance(1000);
}

describe('BootScreen', () => {
  it('calls onReady as soon as the daemon answers', async () => {
    getConfigMock.mockResolvedValue({ config: { wizard_complete: true } });
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} />); });
    await flush();
    expect(onReady).toHaveBeenCalledWith(true);
  });

  it('passes wizard_complete through untouched when false', async () => {
    getConfigMock.mockResolvedValue({ config: { wizard_complete: false } });
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} />); });
    await flush();
    expect(onReady).toHaveBeenCalledWith(false);
  });

  it('shows honest connecting copy before the daemon answers, and no fake progress indicator', async () => {
    getConfigMock.mockReturnValue(new Promise(() => { /* never resolves */ }));
    act(() => { root.render(<BootScreen onReady={vi.fn()} />); });
    await flush();
    expect(container.textContent).toContain('Connecting to Permagent');
    expect(container.querySelector('progress')).toBeNull();
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
  });

  it('shows a real attempt count while retrying, not a fake progress bar', async () => {
    getConfigMock.mockRejectedValue(new Error('ECONNREFUSED'));
    act(() => { root.render(<BootScreen onReady={vi.fn()} />); });
    await flush();
    await advance(1000); // second attempt starts -> 'retrying'
    expect(container.textContent).toContain('Still connecting');
    expect(container.textContent).toMatch(/attempt 2 of 10/);
  });

  it('after exhausting retries, shows an honest failure state with a Retry button (Button primitive)', async () => {
    getConfigMock.mockRejectedValue(new Error('ECONNREFUSED'));
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} />); });
    await runLoop();

    expect(container.textContent).toContain('Could not reach the daemon');
    const retryBtn = container.querySelector('button.pa-btn');
    expect(retryBtn).not.toBeNull();
    expect(retryBtn?.textContent).toContain('Retry');
    expect(onReady).not.toHaveBeenCalled();
  });

  it('Retry re-attempts the connection and can succeed', async () => {
    getConfigMock.mockRejectedValue(new Error('down'));
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} />); });
    await runLoop();
    expect(container.textContent).toContain('Could not reach the daemon');

    getConfigMock.mockResolvedValue({ config: { wizard_complete: false } });
    const retryBtn = container.querySelector('button.pa-btn') as HTMLButtonElement;
    act(() => { retryBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await flush();

    expect(onReady).toHaveBeenCalledWith(false);
  });
});
