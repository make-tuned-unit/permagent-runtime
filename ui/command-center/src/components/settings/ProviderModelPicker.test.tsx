/** @vitest-environment jsdom */
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { ProviderModelPicker } from './ProviderModelPicker';
import type { ProviderInfo } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const { getProviderModels } = vi.hoisted(() => ({ getProviderModels: vi.fn() }));
vi.mock('../../lib/api', () => ({ api: { getProviderModels } }));
const state = vi.hoisted(() => ({ providers: [] as ProviderInfo[], loadProviders: vi.fn() }));
vi.mock('../../lib/store', () => ({ useCommandCenter: (selector: (s: typeof state) => unknown) => selector(state) }));

const provider = (name: string, models: string[]): ProviderInfo => ({ name, displayName: name[0].toUpperCase() + name.slice(1), description: '', defaultModel: models[0], knownModels: models, configKeys: [], isConfigured: true, isDefault: false, providerType: 'Builtin' });
let root: Root; let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement('div'); document.body.appendChild(host); root = createRoot(host);
  state.providers = [provider('anthropic', ['known-model']), provider('openai', ['gpt-5'])];
  getProviderModels.mockReset().mockResolvedValue(['fetched-model']);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

describe('ProviderModelPicker', () => {
  it('shows providers first, expands one, and selects a fetched model', async () => {
    const onChange = vi.fn();
    await act(async () => { root.render(<ProviderModelPicker value={{ provider: null, model: null }} onChange={onChange} />); });
    await act(async () => { (host.querySelector('button[aria-haspopup]') as HTMLButtonElement).click(); });
    expect(host.textContent).toContain('Anthropic'); expect(host.textContent).not.toContain('known-model');
    await act(async () => { (Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('Anthropic')) as HTMLButtonElement).click(); });
    expect(getProviderModels).toHaveBeenCalledWith('anthropic'); expect(host.textContent).toContain('fetched-model');
    await act(async () => { (Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('fetched-model')) as HTMLButtonElement).click(); });
    expect(onChange).toHaveBeenCalledWith({ provider: 'anthropic', model: 'fetched-model' });
  });

  it('offers session action and labels known models as fallback when fetch fails', async () => {
    const onUseSession = vi.fn(); getProviderModels.mockRejectedValueOnce(new Error('offline'));
    await act(async () => { root.render(<ProviderModelPicker value={{ provider: null, model: null }} onChange={vi.fn()} onUseSession={onUseSession} />); });
    await act(async () => { (host.querySelector('button[aria-haspopup]') as HTMLButtonElement).click(); });
    await act(async () => { (Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('Anthropic')) as HTMLButtonElement).click(); });
    expect(host.textContent).toContain('known-model');
    expect(host.textContent).toContain('Live models unavailable');
    await act(async () => { (Array.from(host.querySelectorAll('button')).find(b => b.textContent?.includes('Use session model')) as HTMLButtonElement).click(); });
    expect(onUseSession).toHaveBeenCalled();
  });
});
