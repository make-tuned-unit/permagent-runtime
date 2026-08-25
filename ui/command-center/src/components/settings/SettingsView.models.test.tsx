/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

// Every api method ModelsPanel is allowed to touch. Anything else — notably
// the retired getWorkers / GET /api/agent/workers — throws on access, so a
// regression that re-adds the duplicate roster fails here.
const allowed = vi.hoisted(() => ({
  getConfig: vi.fn(),
  readConfig: vi.fn(),
  upsertConfig: vi.fn(),
  getOllamaStatus: vi.fn(),
  getLibrarianSchedule: vi.fn(),
  setLibrarianSchedule: vi.fn(),
  runLibrarianNow: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: new Proxy(allowed, {
    get(target, key) {
      if (typeof key !== 'string') return undefined;
      if (key in target) return target[key as keyof typeof target];
      throw new Error(`ModelsPanel touched api.${key}, which is not part of its surface`);
    },
  }),
  apiFetch: vi.fn(async (endpoint: string) => {
    throw new Error(`unexpected fetch ${endpoint}`);
  }),
}));

import { ModelsPanel } from './SettingsView';
import { FEATURE_ROWS } from './features/features';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  allowed.getConfig.mockReset().mockResolvedValue({ config: {}, effective_goose_mode: 'auto' } as never);
  allowed.readConfig.mockReset().mockResolvedValue(null as never);
  allowed.upsertConfig.mockReset().mockResolvedValue({} as never);
  allowed.getOllamaStatus.mockReset().mockResolvedValue({ reachable: false } as never);
  allowed.getLibrarianSchedule.mockReset().mockResolvedValue(null as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(goto: (key: string) => void) {
  await act(async () => { root.render(<ModelsPanel goto={goto} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

/** atoms.Toggle is the only 36px-wide button on the pane. */
function toggles(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll('button')).filter(
    b => (b as HTMLButtonElement).style.width === '36px',
  ) as HTMLButtonElement[];
}

/**
 * The Guard's switch is written by THREE surfaces — this pane, Settings →
 * Features, and the Guard's own page under Settings → Agents — and it is only
 * one switch because all three upsert the same key. This pane was the one writer
 * with no test at all, so a rename here would have been caught by nothing: the
 * other two surfaces would keep working while this toggle silently wrote a key
 * the daemon does not read. The key is imported from FEATURE_ROWS rather than
 * typed as a literal, so the tripwire is against the two DISAGREEING, not
 * against either one changing.
 */
describe('ModelsPanel Guard block', () => {
  it('writes the same config key the Features pane writes', async () => {
    const key = FEATURE_ROWS.find(r => r.key === 'strix_enabled')?.key;
    expect(key).toBe('strix_enabled');
    allowed.readConfig.mockImplementation(async (k: string) =>
      (k === key ? false : null) as never,
    );

    await mount(vi.fn());
    const guard = toggles();
    expect(guard.length).toBeGreaterThan(0);

    await act(async () => { guard[0].click(); });
    expect(allowed.upsertConfig).toHaveBeenCalledWith(key, true);
    // No second key: a per-pane flag would be a second source of truth.
    const keysWritten = allowed.upsertConfig.mock.calls.map(c => c[0]);
    expect(keysWritten).toEqual([key]);
  });

  it('points at the other two surfaces that write it', async () => {
    const goto = vi.fn();
    await mount(goto);

    expect(container.textContent).toContain('strix_enabled');

    const features = container.querySelector('[data-testid="guard-open-features"]') as HTMLButtonElement;
    const agents = container.querySelector('[data-testid="guard-open-agents"]') as HTMLButtonElement;
    expect(features).toBeTruthy();
    expect(agents).toBeTruthy();
    await act(async () => { features.click(); });
    expect(goto).toHaveBeenCalledWith('features');
    await act(async () => { agents.click(); });
    expect(goto).toHaveBeenCalledWith('agents');
  });
});

describe('ModelsPanel roster pointer', () => {
  it('points at Settings → Agents instead of fetching its own worker roster', async () => {
    const goto = vi.fn();
    await mount(goto);

    const text = container.textContent ?? '';
    expect(text).not.toContain('Loading roster');
    expect(text).not.toContain('No workers configured');

    const open = container.querySelector('[data-testid="models-open-agents"]') as HTMLButtonElement;
    expect(open).toBeTruthy();
    await act(async () => { open.click(); });
    expect(goto).toHaveBeenCalledWith('agents');
  });
});

/**
 * The voice model route (crates/goose/src/config/voice_model.rs): which
 * model answers a SPOKEN turn, separate from the main GOOSE_MODEL that
 * answers chat. `voice_provider` and `voice_model` set together override a
 * measured default (custom_deepseek / deepseek-chat); either key set to
 * session/off/none turns the feature off. This block checks the panel's
 * display mirrors that precedence and that it writes only on user action —
 * never as a side effect of mounting (the Guard test above already tripwires
 * that for its own key; the "typing alone doesn't save" case below tripwires
 * it for these two).
 */
describe('ModelsPanel voice model block', () => {
  function providerInput(): HTMLInputElement {
    return container.querySelector('input[placeholder="custom_deepseek"]') as HTMLInputElement;
  }
  function modelInput(): HTMLInputElement {
    return container.querySelector('input[placeholder="deepseek-chat"]') as HTMLInputElement;
  }
  async function typeInto(input: HTMLInputElement, value: string) {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }
  function saveButton(): HTMLButtonElement {
    return Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Save') as HTMLButtonElement;
  }
  function sessionButton(): HTMLButtonElement {
    return Array.from(container.querySelectorAll('button')).find(
      b => b.textContent === 'Use the session model',
    ) as HTMLButtonElement;
  }

  it('shows the measured default when neither key is set', async () => {
    await mount(vi.fn());
    expect(container.textContent).toContain('custom_deepseek / deepseek-chat (default)');
  });

  it('shows the configured route when both keys are set', async () => {
    allowed.readConfig.mockImplementation(async (k: string) =>
      (k === 'voice_provider' ? 'anthropic' : k === 'voice_model' ? 'claude-haiku-4-5-20251001' : null) as never,
    );
    await mount(vi.fn());
    expect(container.textContent).toContain('anthropic / claude-haiku-4-5-20251001');
  });

  it('shows "session model" when voice_model is set to session', async () => {
    allowed.readConfig.mockImplementation(async (k: string) => (k === 'voice_model' ? 'session' : null) as never);
    await mount(vi.fn());
    expect(container.textContent).toContain('session model');
  });

  it('does not write either key just from mounting or typing', async () => {
    await mount(vi.fn());
    await typeInto(providerInput(), 'minimax');
    expect(allowed.upsertConfig).not.toHaveBeenCalled();
  });

  it('Save writes both voice_provider and voice_model', async () => {
    await mount(vi.fn());
    await typeInto(providerInput(), 'minimax');
    await typeInto(modelInput(), 'MiniMax-M2.7-highspeed');
    await act(async () => { saveButton().click(); });

    expect(allowed.upsertConfig).toHaveBeenCalledWith('voice_provider', 'minimax');
    expect(allowed.upsertConfig).toHaveBeenCalledWith('voice_model', 'MiniMax-M2.7-highspeed');
  });

  it('"Use the session model" writes only voice_model, to session', async () => {
    await mount(vi.fn());
    await act(async () => { sessionButton().click(); });

    expect(allowed.upsertConfig).toHaveBeenCalledWith('voice_model', 'session');
    const keysWritten = allowed.upsertConfig.mock.calls.map(c => c[0]);
    expect(keysWritten).toEqual(['voice_model']);
  });
});
