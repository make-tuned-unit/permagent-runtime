/** @vitest-environment jsdom
 *
 * Removing a custom provider is Tier 2 — destructive but recoverable with
 * effort (re-add the definition, re-enter the key). Per the destructive-action
 * ruling that tier confirms INLINE, on the row, so what is being removed stays
 * on screen; it does not spend a full-attention modal, and it certainly does
 * not spend an OS dialog that bypasses the theme and the interface voice.
 */

import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const allowed = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getConfig: vi.fn(),
  getSecretSources: vi.fn(),
  getProviderModels: vi.fn(),
  checkProvider: vi.fn(),
  removeCustomProvider: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`ProvidersSection touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { ProvidersSection } from './ProvidersSection';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const CUSTOM_PROVIDER = {
  name: 'my-llm',
  is_configured: true,
  is_default: false,
  provider_type: 'Custom',
  metadata: {
    display_name: 'My LLM',
    description: 'A self-hosted endpoint',
    default_model: 'my-model',
    known_models: [{ name: 'my-model' }],
    config_keys: [{ name: 'MY_LLM_API_KEY', required: true, secret: true }],
  },
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  allowed.getProviders.mockReset().mockResolvedValue([CUSTOM_PROVIDER] as never);
  allowed.getConfig.mockReset().mockResolvedValue({ config: {} } as never);
  allowed.getSecretSources.mockReset().mockResolvedValue({ keys: [] } as never);
  allowed.getProviderModels.mockReset().mockResolvedValue(['my-model'] as never);
  allowed.checkProvider.mockReset().mockResolvedValue({ ok: true } as never);
  allowed.removeCustomProvider.mockReset().mockResolvedValue(undefined as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState({ providers: [], providersError: false });
});

async function render() {
  await act(async () => { root.render(<ProvidersSection />); });
  await act(async () => { await Promise.resolve(); });
}

function button(match: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll('button')).find(
    b => (b.textContent ?? '').trim() === match,
  );
}

async function click(el: HTMLButtonElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

describe('removing a custom provider', () => {
  it('asks on the row first, and removes nothing on the first click', async () => {
    await render();
    const remove = button('Remove');
    expect(remove, 'the custom provider card should offer Remove').toBeDefined();

    await click(remove!);

    expect(allowed.removeCustomProvider).not.toHaveBeenCalled();
    const text = container.textContent ?? '';
    // The consequence, on the row, in the app's own voice.
    expect(text).toContain('Remove My LLM?');
    expect(text).toContain('saved configuration');
    expect(button('Cancel'), 'the two-step must be escapable').toBeDefined();
  });

  it('removes on the second, explicit click', async () => {
    await render();
    await click(button('Remove')!);
    const confirm = button('Remove provider');
    expect(confirm, 'the confirming control must be its own labelled button').toBeDefined();

    await click(confirm!);

    expect(allowed.removeCustomProvider).toHaveBeenCalledWith('my-llm');
  });

  it('backs out without removing anything', async () => {
    await render();
    await click(button('Remove')!);
    await click(button('Cancel')!);

    expect(allowed.removeCustomProvider).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('Remove My LLM?');
    expect(button('Remove'), 'the row returns to rest').toBeDefined();
  });

  it('says so on the row when the removal fails, and never looks like it worked', async () => {
    allowed.removeCustomProvider.mockRejectedValue(new Error('daemon said no') as never);
    await render();
    await click(button('Remove')!);
    await click(button('Remove provider')!);

    const alert = container.querySelector('[role="alert"]');
    expect(alert, 'a failed removal must be visible, not a console line').not.toBeNull();
    expect(alert!.textContent).toContain("Couldn't remove");
    expect(container.textContent).toContain('My LLM');
  });
});
