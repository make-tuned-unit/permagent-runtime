/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { readConfig, upsertConfig } = vi.hoisted(() => ({
  readConfig: vi.fn(),
  upsertConfig: vi.fn(),
}));

vi.mock('../../lib/api', () => ({ api: { readConfig, upsertConfig } }));
vi.mock('../../stores/identityStore', () => ({
  useIdentityStore: { getState: () => ({ refresh: vi.fn() }) },
}));

import { OnChainIdentitySection } from './OnChainIdentitySection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  readConfig.mockReset().mockResolvedValue(null);
  upsertConfig.mockReset().mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => { root.render(<OnChainIdentitySection />); });
  await act(async () => { await Promise.resolve(); });
}

function input(label: string): HTMLInputElement {
  const row = [...container.querySelectorAll('.pa-set-row')]
    .find(el => el.querySelector('.pa-set-row-label')?.textContent?.includes(label));
  const field = row?.querySelector<HTMLInputElement>('input');
  if (!field) throw new Error(`Missing identity input: ${label}`);
  return field;
}

function change(field: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
  setter.call(field, value);
  field.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('Settings → Persona → On-chain identity', () => {
  it('writes the opt-in slug and complete contract set through their config keys', async () => {
    await render();
    const address = `0x${'a'.repeat(40)}`;
    act(() => {
      change(input('chitin.id slug'), 'ada.agent');
      change(input('Soul token contract'), address);
      change(input('Soul token id'), '12');
      change(input('Passport contract'), address);
      change(input('Passport token id'), '34');
      change(input('Owner wallet'), address);
    });
    const save = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('Save')) as HTMLButtonElement;
    expect(save.disabled).toBe(false);
    await act(async () => { save.click(); });

    expect(upsertConfig).toHaveBeenNthCalledWith(1, 'identity_chitin_slug', 'ada.agent');
    expect(upsertConfig).toHaveBeenNthCalledWith(2, 'identity_chain', {
      sbt_contract: address, passport_contract: address, owner: address,
      sbt_token_id: '12', passport_token_id: '34',
    });
    expect([...container.querySelectorAll('button')].find(b => b.textContent?.includes('Save'))?.disabled).toBe(true);
  });

  it('rejects malformed contract data before issuing any config write', async () => {
    await render();
    act(() => change(input('chitin.id slug'), 'not a slug'));
    const save = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('Save')) as HTMLButtonElement;
    await act(async () => { save.click(); });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('slug may only use');
    expect(upsertConfig).not.toHaveBeenCalled();
  });
});
