/**
 * @vitest-environment jsdom
 *
 * Source copy and Start gating when system audio IS available. (The focus
 * suite covers the mic-only build.) The picker is a modal: opening it must put focus INSIDE the dialog
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
vi.mock('../../lib/systemAudio', async (orig) => ({
  ...(await orig<typeof import('../../lib/systemAudio')>()),
  systemAudioAvailable: async () => true,
  listSystemAudioApps: async () => [],
}));

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

describe('Record sources with system audio available', () => {
  it('names both steps for call-audio-only, and keeps Start disabled when nothing is selected', async () => {
    act(() => recordButton().click());
    await flush();
    await act(async () => { await Promise.resolve(); });
    const d = dialog()!;
    expect(d.textContent).toContain('To record call audio only, enable system audio and turn off your microphone.');
    expect(d.textContent).not.toContain('Turn it off to record only');
    const box = (label: string) => Array.from(d.querySelectorAll('label'))
      .find(l => l.textContent?.includes(label))!.querySelector('input')!;
    const start = () => Array.from(d.querySelectorAll('button')).find(b => b.textContent?.includes('Start recording'))!;
    const noSource = () => d.querySelector('[role="alert"]')?.textContent ?? '';
    expect(noSource()).not.toMatch(/at least one source/);   // microphone only
    act(() => box('Also record the other participants').click());
    act(() => box('Record my microphone').click());
    expect(noSource()).not.toMatch(/at least one source/);   // call audio only
    act(() => box('Also record the other participants').click());
    expect(box('Record my microphone').checked).toBe(false);
    expect(noSource()).toMatch(/at least one source/);       // nothing selected
    expect(start().disabled).toBe(true);
  });
});
