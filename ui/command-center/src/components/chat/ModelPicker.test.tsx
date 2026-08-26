/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import type { ProviderInfo } from '../../lib/store';
import { filterConfiguredModels, ModelPicker } from './ModelPicker';

const state = vi.hoisted(() => ({
  providers: [] as ProviderInfo[],
  currentModel: 'claude-opus-4-6',
  loadProviders: vi.fn(),
  setDefaultProvider: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function provider(partial: Partial<ProviderInfo> & Pick<ProviderInfo, 'name' | 'knownModels'>): ProviderInfo {
  return {
    displayName: partial.displayName ?? partial.name,
    description: '',
    defaultModel: partial.knownModels[0] ?? '',
    configKeys: [],
    isConfigured: true,
    isDefault: false,
    providerType: 'Builtin',
    ...partial,
  };
}

describe('filterConfiguredModels', () => {
  const providers = [
    provider({ name: 'anthropic', displayName: 'Anthropic', knownModels: ['claude-opus-4-6', 'claude-haiku-4-5'] }),
    provider({ name: 'openai', displayName: 'OpenAI', knownModels: ['gpt-5.4'] }),
  ];

  it('empty query shows all models', () => {
    expect(filterConfiguredModels(providers, '').map(m => m.model)).toEqual([
      'claude-opus-4-6',
      'claude-haiku-4-5',
      'gpt-5.4',
    ]);
  });

  it('filter haiku hides opus', () => {
    const hit = filterConfiguredModels(providers, 'haiku');
    expect(hit.map(m => m.model)).toEqual(['claude-haiku-4-5']);
    expect(hit.some(m => m.model.includes('opus'))).toBe(false);
  });
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  state.providers = [
    provider({
      name: 'anthropic',
      displayName: 'Anthropic',
      knownModels: ['claude-opus-4-6', 'claude-haiku-4-5'],
      isDefault: true,
    }),
  ];
  state.loadProviders.mockReset();
  state.setDefaultProvider.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ModelPicker', () => {
  it('typeahead haiku hides opus', async () => {
    await act(async () => { root.render(<ModelPicker />); });
    const toggle = container.querySelector('button');
    expect(toggle).toBeTruthy();
    await act(async () => { toggle!.click(); });

    const input = container.querySelector('input') as HTMLInputElement;
    expect(input).toBeTruthy();
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(input, 'haiku');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const items = Array.from(container.querySelectorAll('button'))
      .slice(1)
      .map(b => b.textContent ?? '');
    expect(items.some(t => t.includes('claude-haiku-4-5'))).toBe(true);
    expect(items.some(t => t.includes('claude-opus-4-6'))).toBe(false);
  });
});
