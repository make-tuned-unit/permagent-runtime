/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getConfig: vi.fn(),
    upsertConfig: vi.fn(),
    getBudget: vi.fn(),
    setBudget: vi.fn(),
  },
  apiFetch: vi.fn(),
}));

import { api } from '../../lib/api';
import { AutonomyPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getConfig = vi.mocked(api.getConfig);
const getBudget = vi.mocked(api.getBudget);
const setBudget = vi.mocked(api.setBudget);
let container: HTMLDivElement;
let root: Root;

const budget = {
  session: { soft: 2, gate: 4, hard: 7 },
  task: { soft: 5, gate: 10, hard: 25 },
};

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getConfig.mockReset().mockResolvedValue({ config: {}, effective_goose_mode: 'auto' } as never);
  getBudget.mockReset().mockResolvedValue(budget);
  setBudget.mockReset().mockResolvedValue(budget);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => { root.render(<AutonomyPanel />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe('AutonomyPanel guardrail wiring', () => {
  it('loads current spend caps and persists slider changes through the budget endpoint', async () => {
    await mount();
    expect(getBudget).toHaveBeenCalledTimes(1);
    const sliders = Array.from(container.querySelectorAll('input[type="range"]')) as HTMLInputElement[];
    expect(sliders.map(input => input.value)).toEqual(['7', '25']);

    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      setter.call(sliders[0], '9');
      sliders[0].dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(setBudget).toHaveBeenCalledWith({ session: { hard: 9 } });
  });

  it('renders per-action confirmation controls disabled and non-interactive', async () => {
    await mount();
    const section = Array.from(container.querySelectorAll('section, div')).find(
      node => node.firstElementChild?.textContent === 'Always confirm before…',
    );
    expect(section).toBeTruthy();
    const toggles = Array.from(section!.querySelectorAll('button')) as HTMLButtonElement[];
    expect(toggles).toHaveLength(5);
    expect(toggles.every(toggle => toggle.disabled)).toBe(true);
    toggles.forEach(toggle => toggle.click());
    expect(api.upsertConfig).not.toHaveBeenCalled();
  });
});
