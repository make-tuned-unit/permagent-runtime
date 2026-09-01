/** @vitest-environment jsdom
 *
 * The relocated per-agent settings (J8 / C7). These pin the half a rename
 * cannot: that the blocks which used to live under Models still write the
 * same config keys, and that they only appear on the agent they belong to.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const mocked = vi.hoisted(() => ({
  readConfig: vi.fn(async (_k: string) => null as unknown),
  upsertConfig: vi.fn(async () => ({})),
  getLibrarianSchedule: vi.fn(async () => null as unknown),
  getOllamaStatus: vi.fn(async () => ({ reachable: false, installed: [], running: [] })),
  runLibrarianNow: vi.fn(async () => ({})),
}));

vi.mock('../../../lib/api', () => ({ api: mocked, apiFetch: vi.fn() }));

import { AgentSettingsBlock, AGENTS_WITH_SETTINGS, nextRunText } from './agentSettings';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  for (const fn of Object.values(mocked)) fn.mockClear();
  mocked.readConfig.mockImplementation(async () => null);
  mocked.getLibrarianSchedule.mockImplementation(async () => null);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(agentId: string) {
  await act(async () => { root.render(<AgentSettingsBlock agentId={agentId} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

function testid(id: string) {
  return container.querySelector(`[data-testid="${id}"]`);
}

it('an agent with no extra settings gets no block at all', async () => {
  await mount('git_steward');
  expect(container.innerHTML).toBe('');
  expect(AGENTS_WITH_SETTINGS.has('git_steward')).toBe(false);
});

describe("the Guard's sweep settings", () => {
  it('reads and writes the cadence key that used to be written from Models', async () => {
    mocked.readConfig.mockImplementation(async (k: string) => (k === 'strix_sweep_hours' ? 72 : null));
    await mount('strix');

    const select = testid('guard-sweep-hours') as HTMLSelectElement;
    expect(select.value).toBe('72');

    select.value = '168';
    await act(async () => { select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(mocked.upsertConfig).toHaveBeenCalledWith('strix_sweep_hours', 168);
  });

  it('carries no second on/off switch — the agent page already has one', async () => {
    await mount('strix');
    // `AgentEnableRow` above this block is the switch. Two on one page would
    // rebuild the "which one is the real switch" problem the move fixed.
    expect(mocked.readConfig.mock.calls.map(c => c[0])).not.toContain('strix_enabled');
  });

  it('says where it scans when no remote host is configured', async () => {
    await mount('strix');
    expect(container.textContent).toContain('this Mac');
  });
});

describe("the Watcher's teaching keys", () => {
  it('saves topics as a list, trimmed, on Enter', async () => {
    await mount('watcher');
    const input = testid('watcher-topics') as HTMLInputElement;
    // React tracks its own value on the node; a bare assignment is invisible
    // to it, so drive the native setter the way React's own tests do.
    const setValue = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(input, 'local-first software, prediction markets ');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(mocked.upsertConfig).toHaveBeenCalledWith(
      'watcher_topics',
      ['local-first software', 'prediction markets'],
    );
  });
});

describe("the Librarian's schedule", () => {
  it('waits for the schedule rather than drawing an empty one', async () => {
    await mount('librarian');
    expect(container.textContent).toContain('Reading the schedule…');
  });

  it('renders the schedule once the daemon answers', async () => {
    mocked.getLibrarianSchedule.mockImplementation(async () => ({
      enabled: true, start_time: '01:50', duration_minutes: 260,
      model: 'qwen3:8b', run_if_launched_in_window: true, pruning_enabled: false,
    }));
    await mount('librarian');
    expect(container.textContent).toContain('Next run');
    expect(container.textContent).toContain('Run Librarian now');
  });
});

it('nextRunText says Disabled rather than inventing a next run', () => {
  expect(nextRunText({
    enabled: false, start_time: '01:50', duration_minutes: 260,
    model: 'qwen3:8b', run_if_launched_in_window: true,
  })).toBe('Disabled');
});
