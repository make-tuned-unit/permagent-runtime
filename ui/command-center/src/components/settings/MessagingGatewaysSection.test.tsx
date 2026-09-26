/** @vitest-environment jsdom
 *
 * Wiring test for Devices → Messaging gateways (ST5, dagK settings pass):
 * `/gateway/*` and `/tunnel/status` had no UI. Covers connect/stop/unpair
 * confirmation and the tunnel status readout.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  getGatewayStatus: vi.fn(),
  startGateway: vi.fn(),
  stopGateway: vi.fn(),
  unpairGatewayUser: vi.fn(),
  getTunnelInfo: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`MessagingGatewaysSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { MessagingGatewaysSection } from './MessagingGatewaysSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  allowed.getTunnelInfo.mockResolvedValue({ state: 'Idle', url: '', hostname: '', secret: '' });
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => { root.render(<MessagingGatewaysSection />); });
}

describe('MessagingGatewaysSection', () => {
  it('offers a connect form when Telegram is not configured', async () => {
    allowed.getGatewayStatus.mockResolvedValue([]);
    await mount();
    expect(container.textContent).toContain('Connect Telegram');
  });

  it('confirms before connecting, then starts the gateway with the pasted token', async () => {
    allowed.getGatewayStatus.mockResolvedValue([]);
    allowed.startGateway.mockResolvedValue(undefined);
    await mount();
    const input = container.querySelector('input') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, '123:abc');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const connectBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Connect') as HTMLButtonElement;
    await act(async () => { connectBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.startGateway).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Connect this Telegram bot?');
    const confirmBtn = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Connect').pop() as HTMLButtonElement;
    await act(async () => { confirmBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.startGateway).toHaveBeenCalledWith('telegram', { bot_token: '123:abc' });
  });

  it('shows a running gateway with its paired users and confirms unpair', async () => {
    allowed.getGatewayStatus.mockResolvedValue([{
      gateway_type: 'telegram', running: true, configured: true,
      paired_users: [{ platform: 'telegram', user_id: 'u1', display_name: 'Alex Rivera', session_id: 's1', paired_at: 0 }],
      info: {},
    }]);
    allowed.unpairGatewayUser.mockResolvedValue(undefined);
    await mount();
    expect(container.textContent).toContain('Telegram');
    expect(container.textContent).toContain('Alex Rivera');
    const unpairBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Unpair') as HTMLButtonElement;
    await act(async () => { unpairBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.unpairGatewayUser).not.toHaveBeenCalled();
    const confirmBtn = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Unpair').pop() as HTMLButtonElement;
    await act(async () => { confirmBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.unpairGatewayUser).toHaveBeenCalledWith('telegram', 'u1');
  });

  it('reads the tunnel status and shows it running with its URL', async () => {
    allowed.getGatewayStatus.mockResolvedValue([]);
    allowed.getTunnelInfo.mockResolvedValue({ state: 'Running', url: 'https://abc.trycloudflare.com', hostname: 'abc', secret: 's' });
    await mount();
    expect(container.textContent).toContain('Running');
    expect(container.textContent).toContain('https://abc.trycloudflare.com');
  });
});
