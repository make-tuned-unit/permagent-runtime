/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
vi.mock('../../lib/api', () => ({ api: { getSovereignty: vi.fn(), setSovereignty: vi.fn(), getEgressLog: vi.fn(async () => []), getAuthLog: vi.fn(async () => []) }, apiFetch: vi.fn(async () => []) }));
vi.mock('./AuthLogSection', () => ({ AuthLogSection: () => null }));
import { api } from '../../lib/api';
import { SovereigntyBlock } from './SettingsView';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
it('never describes an unknown or unconfirmed boundary as allowed or blocked', async () => {
  vi.mocked(api.getSovereignty).mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce({ enabled: false, capturePrompts: false, localProviderAvailable: true });
  vi.mocked(api.setSovereignty).mockRejectedValueOnce(new Error('disconnected'));
  const host = document.createElement('div'); document.body.appendChild(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<SovereigntyBlock />));
    expect(host.textContent).toContain('Current data boundary is unknown');
    expect(host.textContent).not.toContain('Cloud inference is allowed');
    expect(host.querySelector<HTMLButtonElement>('[role="switch"]')!.disabled).toBe(true);
    await act(async () => Array.from(host.querySelectorAll('button')).find(b => b.textContent === 'Retry status')!.click());
    expect(host.textContent).toContain('Cloud inference is allowed');
    await act(async () => Array.from(host.querySelectorAll('button')).find(b => b.textContent === 'Pull the cable')!.click());
    expect(host.textContent).toContain('Current data boundary is unknown');
    expect(host.textContent).not.toContain('All cloud inference is blocked');
    expect(api.setSovereignty).toHaveBeenCalledWith({ enabled: true });
  } finally { await act(async () => root.unmount()); host.remove(); }
});
