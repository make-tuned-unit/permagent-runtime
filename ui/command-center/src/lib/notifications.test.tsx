/**
 * @vitest-environment jsdom
 *
 * Download feedback (#6, fix G1+G2 forensics item 6). Before this thread,
 * `routes/inbox.rs::create_inbox_handler` emitted nothing when a file landed
 * in the Downloads inbox, so `ensureNotificationStream`'s switch here had no
 * `inbox_file_received` case to handle even if the daemon had sent one — a
 * download produced no toast, no tray entry, nothing short of opening
 * Settings → Inbox and looking.
 *
 * Only `eventsWsUrl` (real network/token plumbing) is stubbed; everything
 * else in `./api` stays real. A fake `WebSocket` captures the instance
 * `connect()` creates so the test can hand it a real daemon frame shape and
 * observe the resulting `AppNotification` through the real `useNotifications`
 * hook.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api')>();
  return { ...actual, eventsWsUrl: vi.fn(async () => 'ws://localhost/events') };
});

import { ensureNotificationStream, useNotifications, toast, setNotificationPref } from './notifications';
import type { AppNotification } from './notifications';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  onmessage: ((ev: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
  }
}

let container: HTMLDivElement;
let root: Root;
let captured: AppNotification[] = [];
let realWebSocket: typeof WebSocket | undefined;

function Probe() {
  const { items } = useNotifications();
  captured = items;
  return null;
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  captured = [];
  FakeWebSocket.instances = [];
  realWebSocket = (globalThis as unknown as { WebSocket?: typeof WebSocket }).WebSocket;
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = FakeWebSocket;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  (globalThis as unknown as { WebSocket: unknown }).WebSocket = realWebSocket;
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('inbox_file_received → download notification', () => {
  it('turns a real inbox_file_received frame into a download AppNotification naming the file', async () => {
    await act(async () => {
      root.render(<Probe />);
    });
    ensureNotificationStream();
    await flush();

    const ws = FakeWebSocket.instances[0];
    expect(ws, 'ensureNotificationStream must open a WebSocket').toBeTruthy();

    await act(async () => {
      ws.onmessage?.({
        data: JSON.stringify({
          type: 'inbox_file_received',
          timestamp: new Date().toISOString(),
          payload: {
            filename: 'invoice.pdf',
            size_bytes: 2048,
            source_host: 'example.com',
            status: 'received',
          },
        }),
      });
    });
    await flush();

    const found = captured.find((n) => n.kind === 'download');
    expect(found, `expected a download notification, got: ${JSON.stringify(captured)}`).toBeTruthy();
    expect(found?.body).toContain('invoice.pdf');
  });
});

describe('toast() — "System messages" preference vs. opts.force (R8, coordinator ruling 2026-09-25)', () => {
  afterEach(() => setNotificationPref('system', true));

  it('a normal toast is silenced by the "System messages" preference', async () => {
    setNotificationPref('system', false);
    act(() => { root.render(<Probe />); });
    act(() => { toast('Background job finished'); });
    await flush();
    expect(captured.find(n => n.title === 'Background job finished')).toBeUndefined();
  });

  it('opts.force bypasses the preference — a failure caused by the user\'s own click always toasts (NN/g H9, HAX G16)', async () => {
    setNotificationPref('system', false);
    act(() => { root.render(<Probe />); });
    act(() => { toast('Action failed', 'the daemon said no', undefined, { tone: 'danger', force: true }); });
    await flush();
    const found = captured.find(n => n.title === 'Action failed');
    expect(found).toBeTruthy();
    expect(found?.tone).toBe('danger');
  });
});
