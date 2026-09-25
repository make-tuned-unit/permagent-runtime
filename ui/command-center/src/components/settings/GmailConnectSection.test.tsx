/** @vitest-environment jsdom
 *
 * Wiring test for Services → Gmail (ST5, dagK settings pass):
 * `/integrations/gmail/connect` + `/integrations/gmail` (DELETE) had no UI —
 * only a read-only precondition line elsewhere. Covers connect/disconnect
 * confirmation and that connecting opens Google's consent URL rather than
 * handling OAuth itself.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

const allowed = vi.hoisted(() => ({
  getIntegrations: vi.fn(),
  gmailConnect: vi.fn(),
  gmailDisconnect: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`GmailConnectSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { GmailConnectSection } from './GmailConnectSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let openSpy: any;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  openSpy = vi.spyOn(window, 'open').mockImplementation(() => null);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
  vi.clearAllMocks();
  openSpy.mockRestore();
});

async function mount() {
  await act(async () => { root.render(<GmailConnectSection />); });
}

function setValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('GmailConnectSection', () => {
  it('shows the connect form when not connected', async () => {
    allowed.getIntegrations.mockResolvedValue([{ provider: 'gmail', connected: false, token_present: false }]);
    await mount();
    expect(container.textContent).toContain('Not connected');
    expect(container.textContent).toContain('OAuth client ID');
  });

  it('shows Disconnect, not the connect form, when already connected', async () => {
    allowed.getIntegrations.mockResolvedValue([{ provider: 'gmail', connected: true, token_present: true }]);
    await mount();
    expect(container.textContent).toContain('Connected');
    expect(container.textContent).not.toContain('OAuth client ID');
    expect(Array.from(container.querySelectorAll('button')).some(b => b.textContent === 'Disconnect')).toBe(true);
  });

  it('confirms before connecting, then opens the auth URL without handling OAuth itself', async () => {
    allowed.getIntegrations.mockResolvedValue([{ provider: 'gmail', connected: false, token_present: false }]);
    allowed.gmailConnect.mockResolvedValue({ auth_url: 'https://accounts.google.com/o/oauth2/v2/auth?x=1', callback_port: 8095 });
    await mount();
    const [idInput, secretInput] = Array.from(container.querySelectorAll('input')) as HTMLInputElement[];
    await act(async () => { setValue(idInput, 'client-id'); setValue(secretInput, 'client-secret'); });
    const connectBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Connect Gmail') as HTMLButtonElement;
    await act(async () => { connectBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.gmailConnect).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('Connect Gmail?');
    const confirmBtn = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Continue to Google').pop() as HTMLButtonElement;
    await act(async () => { confirmBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.gmailConnect).toHaveBeenCalledWith('client-id', 'client-secret');
    expect(openSpy).toHaveBeenCalledWith('https://accounts.google.com/o/oauth2/v2/auth?x=1', '_blank', 'noopener,noreferrer');
  });

  it('confirms before disconnecting', async () => {
    allowed.getIntegrations.mockResolvedValue([{ provider: 'gmail', connected: true, token_present: true }]);
    allowed.gmailDisconnect.mockResolvedValue(undefined);
    await mount();
    const disconnectBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Disconnect') as HTMLButtonElement;
    await act(async () => { disconnectBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.gmailDisconnect).not.toHaveBeenCalled();
    const confirmBtn = Array.from(document.querySelectorAll('button')).filter(b => b.textContent === 'Disconnect').pop() as HTMLButtonElement;
    await act(async () => { confirmBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(allowed.gmailDisconnect).toHaveBeenCalledOnce();
  });
});
