/** @vitest-environment jsdom */

// The Settings provider list is entirely data-driven: it renders whatever the
// daemon's /providers route returns. So "Z.AI shows up in Settings" is not a UI
// constant to assert — it is a claim about the daemon → store → card path
// holding for the Z.AI metadata shape that `providers/zai.rs` emits. This test
// feeds that exact payload in and walks it all the way to the API-key field the
// operator types into.

import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const allowed = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getConfig: vi.fn(),
  getSecretSources: vi.fn(),
  getProviderModels: vi.fn(),
  checkProvider: vi.fn(),
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

/** The metadata `ZaiProvider::metadata()` serialises onto GET /providers. */
const ZAI_PROVIDER = {
  name: 'zai',
  is_configured: false,
  is_default: false,
  provider_type: 'Builtin',
  metadata: {
    display_name: 'Z.AI',
    description:
      'GLM models from Z.AI (Zhipu AI), including the GLM-5 and GLM-4.x coding and vision families',
    default_model: 'glm-4.7',
    known_models: [
      { name: 'glm-5.3' },
      { name: 'glm-5.2' },
      { name: 'glm-4.7' },
      { name: 'glm-4.6' },
      { name: 'glm-4.5-air' },
    ],
    config_keys: [
      { name: 'ZAI_API_KEY', required: true, secret: true },
      { name: 'ZAI_HOST', required: false, secret: false },
    ],
  },
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  allowed.getProviders.mockReset().mockResolvedValue([ZAI_PROVIDER] as never);
  allowed.getConfig.mockReset().mockResolvedValue({ config: {} } as never);
  allowed.getSecretSources.mockReset().mockResolvedValue({ keys: [] } as never);
  allowed.getProviderModels.mockReset().mockResolvedValue(
    ZAI_PROVIDER.metadata.known_models.map(m => m.name) as never,
  );
  allowed.checkProvider.mockReset().mockResolvedValue({ ok: true } as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState({ providers: [], providersError: false });
});

async function render() {
  await act(async () => {
    root.render(<ProvidersSection />);
  });
  // let the load effects settle
  await act(async () => { await Promise.resolve(); });
}

describe('Z.AI in the Settings provider list', () => {
  it('renders a Z.AI card from the daemon metadata', async () => {
    await render();
    const text = container.textContent ?? '';
    expect(text).toContain('Z.AI');
    expect(text).toContain('GLM models from Z.AI');
    // No key set in this fixture, so the operator is told so.
    expect(text).toContain('Not configured');
  });

  it('maps the Z.AI config keys and models into the store', async () => {
    await render();
    const zai = useCommandCenter.getState().providers.find(p => p.name === 'zai');
    expect(zai).toBeDefined();
    expect(zai!.displayName).toBe('Z.AI');
    expect(zai!.defaultModel).toBe('glm-4.7');
    expect(zai!.knownModels).toContain('glm-5.3');
    // The card picks the API-key field by `secret` — if ZAI_API_KEY ever stopped
    // being marked secret, Settings would offer no key entry at all.
    const secret = zai!.configKeys.find(k => k.secret);
    expect(secret?.name).toBe('ZAI_API_KEY');
    expect(secret?.required).toBe(true);
  });

  it('offers a ZAI_API_KEY entry when the card is configured', async () => {
    await render();
    const configure = Array.from(container.querySelectorAll('button')).find(b =>
      (b.textContent ?? '').includes('Configure'),
    );
    expect(configure, 'the Z.AI card should have a Configure button').toBeDefined();

    await act(async () => {
      configure!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await act(async () => { await Promise.resolve(); });

    const body = document.body.textContent ?? '';
    expect(body).toContain('ZAI_API_KEY');
    const keyInput = document.querySelector('input[type="password"]');
    expect(keyInput, 'the API key must be entered in a masked field').not.toBeNull();
  });
});

const OPENAI_CONNECTED = {
  name: 'openai',
  is_configured: true,
  is_default: true,
  provider_type: 'Builtin',
  metadata: {
    display_name: 'OpenAI',
    description: 'GPT models',
    default_model: 'gpt-4o',
    known_models: [{ name: 'gpt-4o' }],
    config_keys: [{ name: 'OPENAI_API_KEY', required: true, secret: true }],
  },
};

describe('Connected vs Providers tabs', () => {
  it('opens on Connected when a key is already in, and hides the catalogue', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED, ZAI_PROVIDER] as never);
    await render();

    expect(container.textContent).toContain('OpenAI');
    expect(container.textContent).toContain('Connected');
    expect(container.textContent).not.toContain('Z.AI');

    const connected = container.querySelector('[data-testid="providers-tab-connected"]') as HTMLButtonElement;
    const catalogue = container.querySelector('[data-testid="providers-tab-providers"]') as HTMLButtonElement;
    expect(connected.getAttribute('aria-selected')).toBe('true');
    expect(catalogue.textContent).toContain('(1)');
  });

  it('moves a connected provider off the Providers tab', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED, ZAI_PROVIDER] as never);
    await render();

    const catalogue = container.querySelector('[data-testid="providers-tab-providers"]') as HTMLButtonElement;
    await act(async () => {
      catalogue.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(container.textContent).toContain('Z.AI');
    expect(container.textContent).not.toContain('OpenAI');
  });
});


