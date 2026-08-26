/**
 * @vitest-environment jsdom
 *
 * Settings → Features pins: the pane reads the four daemon config keys, a flip
 * upserts the right key, a failed save reverts the toggle, and the Concierge
 * row states its Gmail-token precondition (with the CLI command) when no token
 * is stored.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../../lib/api', () => ({
  api: {
    readConfig: vi.fn(),
    upsertConfig: vi.fn(),
    getIntegrations: vi.fn(),
    getCouncilMembers: vi.fn(),
    putCouncilMembers: vi.fn(),
  },
}));

import { FeaturesPanel } from './FeaturesPanel';
import { api } from '../../../lib/api';
import { GMAIL_CONNECT_COMMAND } from './features';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const readConfig = vi.mocked(api.readConfig);
const upsertConfig = vi.mocked(api.upsertConfig);
const getIntegrations = vi.mocked(api.getIntegrations);
const getCouncilMembers = vi.mocked(api.getCouncilMembers);
const putCouncilMembers = vi.mocked(api.putCouncilMembers);

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

async function mount(goto = vi.fn()) {
  await act(async () => { root.render(<FeaturesPanel goto={goto} />); });
  await flush();
}

function toggles(): HTMLButtonElement[] {
  // Toggle renders a 36px-wide button; the goto link is the only other button.
  return Array.from(container.querySelectorAll('button')).filter(
    b => (b as HTMLButtonElement).style.width === '36px',
  ) as HTMLButtonElement[];
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  readConfig.mockReset();
  upsertConfig.mockReset();
  getIntegrations.mockReset();
  getCouncilMembers.mockReset();
  putCouncilMembers.mockReset();
  readConfig.mockImplementation(async (key: string) => key === 'initiative_enabled');
  upsertConfig.mockResolvedValue({});
  getCouncilMembers.mockResolvedValue({ enabled: false, exclude: [], seats: [] });
  putCouncilMembers.mockResolvedValue({ enabled: false, exclude: [], seats: [] });
  getIntegrations.mockResolvedValue([
    { provider: 'gmail', connected: false, token_present: false },
    { provider: 'slack', connected: false, token_present: false },
  ]);
});

afterEach(async () => {
  await act(async () => { root.unmount(); });
  container.remove();
});

describe('FeaturesPanel', () => {
  // REGRESSION on the Guard's row: `strix_enabled` used to be switchable only
  // from the Models pane, so this pane read four keys and the Guard was not
  // among them. The assertion below fails on that roster.
  it('reads every config key, the Guard included, and renders one toggle per row', async () => {
    await mount();
    const keys = readConfig.mock.calls.map(c => c[0]).sort();
    expect(keys).toEqual([
      'concierge_enabled',
      'council_enabled',
      'initiative_enabled',
      'playbook_enabled',
      'steward_scan_enabled',
      'strix_enabled',
    ]);
    expect(toggles()).toHaveLength(6);
    expect(getCouncilMembers).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Initiative');
    expect(container.textContent).toContain('Decision Playbook');
    expect(container.textContent).toContain('Concierge');
    expect(container.textContent).toContain('Steward git-health');
    expect(container.textContent).toContain('The Guard');
    expect(container.textContent).toContain('The Council');
    // The read-back value is honoured: initiative on, the rest off.
    expect(toggles()[0].style.transform).toBe('');
    const knob = (t: HTMLButtonElement) => (t.firstElementChild as HTMLElement).style.transform;
    expect(knob(toggles()[0])).toBe('translateX(14px)');
    expect(knob(toggles()[1])).toBe('translateX(0)');
  });

  it('flipping a row upserts exactly that key', async () => {
    await mount();
    // Row order: initiative, playbook, concierge, steward.
    await act(async () => { toggles()[1].click(); });
    expect(upsertConfig).toHaveBeenCalledTimes(1);
    expect(upsertConfig).toHaveBeenCalledWith('playbook_enabled', true);
    await act(async () => { toggles()[3].click(); });
    expect(upsertConfig).toHaveBeenLastCalledWith('steward_scan_enabled', true);
  });

  it('reverts the toggle and shows the error when the save fails', async () => {
    upsertConfig.mockRejectedValue(new Error('daemon said no'));
    await mount();
    const knob = () => (toggles()[2].firstElementChild as HTMLElement).style.transform;
    expect(knob()).toBe('translateX(0)');
    await act(async () => { toggles()[2].click(); });
    await flush();
    expect(upsertConfig).toHaveBeenCalledWith('concierge_enabled', true);
    expect(knob()).toBe('translateX(0)');
    expect(container.textContent).toContain("Couldn't save: daemon said no");
  });

  it('states the Concierge Gmail precondition with the CLI command when no token is stored', async () => {
    await mount();
    const line = container.querySelector('[data-testid="concierge-precondition"]');
    expect(line?.textContent).toContain('Needs a Gmail token');
    expect(line?.textContent).toContain(GMAIL_CONNECT_COMMAND);
    // The toggle is NOT disabled — the loop is inert without a token, and the
    // copy says why; no dead control.
    expect(toggles()[2].disabled).toBe(false);
  });

  it('says the token is present when /integrations reports one', async () => {
    getIntegrations.mockResolvedValue([{ provider: 'gmail', connected: true, token_present: true }]);
    await mount();
    const line = container.querySelector('[data-testid="concierge-precondition"]');
    expect(line?.textContent).toContain('Gmail token present');
    expect(line?.textContent).not.toContain(GMAIL_CONNECT_COMMAND);
  });

  // REGRESSION, the write half of the Guard's row. Before `strix_enabled` joined
  // FEATURE_ROWS this pane had four toggles and none of them wrote that key —
  // toggles()[4] did not exist.
  it('flipping the Guard row writes strix_enabled and nothing else', async () => {
    await mount();
    expect(toggles()).toHaveLength(6);
    await act(async () => { toggles()[4].click(); });
    await flush();
    expect(upsertConfig).toHaveBeenCalledTimes(1);
    expect(upsertConfig).toHaveBeenCalledWith('strix_enabled', true);
  });

  // REGRESSION on the hand-written UNLOADED map. It listed the four original
  // keys by hand, so a newly added row read as `undefined` — which is not
  // `null`, so the panel skipped "Loading…" and drew a toggle claiming OFF
  // before the daemon had answered. With that map, the Guard row below renders
  // a fifth toggle instead of the loading line.
  it('shows Loading for an unread flag rather than a toggle claiming off', async () => {
    readConfig.mockImplementation((key: string) =>
      key === 'strix_enabled'
        ? new Promise(() => {})            // never settles: the daemon has not answered
        : Promise.resolve(key === 'initiative_enabled'),
    );
    await mount();
    expect(toggles()).toHaveLength(5);
    expect(container.textContent).toContain('Loading…');
  });

  it('links to Settings → Agents for the live roster', async () => {
    const goto = vi.fn();
    await mount(goto);
    const link = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Settings → Agents');
    expect(link).toBeTruthy();
    await act(async () => { link!.click(); });
    expect(goto).toHaveBeenCalledWith('agents');
  });

  it('lists connected chat seats when the Council is on, and unchecking excludes one', async () => {
    readConfig.mockImplementation(async (key: string) => key === 'council_enabled');
    getCouncilMembers.mockResolvedValue({
      enabled: true,
      exclude: [],
      seats: [
        { provider: 'anthropic', display_name: 'Anthropic', model: 'claude-haiku', configured: true, excluded: false, cli_or_acp: false },
        { provider: 'ollama', display_name: 'Ollama', model: 'qwen', configured: true, excluded: false, cli_or_acp: false },
        { provider: 'claude_code', display_name: 'Claude Code', model: 'opus', configured: true, excluded: false, cli_or_acp: true },
      ],
    });
    putCouncilMembers.mockResolvedValue({
      enabled: true,
      exclude: ['ollama'],
      seats: [
        { provider: 'anthropic', display_name: 'Anthropic', model: 'claude-haiku', configured: true, excluded: false, cli_or_acp: false },
        { provider: 'ollama', display_name: 'Ollama', model: 'qwen', configured: true, excluded: true, cli_or_acp: false },
        { provider: 'claude_code', display_name: 'Claude Code', model: 'opus', configured: true, excluded: false, cli_or_acp: true },
      ],
    });
    await mount();
    await flush();
    expect(getCouncilMembers).toHaveBeenCalled();
    expect(container.textContent).toContain('Council seats');
    expect(container.textContent).toContain('Anthropic');
    expect(container.querySelector('[data-testid="council-seat-anthropic"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="council-seat-ollama"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="council-seat-claude_code"]')).toBeNull();
    const boxes = Array.from(container.querySelectorAll('input[type="checkbox"]')) as HTMLInputElement[];
    expect(boxes).toHaveLength(2);
    await act(async () => { boxes[1].click(); });
    await flush();
    expect(putCouncilMembers).toHaveBeenCalledWith(['ollama']);
  });
});
