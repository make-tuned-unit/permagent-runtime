/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';

// Every api method ModelsPanel is allowed to touch. Anything else — notably
// the retired getWorkers / GET /api/agent/workers — throws on access, so a
// regression that re-adds the duplicate roster fails here.
const allowed = vi.hoisted(() => ({
  getConfig: vi.fn(),
  readConfig: vi.fn(),
  upsertConfig: vi.fn(),
  setModelRoute: vi.fn(),
  getOllamaStatus: vi.fn(),
  getLibrarianSchedule: vi.fn(),
  setLibrarianSchedule: vi.fn(),
  runLibrarianNow: vi.fn(),
  getPacks: vi.fn(),
  applyPacks: vi.fn(),
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

// The provider-first chooser has its own interaction suite. These settings
// tests exercise the pair persistence contract without depending on the
// picker's search/expansion DOM.
vi.mock('./ProviderModelPicker', () => ({
  ProviderModelPicker: ({ onChange, onUseSession, 'aria-label': label }: {
    onChange: (value: { provider: string; model: string }) => void;
    onUseSession?: () => void;
    'aria-label'?: string;
  }) => {
    const voice = label?.startsWith('Voice');
    return <div>
      <button
        type="button"
        data-testid={`choose-${voice ? 'voice' : label?.toLowerCase().split(' ')[0]}`}
        onClick={() => onChange(voice
          ? { provider: 'minimax', model: 'MiniMax-M2.7-highspeed' }
          : { provider: 'anthropic', model: 'claude-sonnet-5' })}
      >Choose configured model</button>
      {onUseSession && <button type="button" onClick={onUseSession}>Use session model</button>}
    </div>;
  },
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
  allowed.setModelRoute.mockReset().mockResolvedValue({} as never);
  allowed.getOllamaStatus.mockReset().mockResolvedValue({ reachable: false } as never);
  allowed.getLibrarianSchedule.mockReset().mockResolvedValue(null as never);
  allowed.getPacks.mockReset().mockResolvedValue({
    prompt: false,
    configured: [],
    recommendation: { recommendations: [], considered: [] },
  } as never);
  allowed.applyPacks.mockReset().mockResolvedValue({ applied: [] } as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount(goto: (key: string) => void) {
  await act(async () => { root.render(<ModelsPanel goto={goto} />); });
  // The role table loads six keys via Promise.all — one more microtask hop
  // than the single .then() chains elsewhere on this pane.
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

/** atoms.Toggle is the only 36px-wide button on the pane. */
function toggles(): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll('button')).filter(
    b => (b as HTMLButtonElement).style.width === '36px',
  ) as HTMLButtonElement[];
}

// ── Chat / Voice / Harness role table helpers ────────────────────────────
// All three rows use the shared provider-first picker and are scoped by their
// data-testid; there are no independently editable half-pairs anymore.

/** The scoping wrapper for one role row. */
function roleRow(role: 'chat' | 'voice' | 'harness'): HTMLElement {
  const el = container.querySelector(`[data-testid="role-row-${role}"]`);
  if (!el) throw new Error(`no ${role} role row rendered`);
  return el as HTMLElement;
}

/** `allowed.readConfig` keyed by config key; anything not listed answers
 *  `null` (unset), matching the default mock. */
function mockRoleConfig(map: Record<string, unknown>) {
  allowed.readConfig.mockImplementation(async (k: string) => (k in map ? map[k] : null) as never);
}

function mockSessionModel(provider: string, model: string) {
  allowed.getConfig.mockResolvedValue({
    config: { GOOSE_PROVIDER: provider, GOOSE_MODEL: model },
    effective_goose_mode: 'auto',
  } as never);
}

function choose(role: 'chat' | 'voice' | 'harness'): HTMLButtonElement {
  const button = roleRow(role).querySelector(`[data-testid="choose-${role}"]`);
  if (!button) throw new Error(`no model chooser in the ${role} row`);
  return button as HTMLButtonElement;
}

describe('ModelsPanel Chat/Voice/Harness role table', () => {
  it('reads a default on all three rows when nothing is configured', async () => {
    await mount(vi.fn());
    // Chat and Harness say "(default) · built-in default"; Voice names its own
    // measured pair, because voice_model.rs resolves it above GOOSE_MODEL.
    expect(roleRow('chat').textContent).toContain('built-in default');
    expect(roleRow('harness').textContent).toContain('built-in default');
    expect(roleRow('voice').textContent).toContain('custom_deepseek / deepseek-chat (default)');
    expect(container.textContent).not.toContain('from GOOSE_MODEL');
  });

  it('falls Chat and Harness back to the session model, but NOT Voice', async () => {
    // The one place the three rows deliberately disagree. Chat and Harness
    // treat an explicit GOOSE_MODEL as the user's choice and defer to it;
    // voice_model.rs puts its measured default ABOVE GOOSE_MODEL, because a
    // spoken turn on a reasoning model is ten seconds of silence.
    mockSessionModel('zai', 'glm-5.3');
    await mount(vi.fn());
    expect(roleRow('chat').textContent).toContain('zai / glm-5.3');
    expect(roleRow('chat').textContent).toContain('from GOOSE_MODEL');
    expect(roleRow('harness').textContent).toContain('zai / glm-5.3');
    expect(roleRow('harness').textContent).toContain('from GOOSE_MODEL');
    expect(roleRow('voice').textContent).not.toContain('zai / glm-5.3');
    expect(roleRow('voice').textContent).toContain('custom_deepseek / deepseek-chat (default)');
  });

  it('separates Chat from Harness and Voice when only chat_* is set', async () => {
    // This is the whole point of splitting the knobs: setting Chat must not
    // leak into either of the other two.
    mockSessionModel('zai', 'glm-5.3');
    mockRoleConfig({ chat_provider: 'anthropic', chat_model: 'claude-sonnet-5' });
    await mount(vi.fn());
    expect(roleRow('chat').textContent).toContain('anthropic / claude-sonnet-5');
    expect(roleRow('chat').textContent).toContain('from chat_model');
    expect(roleRow('harness').textContent).toContain('zai / glm-5.3');
    expect(roleRow('harness').textContent).toContain('from GOOSE_MODEL');
    expect(roleRow('voice').textContent).not.toContain('claude-sonnet-5');
  });

  it('does not expose independently editable provider/model half-pairs', async () => {
    await mount(vi.fn());
    expect(roleRow('chat').querySelector('input[placeholder="provider"]')).toBeNull();
    expect(roleRow('chat').querySelector('input[placeholder^="model id"]')).toBeNull();
    expect(allowed.setModelRoute).not.toHaveBeenCalled();
  });

  it('writes both keys from one configured-model selection', async () => {
    await mount(vi.fn());
    await act(async () => { choose('chat').click(); });
    expect(allowed.setModelRoute).toHaveBeenCalledOnce();
    expect(allowed.setModelRoute).toHaveBeenCalledWith('chat', 'anthropic', 'claude-sonnet-5');
  });

  it('renders the Harness row as running on the session model when harness_model is "session"', async () => {
    mockRoleConfig({ harness_model: 'session' });
    await mount(vi.fn());
    expect(container.textContent).toContain('session model (explicit)');
  });
});

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
describe('ModelsPanel no longer hosts the agents\' own settings (J8/C7)', () => {
  it('writes no agent gate key at all — the switch has one home', async () => {
    const key = FEATURE_ROWS.find(r => r.key === 'strix_enabled')?.key;
    expect(key).toBe('strix_enabled');

    await mount(vi.fn());
    // No switch on this pane writes an agent flag. (Any toggle that appears
    // here in future must not be one of these.)
    for (const t of toggles()) {
      await act(async () => { t.click(); });
    }
    const keysWritten = allowed.upsertConfig.mock.calls.map(c => c[0]);
    expect(keysWritten).not.toContain('strix_enabled');
    expect(keysWritten).not.toContain('strix_sweep_hours');
    expect(keysWritten).not.toContain('watcher_topics');
  });

  it('does not read the relocated keys either — the pane is not a second reader', async () => {
    await mount(vi.fn());
    const keysRead = allowed.readConfig.mock.calls.map(c => c[0]);
    expect(keysRead).not.toContain('strix_enabled');
    expect(keysRead).not.toContain('strix_sweep_hours');
    expect(keysRead).not.toContain('watcher_topics');
    expect(allowed.getLibrarianSchedule).not.toHaveBeenCalled();
  });

  it('points at Agents instead, with the redirect convention Settings already uses', async () => {
    const goto = vi.fn();
    await mount(goto);
    const open = container.querySelector('[data-testid="models-open-agents"]') as HTMLButtonElement;
    expect(open).toBeTruthy();
    expect(open.textContent).toContain('Open Agents');
    await act(async () => { open.click(); });
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

describe('ModelsPanel role routing', () => {
  it('surfaces Apply recommended routing as the first action when prompted', async () => {
    allowed.getPacks.mockResolvedValue({
      prompt: true,
      configured: [],
      recommendation: {
        considered: ['openai/gpt-5.4', 'ollama/qwen3'],
        recommendations: [
          { role: 'edit', provider: 'openai', model: 'gpt-5.4' },
          { role: 'mechanical', provider: 'ollama', model: 'qwen3' },
        ],
      },
    } as never);

    await mount(vi.fn());
    expect(container.textContent).toContain('Apply recommended routing');
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
describe('ModelsPanel voice model row', () => {
  function sessionButton(): HTMLButtonElement {
    return Array.from(roleRow('voice').querySelectorAll('button')).find(
      b => b.textContent === 'Use session model',
    ) as HTMLButtonElement;
  }

  it('shows the measured default when neither key is set', async () => {
    await mount(vi.fn());
    expect(roleRow('voice').textContent).toContain('custom_deepseek / deepseek-chat (default)');
  });

  it('shows the configured route when both keys are set', async () => {
    allowed.readConfig.mockImplementation(async (k: string) =>
      (k === 'voice_provider' ? 'minimax' : k === 'voice_model' ? 'MiniMax-M2.7' : null) as never,
    );
    await mount(vi.fn());
    // Deliberately NOT the default pair — otherwise this passes on the default
    // readout and proves nothing about reading the configured keys. Scoped to
    // the row: Chat and Harness are unset here and legitimately say "(default)".
    expect(roleRow('voice').textContent).toContain('minimax / MiniMax-M2.7');
    expect(roleRow('voice').textContent).not.toContain('(default)');
  });

  it('shows "session model" when voice_model is set to session', async () => {
    allowed.readConfig.mockImplementation(async (k: string) => (k === 'voice_model' ? 'session' : null) as never);
    await mount(vi.fn());
    expect(roleRow('voice').textContent).toContain('session model');
  });

  it('does not write either key just from mounting', async () => {
    await mount(vi.fn());
    expect(allowed.setModelRoute).not.toHaveBeenCalled();
  });

  it('one configured-model selection writes both voice keys', async () => {
    await mount(vi.fn());
    await act(async () => { choose('voice').click(); });

    expect(allowed.setModelRoute).toHaveBeenCalledWith('voice', 'minimax', 'MiniMax-M2.7-highspeed');
  });

  it('"Use session model" clears the provider and writes the session sentinel', async () => {
    await mount(vi.fn());
    await act(async () => { sessionButton().click(); });

    expect(allowed.setModelRoute).toHaveBeenCalledOnce();
    expect(allowed.setModelRoute).toHaveBeenCalledWith('voice', '', 'session');
  });
});
