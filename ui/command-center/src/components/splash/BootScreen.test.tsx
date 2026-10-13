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

const eventMock = vi.hoisted(() => ({
  handlers: [] as Array<(e: { payload: unknown }) => void>,
  unlisten: vi.fn(),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    eventMock.handlers.push(cb);
    return Promise.resolve(eventMock.unlisten);
  },
}));

import { api } from '../../lib/api';
import { BootScreen } from './BootScreen';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const getConfigMock = vi.mocked(api.getConfig);

let container: HTMLDivElement;
let root: Root;

const originalTimeOrigin = Object.getOwnPropertyDescriptor(performance, 'timeOrigin');
function setTimeOrigin(value: number) {
  Object.defineProperty(performance, 'timeOrigin', { configurable: true, value });
}

beforeEach(() => {
  vi.useFakeTimers();
  setTimeOrigin(Date.now());
  window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now();
  getConfigMock.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  delete window.__PERMAGENT_LAUNCH_EPOCH_MS__;
  if (originalTimeOrigin) Object.defineProperty(performance, 'timeOrigin', originalTimeOrigin);
  else Reflect.deleteProperty(performance, 'timeOrigin');
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
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />); });
    await flush();
    expect(onReady).toHaveBeenCalledWith('app');
  });

  it('routes a first-run config to the wizard', async () => {
    getConfigMock.mockResolvedValue({ config: { wizard_complete: false } });
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />); });
    await flush();
    expect(onReady).toHaveBeenCalledWith('wizard');
  });

  it('shows honest connecting copy before the daemon answers, and no fake progress indicator', async () => {
    getConfigMock.mockReturnValue(new Promise(() => { /* never resolves */ }));
    act(() => { root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />); });
    await flush();
    expect(container.textContent).toContain('Connecting to Permagent');
    expect(container.querySelector('progress')).toBeNull();
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
  });

  it('shows a real attempt count while retrying, not a fake progress bar', async () => {
    getConfigMock.mockRejectedValue(new Error('ECONNREFUSED'));
    act(() => { root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />); });
    await flush();
    await advance(1000); // second attempt starts -> 'retrying'
    expect(container.textContent).toContain('Still connecting');
    expect(container.textContent).toMatch(/attempt 2 of 10/);
  });

  it('after exhausting retries, shows an honest failure state with a Retry button (Button primitive)', async () => {
    getConfigMock.mockRejectedValue(new Error('ECONNREFUSED'));
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />); });
    await runLoop();

    expect(container.textContent).toContain("Couldn't reach Permagent's background service");
    expect(container.textContent).not.toMatch(/daemon/i);
    const retryBtn = container.querySelector('button.pa-btn');
    expect(retryBtn).not.toBeNull();
    expect(retryBtn?.textContent).toContain('Retry');
    expect(onReady).not.toHaveBeenCalled();
  });

  it('Retry re-attempts the connection and can succeed', async () => {
    getConfigMock.mockRejectedValue(new Error('down'));
    const onReady = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />); });
    await runLoop();
    expect(container.textContent).toContain("Couldn't reach Permagent's background service");
    expect(container.textContent).not.toMatch(/daemon/i);

    getConfigMock.mockResolvedValue({ config: { wizard_complete: false } });
    const retryBtn = container.querySelector('button.pa-btn') as HTMLButtonElement;
    act(() => { retryBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await flush();

    expect(onReady).toHaveBeenCalledWith('wizard');
  });

  it('routes a malformed config away from onboarding, through the shared classifier', async () => {
    getConfigMock.mockResolvedValue({ config: { wizard_complete: 'false' } });
    const onReady = vi.fn();
    const onMalformed = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={onMalformed} />); });
    await flush();
    expect(onMalformed).toHaveBeenCalledTimes(1);
    expect(onReady).not.toHaveBeenCalled();
  });

  it('does not restart its connect loop when the parent re-renders with the same callbacks', async () => {
    getConfigMock.mockReturnValue(new Promise(() => { /* never resolves */ }));
    const onReady = vi.fn();
    const onMalformed = vi.fn();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={onMalformed} />); });
    await flush();
    act(() => { root.render(<BootScreen onReady={onReady} onMalformed={onMalformed} />); });
    await flush();
    expect(getConfigMock).toHaveBeenCalledTimes(1);
  });
});