const LM_STUDIO_KEYLESS = {
  name: 'lmstudio',
  is_configured: true,
  is_default: false,
  provider_type: 'Builtin',
  metadata: {
    display_name: 'LM Studio',
    description: 'Local models',
    default_model: 'local',
    known_models: [{ name: 'local' }],
    config_keys: [{ name: 'LMSTUDIO_HOST', required: false, secret: false }],
  },
};

describe('Status badge on keyless providers', () => {
  it('says "No key needed" for a provider with no secret, "Connected" for a keyed one, same slot on both', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED, LM_STUDIO_KEYLESS] as never);
    await render();

    const keyed = container.querySelector('[data-testid="provider-status-openai"]')!;
    expect(keyed.textContent).toContain('Connected');
    await act(async () => {
      container.querySelector('[data-testid="providers-tab-ready"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    const keyless = container.querySelector('[data-testid="provider-status-lmstudio"]')!;
    expect(keyless.textContent).toBe('No key needed');
    // Same placement: the status badge is the first chip in each card's badge group.
    expect(keyless.parentElement!.firstElementChild).toBe(keyless);
    expect(keyed.parentElement!.firstElementChild).toBe(keyed);
  });
});

describe('tab counts agree with badges', () => {
  it('counts a keyless provider under Ready to use, never Connected', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED, LM_STUDIO_KEYLESS, ZAI_PROVIDER] as never);
    await render();
    const tab = (id: string) => container.querySelector(`[data-testid="providers-tab-${id}"]`) as HTMLButtonElement;
    expect(tab('connected').textContent).toBe('Connected (1)');
    expect(tab('ready').textContent).toBe('Ready to use (1)');
    expect(tab('providers').textContent).toBe('Providers (1)');
    await act(async () => { tab('ready').dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(container.querySelector('[data-testid="provider-status-lmstudio"]')!.textContent).toBe('No key needed');
    expect(container.textContent).not.toContain('OpenAI');
  });
});

const optionalSecretLocal = (name: string, display: string) => ({
  name,
  is_configured: true,
  is_default: false,
  provider_type: 'Builtin',
  metadata: {
    display_name: display,
    description: 'Local OpenAI-compatible runtime',
    default_model: 'local',
    known_models: [{ name: 'local' }],
    config_keys: [
      { name: `${name.toUpperCase()}_HOST`, required: false, secret: false },
      { name: 'OPENAI_CUSTOM_HEADERS', required: false, secret: true },
    ],
  },
});

describe('G15: optional secrets do not make a provider Connected', () => {
  it('blank profile: Llama Swap, LM Studio and the Qwen split are Ready to use, none Connected', async () => {
    allowed.getProviders.mockResolvedValue([
      optionalSecretLocal('llamaswap', 'Llama Swap'),
      optionalSecretLocal('lmstudio', 'LM Studio'),
      optionalSecretLocal('qwen38split', 'Qwen3.8-27B (split)'),
    ] as never);
    await render();
    const tab = (id: string) => container.querySelector(`[data-testid="providers-tab-${id}"]`) as HTMLButtonElement;
    expect(tab('connected').textContent).toBe('Connected (0)');
    expect(tab('ready').textContent).toBe('Ready to use (3)');
    await act(async () => { tab('ready').dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    for (const n of ['llamaswap', 'lmstudio', 'qwen38split']) {
      expect(container.querySelector(`[data-testid="provider-status-${n}"]`)!.textContent).toBe('No key needed');
    }
    expect(container.textContent).not.toContain('Connected (3)');
  });

  it('a required secret that resolves is still Connected', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED, optionalSecretLocal('lmstudio', 'LM Studio')] as never);
    await render();
    const tab = (id: string) => container.querySelector(`[data-testid="providers-tab-${id}"]`) as HTMLButtonElement;
    expect(tab('connected').textContent).toBe('Connected (1)');
    expect(container.querySelector('[data-testid="provider-status-openai"]')!.textContent).toContain('Connected');
  });

  it('a required key on an unconfirmed password-manager source shows "Key saved?"', async () => {
    allowed.getProviders.mockResolvedValue([OPENAI_CONNECTED] as never);
    allowed.getSecretSources.mockResolvedValue({
      keys: [{ key: 'OPENAI_API_KEY', kind: 'onepassword', label: '1Password', reference: 'op://x/y/z', resolves: null }],
    } as never);
    await render();
    expect(container.querySelector('[data-testid="provider-status-openai"]')!.textContent).toBe('Key saved?');
  });
});
