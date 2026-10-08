/**
 * @vitest-environment jsdom
 *
 * The Record picker is a modal: opening it must put focus INSIDE the dialog
 * (not leave it on the rail behind the scrim), Tab must stay inside, Escape
 * closes it and focus goes back to the Record button. Rendered for real — only
 * the network is mocked.
 *
 * This proves DOM focus semantics in jsdom. It does not prove what macOS
 * accessibility exposes for the WebKit view; that needs the native pass.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(async () => [
    { id: 'p1', name: 'Synthetic One', status: 'active' },
    { id: 'p2', name: 'Synthetic Two', status: 'active' },
  ]),
}));

const { readiness } = vi.hoisted(() => ({ readiness: {
  status: 'ready', message: 'Local model found; inference not yet verified', retry: vi.fn(async () => {}),
} }));
vi.mock('../../lib/dictationReadiness', () => ({
  useDictationReadiness: () => readiness,
  getDictationReadiness: () => readiness,
}));

import { MeetingRecorder } from './MeetingRecorder';
import { requestRecordOpen, useAppNavigationRequests } from '../../lib/appNavigationRequests';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  localStorage.clear();
  readiness.status = 'ready'; readiness.message = 'Local model found'; readiness.retry.mockClear();
  useAppNavigationRequests.setState({ recordOpen: null });
  host = document.createElement('div');
  document.body.appendChild(host);
  act(() => { root = createRoot(host); root.render(<MeetingRecorder open />); });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const recordButton = () => host.querySelector<HTMLButtonElement>('button[aria-label="Record meeting"]')!;
const dialog = () => document.querySelector<HTMLElement>('[role="dialog"][aria-modal="true"]');

async function flush() {
  // Projects load, then the next animation frame moves focus.
  await act(async () => {
    await new Promise(r => setTimeout(r, 0));
    await new Promise(r => requestAnimationFrame(() => r(null)));
  });
}

function tab(shift = false) {
  const ev = new KeyboardEvent('keydown', { key: 'Tab', shiftKey: shift, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(ev);
  return ev;
}

describe('Record picker focus', () => {
  it('moves focus into the dialog on open, keeping its semantic roles', async () => {
    recordButton().focus();
    act(() => recordButton().click());
    await flush();
    const d = dialog();
    expect(d).not.toBeNull();
    expect(d!.getAttribute('aria-label')).toBe('Record a meeting');
    expect(d!.getAttribute('aria-describedby')).toBe('pa-record-sources-summary');
    expect(document.getElementById('pa-record-sources-summary')?.textContent).toMatch(/Records your microphone only/);
    expect(d!.contains(document.activeElement)).toBe(true);
  });

  it('offers "Record my microphone" checked by default, and Start needs a source', async () => {
    act(() => recordButton().click());
    await flush();
    const mic = Array.from(dialog()!.querySelectorAll('label'))
      .find(l => l.textContent?.includes('Record my microphone'))!.querySelector('input')!;
    expect(mic.checked).toBe(true);
    expect(dialog()!.textContent).toContain('Microphone is the only recording source available here');
    expect(dialog()!.textContent).not.toContain('Turn it off to record only');
    act(() => mic.click());
    expect(mic.checked).toBe(false);
    expect(dialog()!.querySelector('[role="alert"]')?.textContent).toMatch(/at least one source/);
    const startBtn = Array.from(dialog()!.querySelectorAll('button')).find(b => b.textContent?.includes('Start recording'))!;
    expect(startBtn.disabled).toBe(true);
  });

  it('wraps Tab inside the dialog in both directions', async () => {
    act(() => recordButton().click());
    await flush();
    const d = dialog()!;
    const tabbable = Array.from(d.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled])'));
    const last = tabbable[tabbable.length - 1];
    last.focus();
    const ev = tab();
    expect(ev.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(tabbable[0]);
    tab(true);
    expect(document.activeElement).toBe(last);
  });

  it('Escape closes the dialog and returns focus to the Record button', async () => {
    recordButton().focus();
    act(() => recordButton().click());
    await flush();
    expect(dialog()!.contains(document.activeElement)).toBe(true);
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    expect(dialog()).toBeNull();
    expect(document.activeElement).toBe(recordButton());
  });
});

it('an agent open request shows the picker without acquiring microphone media', async () => {
  const original = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
  const getUserMedia = vi.fn();
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } });
  try {
    act(() => requestRecordOpen()); await flush();
    expect(dialog()).not.toBeNull(); expect(useAppNavigationRequests.getState().recordOpen).toBeNull();
    expect(getUserMedia).not.toHaveBeenCalled();
  } finally {
    if (original) Object.defineProperty(navigator, 'mediaDevices', original);
    else Reflect.deleteProperty(navigator, 'mediaDevices');
  }
});

it('shows a model setup error and Retry while preventing Start without opening media', async () => {
  readiness.status = 'unavailable'; readiness.message = 'No local model configured';
  act(() => recordButton().click());
  await flush();
  const d = dialog()!;
  expect(d.textContent).toContain('No local model configured');
  const start = Array.from(d.querySelectorAll('button')).find(b => b.textContent?.includes('Start recording'))!;
  expect(start.disabled).toBe(true);
  const retry = Array.from(d.querySelectorAll('button')).find(b => b.textContent?.includes('Retry transcription setup'))!;
  act(() => retry.click());
  expect(readiness.retry).toHaveBeenCalledTimes(1);
});

it('leaves Enter on Cancel to the button instead of starting the selected project recording', async () => {
  act(() => recordButton().click());
  await flush();
  const d = dialog()!;
  const project = Array.from(d.querySelectorAll('button')).find(b => b.textContent?.trim() === 'Synthetic One')!;
  act(() => project.click());
  const cancel = Array.from(d.querySelectorAll('button')).find(b => b.textContent === 'Cancel')!;
  cancel.focus();
  const enter = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
  act(() => cancel.dispatchEvent(enter));
  expect(enter.defaultPrevented).toBe(false);
  // jsdom does not perform native keyboard activation; emulate the unblocked click.
  act(() => cancel.click());
  expect(dialog()).toBeNull();
});