describe('absolute startup deadline', () => {
  it('spends shell startup time: 8.5s remain after a 1.5s launch, with Retry exactly at launch + 10s', async () => {
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now() - 1500;
    let answer!: (value: never) => void;
    getConfigMock.mockImplementation(() => new Promise(resolve => { answer = resolve; }));
    const onReady = vi.fn();
    act(() => root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain('8.5s remaining');
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(getConfigMock.mock.calls[0][0]?.aborted).toBe(true);
    expect(container.textContent).toContain('6.5s remaining');
    await act(async () => { await vi.advanceTimersByTimeAsync(6499); });
    expect(container.querySelector('button')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    const retry = container.querySelector('button') as HTMLButtonElement;
    expect(retry.textContent).toContain('Retry');
    expect(retry.disabled).toBe(false);
    expect(Date.now()).toBe(window.__PERMAGENT_LAUNCH_EPOCH_MS__ + 10_000);
    expect(getConfigMock.mock.calls.every(([signal]) => signal?.aborted)).toBe(true);
    await act(async () => { answer({ config: { wizard_complete: true } } as never); });
    expect(onReady).not.toHaveBeenCalled();
  });

  it('mounts directly into visible recovery after an 11s shell startup, without starting a request', () => {
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now() - 11_000;
    setTimeOrigin(window.__PERMAGENT_LAUNCH_EPOCH_MS__ + 3000); // first load, page began 3s after launch
    act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain("Couldn't reach Permagent's background service");
    expect(container.textContent).not.toContain('remaining');
    const retry = container.querySelector('button') as HTMLButtonElement;
    expect(retry.textContent).toContain('Retry');
    expect(retry.disabled).toBe(false);
    const screen = container.firstElementChild as HTMLElement;
    expect(screen.style.opacity).toBe('1');
    expect(screen.style.transition).toBe('none');
    expect(getConfigMock).not.toHaveBeenCalled();
  });

  it('uses navigation timeOrigin in a web build without Tauri', async () => {
    delete window.__PERMAGENT_LAUNCH_EPOCH_MS__;
    expect('__TAURI_INTERNALS__' in window).toBe(false);
    const original = Object.getOwnPropertyDescriptor(performance, 'timeOrigin');
    Object.defineProperty(performance, 'timeOrigin', { configurable: true, value: Date.now() - 1500 });
    try {
      getConfigMock.mockReturnValue(new Promise(() => {}));
      act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
      expect(container.textContent).toContain('8.5s remaining');
      await act(async () => { await vi.advanceTimersByTimeAsync(8499); });
      expect(container.querySelector('button')).toBeNull();
      await act(async () => { await vi.advanceTimersByTimeAsync(1); });
      expect(container.querySelector('button')?.textContent).toContain('Retry');
    } finally {
      if (original) Object.defineProperty(performance, 'timeOrigin', original);
      else Reflect.deleteProperty(performance, 'timeOrigin');
    }
  });

  it('a reload 60s after launch gets a fresh deadline from its own page origin and tries the daemon', async () => {
    const launch = Date.now() - 60_000;
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = launch;
    setTimeOrigin(Date.now() - 1000); // this page began 59s after launch
    getConfigMock.mockResolvedValue({ config: { wizard_complete: true } });
    const onReady = vi.fn();
    act(() => root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain('9s remaining');
    await flush();
    expect(getConfigMock).toHaveBeenCalledTimes(1);
    expect(onReady).toHaveBeenCalledWith('app');
  });

  it('a reload with a hung daemon fails at page origin + 10s, not at launch + 10s', async () => {
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now() - 60_000;
    const origin = Date.now();
    setTimeOrigin(origin);
    getConfigMock.mockReturnValue(new Promise(() => {}));
    act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(9999); });
    expect(container.querySelector('button')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    expect(Date.now()).toBe(origin + 10_000);
  });

  it('a web page whose origin is 11s after a stale/absent epoch is origin-based', async () => {
    delete window.__PERMAGENT_LAUNCH_EPOCH_MS__;
    setTimeOrigin(Date.now() - 1500);
    getConfigMock.mockResolvedValue({ config: { wizard_complete: true } });
    const onReady = vi.fn();
    act(() => root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain('8.5s remaining');
    await flush();
    expect(onReady).toHaveBeenCalledWith('app');
  });

  it('only an explicit Retry grants a fresh 10s window and permits successful recovery', async () => {
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now() - 11_000;
    setTimeOrigin(window.__PERMAGENT_LAUNCH_EPOCH_MS__ + 3000);
    const onReady = vi.fn();
    const onMalformed = vi.fn();
    getConfigMock.mockReturnValue(new Promise(() => {}));
    act(() => root.render(<BootScreen onReady={onReady} onMalformed={onMalformed} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(getConfigMock).not.toHaveBeenCalled();
    act(() => (container.querySelector('button') as HTMLButtonElement).click());
    expect(container.textContent).toContain('10s remaining');
    await act(async () => { await vi.advanceTimersByTimeAsync(9999); });
    expect(container.querySelector('button')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    getConfigMock.mockResolvedValue({ config: { wizard_complete: true } });
    act(() => (container.querySelector('button') as HTMLButtonElement).click());
    await flush();
    expect(onReady).toHaveBeenCalledTimes(1);
    expect(onReady).toHaveBeenCalledWith('app');
  });

  it('remounting cannot renew the process-launch deadline', async () => {
    window.__PERMAGENT_LAUNCH_EPOCH_MS__ = Date.now() - 1500;
    getConfigMock.mockReturnValue(new Promise(() => {}));
    act(() => root.render(<BootScreen key="first" onReady={vi.fn()} onMalformed={vi.fn()} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(8000); });
    act(() => root.render(<BootScreen key="second" onReady={vi.fn()} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain('0.5s remaining');
    const activeSignal = getConfigMock.mock.calls[getConfigMock.mock.calls.length - 1][0];
    await act(async () => { await vi.advanceTimersByTimeAsync(499); });
    expect(container.querySelector('button')).toBeNull();
    expect(activeSignal?.aborted).toBe(false);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    expect(activeSignal?.aborted).toBe(true);
  });

  it('aborts hung attempts and offers recovery at exactly 10s, ignoring late success', async () => {
    let answer!: (value: never) => void;
    getConfigMock.mockImplementation(() => new Promise(resolve => { answer = resolve; }));
    const onReady = vi.fn();
    act(() => root.render(<BootScreen onReady={onReady} onMalformed={vi.fn()} />));
    expect(container.textContent).toContain('10s remaining');
    await act(async () => { await vi.advanceTimersByTimeAsync(2000); });
    expect(getConfigMock.mock.calls[0][0]?.aborted).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(7999); });
    expect(container.querySelector('button')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    expect(getConfigMock).toHaveBeenCalledTimes(4);
    await act(async () => { answer({ config: { wizard_complete: true } } as never); });
    expect(onReady).not.toHaveBeenCalled();
  });

  it('changing callbacks cannot renew the deadline, and unmount cancels requests', async () => {
    getConfigMock.mockReturnValue(new Promise(() => {}));
    act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(9500); });
    act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    expect(getConfigMock.mock.calls.every(([signal]) => signal?.aborted)).toBe(true);
  });

  it('three healthy returning connections have no minimum wait', async () => {
    getConfigMock.mockResolvedValue({ config: { wizard_complete: true } });
    for (let i = 0; i < 3; i++) {
      const onReady = vi.fn();
      const started = performance.now();
      act(() => root.render(<BootScreen key={i} onReady={onReady} onMalformed={vi.fn()} />));
      await flush();
      expect(onReady).toHaveBeenCalledWith('app');
      expect(performance.now() - started).toBeLessThan(3000);
    }
  });
});


describe('daemon-failure detail from the Tauri shell', () => {
  afterEach(() => { delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__; });

  it('shows the detail while connecting and in the failed state, without moving the deadline, and unsubscribes', async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    eventMock.handlers.length = 0;
    eventMock.unlisten.mockClear();
    getConfigMock.mockReturnValue(new Promise(() => {}));
    act(() => root.render(<BootScreen onReady={vi.fn()} onMalformed={vi.fn()} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(eventMock.handlers).toHaveLength(1);
    expect(container.textContent).not.toContain('Background service:');

    act(() => eventMock.handlers[0]({ payload: 'sidecar exited with code 1' }));
    expect(container.textContent).toContain('Connection check');
    expect(container.textContent).toContain('Background service: sidecar exited with code 1');

    await act(async () => { await vi.advanceTimersByTimeAsync(9999); });
    expect(container.querySelector('button')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(container.querySelector('button')?.textContent).toContain('Retry');
    expect(container.textContent).toContain('Try again in a moment.');
    expect(container.textContent).toContain('Background service: sidecar exited with code 1');

    act(() => root.unmount());
    expect(eventMock.unlisten).toHaveBeenCalled();
    root = createRoot(container);
  });
});
