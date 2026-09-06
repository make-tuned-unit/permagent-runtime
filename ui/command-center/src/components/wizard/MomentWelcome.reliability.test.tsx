/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getSecretSources: vi.fn(),
  upsertConfig: vi.fn(),
  setProvider: vi.fn(),
  checkProvider: vi.fn(),
  onAdvance: vi.fn(),
  suggestedManager: vi.fn(),
}));

vi.mock('../../lib/api', () => ({ api: mocks }));
vi.mock('../../lib/store', () => ({ useCommandCenter: (selector: (state: Record<string, unknown>) => unknown) => selector({ pushBrowserOverlay: vi.fn(), popBrowserOverlay: vi.fn() }) }));
vi.mock('../../styles/useTheme', () => ({
  useTheme: () => ({ colors: { text: '#fff', textMuted: '#aaa', textDim: '#777', cyan: '#0ff', success: '#0f8', danger: '#f55', border: '#333', inputBg: '#111', surface: '#222' } }),
}));
vi.mock('../mobius/Mobius', () => ({ Mobius: () => null }));
vi.mock('./atoms', () => ({
  Glass: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
  Particles: () => null,
  GhostLink: ({ children, onClick }: { children?: React.ReactNode; onClick?: () => void }) => <button onClick={onClick}>{children}</button>,
  PrimaryButton: ({ children, onClick, disabled }: { children?: React.ReactNode; onClick?: () => void; disabled?: boolean }) => <button onClick={onClick} disabled={disabled}>{children}</button>,
  Input: ({ value, onChange, ...props }: { value: string; onChange: (value: string) => void; [key: string]: unknown }) => <input value={value} onChange={e => onChange(e.target.value)} {...props} />,
  Select: ({ value, onChange, options }: { value: string; onChange: (value: string) => void; options: Array<{ value: string }> }) => <select value={value} onChange={e => onChange(e.target.value)}>{options.map(o => <option key={o.value} value={o.value}>{o.value}</option>)}</select>,
}));
vi.mock('../settings/secretSource', () => ({
  REFERENCE_HINTS: { onepassword: { placeholder: '', help: '' }, bitwarden: { placeholder: '', help: '' } },
  buildSpec: vi.fn((_kind: string, reference: string) => reference.trim() ? ({ spec: reference.trim() }) : null),
  suggestedManager: mocks.suggestedManager,
}));

import { MomentWelcome } from './MomentWelcome';

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  mocks.getProviders.mockReset();
  mocks.getSecretSources.mockReset();
  mocks.upsertConfig.mockReset();
  mocks.setProvider.mockReset();
  mocks.getProviders.mockResolvedValue([{
    name: 'anthropic', is_configured: false, is_default: true,
    metadata: { default_model: 'fixture-model', config_keys: [{ name: 'ANTHROPIC_API_KEY', secret: true }] },
  }]);
  mocks.getSecretSources.mockResolvedValue({ backends: [] });
  mocks.upsertConfig.mockResolvedValue(undefined);
  mocks.setProvider.mockResolvedValue(undefined);
  mocks.checkProvider.mockReset();
  mocks.onAdvance.mockReset();
  mocks.suggestedManager.mockReset().mockReturnValue(null);
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function enterKey() {
  act(() => root.render(<MomentWelcome active onAdvance={mocks.onAdvance} />));
  await act(async () => { await Promise.resolve(); });
  const input = container.querySelector('input[type="password"]') as HTMLInputElement;
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setter?.call(input, 'fixture-provider-key');
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  return Array.from(container.querySelectorAll('button')).find(button => button.textContent === 'Continue') as HTMLButtonElement;
}

describe('provider onboarding readiness', () => {
  it('introduces the companion before configuration without advancing setup', async () => {
    act(() => root.render(<MomentWelcome active onAdvance={mocks.onAdvance} />));
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelectorAll('h1')).toHaveLength(1);
    expect(container.querySelector('h1')?.textContent).toBe('A companion. Built around you.');
    expect(container.textContent).toContain('you can change it anytime');
    expect(container.querySelector('input[type="password"]')).not.toBeNull();
    expect(mocks.checkProvider).not.toHaveBeenCalled();
    expect(mocks.onAdvance).not.toHaveBeenCalled();
  });

  it('does not advance when the daemon readiness check rejects', async () => {
    mocks.checkProvider.mockRejectedValue(new Error('provider unavailable'));
    const continueButton = await enterKey();
    await act(async () => continueButton.click());
    expect(mocks.checkProvider).toHaveBeenCalledWith('anthropic');
    expect(mocks.onAdvance).not.toHaveBeenCalled();
    expect(container.textContent).toContain('provider unavailable');
  });

  it('advances only after the readiness check succeeds', async () => {
    mocks.checkProvider.mockResolvedValue(undefined);
    const continueButton = await enterKey();
    await act(async () => continueButton.click());
    expect(mocks.checkProvider).toHaveBeenCalledWith('anthropic');
    expect(mocks.onAdvance).toHaveBeenCalledWith('anthropic', '');
  });

  it('does not commit or advance an earlier provider after selection changes mid-save', async () => {
    let resolveProviders: ((value: unknown[]) => void) | undefined;
    const continueButton = await enterKey();
    mocks.getProviders.mockImplementationOnce(() => new Promise(resolve => { resolveProviders = resolve; }));
    act(() => continueButton.click());

    const select = container.querySelector('select') as HTMLSelectElement;
    act(() => {
      select.value = 'openai';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await act(async () => {
      resolveProviders?.([{
        name: 'anthropic', is_configured: false, is_default: true,
        metadata: { default_model: 'fixture-model', config_keys: [{ name: 'ANTHROPIC_API_KEY', secret: true }] },
      }]);
      await Promise.resolve();
    });

    expect(mocks.upsertConfig).not.toHaveBeenCalled();
    expect(mocks.onAdvance).not.toHaveBeenCalled();
  });

  it('clears a prior key and secret reference when switching providers', async () => {
    mocks.suggestedManager.mockReturnValue({ id: 'onepassword', displayName: '1Password', installed: true, signedIn: true });
    const continueButton = await enterKey();
    const managerLink = Array.from(container.querySelectorAll('button')).find(button => button.textContent?.includes('use a reference'));
    expect(managerLink).toBeDefined();
    act(() => managerLink?.click());
    const reference = container.querySelector('input:not([type="password"])') as HTMLInputElement;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(reference, 'op://fixture/key/value');
      reference.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const select = container.querySelector('select') as HTMLSelectElement;
    act(() => {
      select.value = 'openai';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });

    expect((container.querySelector('input[type="password"]') as HTMLInputElement).value).toBe('');
    act(() => Array.from(container.querySelectorAll('button')).find(button => button.textContent?.includes('use a reference'))?.click());
    expect((container.querySelector('input:not([type="password"])') as HTMLInputElement).value).toBe('');
    expect(continueButton.disabled).toBe(true);
  });

  it('does not surface a rejected old-provider save over the new selection', async () => {
    let rejectProviders: ((reason?: unknown) => void) | undefined;
    const continueButton = await enterKey();
    mocks.getProviders.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectProviders = reject; }));
    act(() => continueButton.click());
    const select = container.querySelector('select') as HTMLSelectElement;
    act(() => {
      select.value = 'openai';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await act(async () => {
      rejectProviders?.(new Error('old provider failed'));
      await Promise.resolve();
    });

    expect(container.textContent).not.toContain('old provider failed');
    expect((container.querySelector('select') as HTMLSelectElement).value).toBe('openai');
  });
});
