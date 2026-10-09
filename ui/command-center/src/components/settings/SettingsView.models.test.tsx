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
  startOllama: vi.fn(),
  pullOllamaModel: vi.fn(),
  getSystemInfo: vi.fn(),
  setProvider: vi.fn(),
  checkProvider: vi.fn(),
  getLibrarianSchedule: vi.fn(),
  setLibrarianSchedule: vi.fn(),
  runLibrarianNow: vi.fn(),
  getPacks: vi.fn(),
  applyPacks: vi.fn(),
  // ModelsPanel now also mounts LocalModelsSection (ST2, dagK settings pass) —
  // its own wiring tests live in LocalModelsSection.test.tsx; here it just
  // needs to resolve so it does not throw on mount.
  listLocalModels: vi.fn(async () => []),
  downloadLocalModel: vi.fn(),
  cancelLocalModelDownload: vi.fn(),
  deleteLocalModel: vi.fn(),
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
  ProviderModelPicker: ({ value, onChange, onUseSession, 'aria-label': label }: {
    value: { provider: string | null; model: string | null };
    onChange: (value: { provider: string; model: string }) => void;
    onUseSession?: () => void;
    'aria-label'?: string;
  }) => {
    const voice = label?.startsWith('Voice');
    // `data-picker` marks the element the row actually mounts, so a test can
    // assert WHERE in the row it sits; the value attributes record what the row
    // handed it, which is what decides whether its menu can open on a provider.
    return <div
      data-picker={voice ? 'voice' : label?.toLowerCase().split(' ')[0]}
      data-value-provider={value.provider ?? ''}
      data-value-model={value.model ?? ''}
    >
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
import { FEATURE_ROWS } from './workerGates';

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
  allowed.startOllama.mockReset().mockResolvedValue({ launched: false, method: null } as never);
  allowed.pullOllamaModel.mockReset();
  allowed.getSystemInfo.mockReset().mockResolvedValue({ total_ram_bytes: 16 * 1024 ** 3 } as never);
  allowed.setProvider.mockReset().mockResolvedValue(undefined as never);
  allowed.checkProvider.mockReset().mockResolvedValue(undefined as never);
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
  await act(async () => { root.render(<ModelsPanel goto={goto} section="models" />); });
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
    expect(container.textContent).not.toContain('set by your config');
  });

  it('falls Chat and Harness back to the session model, but NOT Voice', async () => {
    // The one place the three rows deliberately disagree. Chat and Harness
    // treat an explicit GOOSE_MODEL as the user's choice and defer to it;
    // voice_model.rs puts its measured default ABOVE GOOSE_MODEL, because a
    // spoken turn on a reasoning model is ten seconds of silence.
    mockSessionModel('zai', 'glm-5.3');
    await mount(vi.fn());
    expect(roleRow('chat').textContent).toContain('zai / glm-5.3');
    expect(roleRow('chat').textContent).toContain('set by your config');
    expect(roleRow('harness').textContent).toContain('zai / glm-5.3');
    expect(roleRow('harness').textContent).toContain('set by your config');
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
    expect(roleRow('chat').textContent).toContain('from your Chat setting');
    expect(roleRow('harness').textContent).toContain('zai / glm-5.3');
    expect(roleRow('harness').textContent).toContain('set by your config');
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

  it('shows the coordinator the daemon resolved for Harness, with its notice, not the raw GOOSE_MODEL', async () => {
    // The bug: the row read GOOSE_MODEL and printed
    // "anthropic / claude-haiku-4-5 · from GOOSE_MODEL", but a coding-harness
    // launch runs the coordinator gate and replaces an ungraduated model.
    // resolved_routes.harness is that gate's answer (model_roles.rs
    // effective_harness_route).
    const notice = 'anthropic/claude-haiku-4-5 is not graduated as a coordinator; using openai/gpt-5.4-mini';
    allowed.getConfig.mockResolvedValue({
      config: { GOOSE_PROVIDER: 'anthropic', GOOSE_MODEL: 'claude-haiku-4-5' },
      effective_goose_mode: 'auto',
      resolved_routes: {
        chat: { provider: 'anthropic', model: 'claude-haiku-4-5', source: 'session_model' },
        harness: { provider: 'openai', model: 'gpt-5.4-mini', source: 'coordinator_fallback', notice },
      },
    } as never);
    await mount(vi.fn());
    const harness = roleRow('harness');
    expect(harness.textContent).toContain('openai / gpt-5.4-mini');
    expect(harness.textContent).toContain('coordinator fallback');
    expect(harness.textContent).not.toContain('set by your config');
    expect(harness.querySelector('[data-testid="role-row-harness-notice"]')?.textContent).toBe(notice);
    // The picker opens on the provider the row names.
    expect(harness.querySelector('[data-picker="harness"]')?.getAttribute('data-value-provider')).toBe('openai');
    // Chat has no gate: it still reads the session model.
    expect(roleRow('chat').textContent).toContain('anthropic / claude-haiku-4-5');
    expect(roleRow('chat').textContent).toContain('set by your config');
    expect(roleRow('chat').querySelector('[data-testid="role-row-chat-notice"]')).toBeNull();
  });

  it('shows no notice when the resolved harness route is the one configured', async () => {
    allowed.getConfig.mockResolvedValue({
      config: {},
      effective_goose_mode: 'auto',
      resolved_routes: { harness: { provider: 'openai', model: 'gpt-5.4-mini', source: 'default' } },
    } as never);
    await mount(vi.fn());
    expect(roleRow('harness').textContent).toContain('openai / gpt-5.4-mini');
    expect(roleRow('harness').textContent).toContain('built-in default');
    expect(roleRow('harness').querySelector('[data-testid="role-row-harness-notice"]')).toBeNull();
  });

  it('falls back to the raw-key display when an older daemon sends no harness route', async () => {
    allowed.getConfig.mockResolvedValue({
      config: { GOOSE_PROVIDER: 'zai', GOOSE_MODEL: 'glm-5.3' },
      effective_goose_mode: 'auto',
      resolved_routes: { chat: { provider: 'zai', model: 'glm-5.3', source: 'session_model' } },
    } as never);
    await mount(vi.fn());
    expect(roleRow('harness').textContent).toContain('zai / glm-5.3');
    expect(roleRow('harness').textContent).toContain('set by your config');
  });

  it('re-reads the resolved harness route after saving a new pair', async () => {
    await mount(vi.fn());
    allowed.getConfig.mockResolvedValue({
      config: {},
      effective_goose_mode: 'auto',
      resolved_routes: {
        harness: {
          provider: 'anthropic', model: 'claude-sonnet-5', source: 'configured',
          notice: 'anthropic/claude-sonnet-5 is not graduated as a coordinator, but it was chosen explicitly, so it stays in control',
        },
      },
    } as never);
    const before = allowed.getConfig.mock.calls.length;
    await act(async () => { choose('harness').click(); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(allowed.setModelRoute).toHaveBeenCalledWith('harness', 'anthropic', 'claude-sonnet-5');
    expect(allowed.getConfig.mock.calls.length).toBeGreaterThan(before);
    expect(roleRow('harness').textContent).toContain('anthropic / claude-sonnet-5');
    expect(roleRow('harness').textContent).toContain('from your Harness setting');
    expect(roleRow('harness').textContent).toContain('chosen explicitly');
  });

  it('renders the Harness row as running on the session model when harness_model is "session"', async () => {
    mockRoleConfig({ harness_model: 'session' });
    await mount(vi.fn());
    expect(container.textContent).toContain('session model (explicit)');
  });

  // ── Layout ────────────────────────────────────────────────────────────
  //
  // The user: "the alignment is all off." Chat's and Harness's pickers ran the
  // full width of the row while Voice's hugged its own label, because Voice —
  // and only Voice — wrapped the picker in a `display: flex` box, which turns a
  // block child into a shrink-to-fit flex item. The picker renders a full-width
  // trigger and is not ours to change, so the row is what has to be right: all
  // three mount it as a direct child of a plain block body. These assertions
  // are about structure, not pixels, because jsdom has no layout.
  it('mounts all three pickers the same way, directly in a block row body', async () => {
    await mount(vi.fn());
    for (const role of ['chat', 'voice', 'harness'] as const) {
      const body = roleRow(role);
      const picker = body.querySelector(`[data-picker="${role}"]`);
      expect(picker, `no picker in the ${role} row`).not.toBeNull();
      // A wrapper between the two is exactly what made Voice shrink.
      expect(picker!.parentElement, `${role}'s picker is not a direct child of its row body`).toBe(body);
      expect(body.style.display, `${role}'s row body is not a block`).toBe('');
    }
  });

  it('puts the current-value line first in every row, at one offset', async () => {
    await mount(vi.fn());
    const offsets = (['chat', 'voice', 'harness'] as const).map(role => {
      const line = roleRow(role).firstElementChild as HTMLElement;
      return line.style.marginBottom;
    });
    expect(offsets[0]).not.toBe('');
    expect(new Set(offsets).size).toBe(1);
  });

  // ── What the picker is given to open on ───────────────────────────────
  //
  // The user: "I cant see the harness models available in the dropdown." With no
  // harness_* keys set the row printed the session model and handed the picker
  // a null provider, and the picker opens on `value.provider` — so the menu
  // listed provider headings and not one model. The row names a provider it
  // already displayed; it must not name a MODEL, because none is chosen for
  // this role and the trigger would then claim one was.
  it('hands each picker the provider its row just named, without claiming a model', async () => {
    mockSessionModel('anthropic', 'claude-haiku-4-5');
    await mount(vi.fn());
    for (const role of ['chat', 'harness'] as const) {
      const picker = roleRow(role).querySelector(`[data-picker="${role}"]`) as HTMLElement;
      expect(picker.getAttribute('data-value-provider'), `${role} has no provider to expand`).toBe('anthropic');
      expect(picker.getAttribute('data-value-model')).toBe('');
    }
    // Voice resolves above GOOSE_MODEL, so its provider is its own default.
    const voice = roleRow('voice').querySelector('[data-picker="voice"]') as HTMLElement;
    expect(voice.getAttribute('data-value-provider')).toBe('custom_deepseek');
    expect(voice.getAttribute('data-value-model')).toBe('');
  });

  it('keeps an explicitly configured role pointed at its own provider', async () => {
    mockSessionModel('zai', 'glm-5.3');
    mockRoleConfig({ harness_provider: 'anthropic', harness_model: 'claude-sonnet-5' });
    await mount(vi.fn());
    const picker = roleRow('harness').querySelector('[data-picker="harness"]') as HTMLElement;
    expect(picker.getAttribute('data-value-provider')).toBe('anthropic');
    expect(picker.getAttribute('data-value-model')).toBe('claude-sonnet-5');
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

const ollamaStatus = (names: string[] = []) => ({
  reachable: true,
  installed: names.map(name => ({ name, size: 2_000_000_000, digest: 'd', modified_at: '' })),
  running: [],
});

function modelButton(label: string): HTMLButtonElement {
  const button = [...container.querySelectorAll('button')].find(item => item.textContent?.trim() === label);
  if (!button) throw new Error(`No ${label} button in Models`);
  return button;
}

describe('ModelsPanel Ollama setup', () => {
  it('keeps a failed status read recoverable without showing stale connection state', async () => {
    allowed.getOllamaStatus.mockRejectedValueOnce(new Error('status timed out'))
      .mockResolvedValue({ reachable: false, installed: [], running: [] } as never);
    await mount(vi.fn());
    expect(container.textContent).toContain('status timed out');
    expect(container.textContent).not.toContain('Connected');
    await act(async () => { modelButton('Check again').click(); });
    expect(container.textContent).toContain('Ollama not running');
  });

  it('offers explicit start, official download and recheck when Ollama is down', async () => {
    const open = vi.spyOn(window, 'open').mockImplementation(() => null);
    try {
      await mount(vi.fn());
      expect(container.textContent).toContain('Ollama not running');
      expect(allowed.startOllama).not.toHaveBeenCalled();
      expect(allowed.pullOllamaModel).not.toHaveBeenCalled();

      await act(async () => { modelButton('Start Ollama').click(); });
      expect(allowed.startOllama).toHaveBeenCalledOnce();
      expect(container.textContent).toContain('could not be started');
      await act(async () => { modelButton('Get Ollama ↗').click(); });
      expect(open).toHaveBeenCalledWith('https://ollama.com/download', '_blank', 'noopener,noreferrer');
      await act(async () => { modelButton('Check again').click(); });
      expect(allowed.getOllamaStatus).toHaveBeenCalledTimes(2);
    } finally {
      open.mockRestore();
    }
  });

  it('checks the service after an explicit successful start before showing Connected', async () => {
    allowed.getOllamaStatus.mockResolvedValueOnce({ reachable: false, installed: [], running: [] } as never)
      .mockResolvedValueOnce(ollamaStatus() as never);
    allowed.startOllama.mockResolvedValue({ launched: true, method: 'app' } as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Start Ollama').click(); });
    expect(allowed.startOllama).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Connected');
    expect(container.textContent).not.toContain('ready for Chat');
  });

  it('does not call an empty SSE stream a downloaded model', async () => {
    allowed.getOllamaStatus.mockResolvedValue(ollamaStatus() as never);
    allowed.pullOllamaModel.mockReturnValue({ promise: Promise.resolve(), abort: vi.fn() } as never);
    await mount(vi.fn());
    expect(container.textContent).toContain('has no models installed');
    await act(async () => { modelButton('Download qwen3:8b').click(); });
    expect(allowed.pullOllamaModel).toHaveBeenCalledWith('qwen3:8b', expect.any(Function));
    expect(allowed.getOllamaStatus).toHaveBeenCalledTimes(2);
    expect(container.textContent).toContain('Download could not be verified');
    expect(container.textContent).not.toContain('ready for Chat');
    expect(modelButton('Download qwen3:8b')).toBeTruthy();
  });

  it('stops waiting without claiming to cancel a server download', async () => {
    allowed.getOllamaStatus.mockResolvedValue(ollamaStatus() as never);
    let rejectPull!: (error: Error) => void;
    const abort = vi.fn(() => rejectPull(new DOMException('Aborted', 'AbortError')));
    allowed.pullOllamaModel.mockReturnValue({
      promise: new Promise<void>((_, reject) => { rejectPull = reject; }), abort,
    } as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Download qwen3:8b').click(); });
    await act(async () => { modelButton('Stop waiting').click(); });
    expect(abort).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Ollama may continue downloading');
    expect(container.textContent).not.toContain('Download stopped');
  });

  it('requires installed inventory, then a separate live check to say Chat is ready', async () => {
    allowed.getOllamaStatus.mockResolvedValueOnce(ollamaStatus() as never)
      .mockResolvedValue(ollamaStatus(['qwen3:8b']) as never);
    allowed.pullOllamaModel.mockReturnValue({ promise: Promise.resolve(), abort: vi.fn() } as never);
    allowed.checkProvider.mockResolvedValue({ live: true, model: 'qwen3:8b' } as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Download qwen3:8b').click(); });
    expect(container.textContent).toContain('is installed');
    expect(allowed.setModelRoute).not.toHaveBeenCalled();
    await act(async () => { modelButton('Use for Chat').click(); });
    expect(allowed.setModelRoute).toHaveBeenCalledWith('chat', 'ollama', 'qwen3:8b');
    expect(allowed.setProvider).not.toHaveBeenCalled();
    expect(allowed.checkProvider).toHaveBeenCalledWith('ollama');
    expect(container.textContent).toContain('Ollama answered a live model-list check');
  });

  it('keeps an inconclusive live check distinct from ready', async () => {
    allowed.getOllamaStatus.mockResolvedValue(ollamaStatus(['small:3b']) as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Use for Chat').click(); });
    expect(container.textContent).toContain('live model-list check did not confirm');
    expect(container.textContent).not.toContain('ready for Chat');
  });

  it.each(['down', 'failed read'])('clears an earlier green model notice when Ollama is %s', async outcome => {
    allowed.getOllamaStatus.mockResolvedValue(ollamaStatus(['small:3b']) as never);
    allowed.checkProvider.mockResolvedValue({ live: true, model: 'small:3b' } as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Use for Chat').click(); });
    expect(container.textContent).toContain('Ollama answered a live model-list check');

    if (outcome === 'down') {
      allowed.getOllamaStatus.mockResolvedValueOnce({ reachable: false, installed: [], running: [] } as never);
    } else {
      allowed.getOllamaStatus.mockRejectedValueOnce(new Error('status timed out'));
    }
    await act(async () => { modelButton('Check again').click(); });
    expect(container.textContent).not.toContain('Ollama answered a live model-list check');
    expect(container.textContent).toContain(outcome === 'down' ? 'Ollama not running' : 'status timed out');
  });

  it('does not save a stale installed-model row as the Chat route', async () => {
    allowed.getOllamaStatus.mockResolvedValueOnce(ollamaStatus(['small:3b']) as never)
      .mockResolvedValue(ollamaStatus() as never);
    await mount(vi.fn());
    await act(async () => { modelButton('Use for Chat').click(); });
    expect(allowed.setModelRoute).not.toHaveBeenCalled();
    expect(allowed.checkProvider).not.toHaveBeenCalled();
    expect(container.textContent).toContain('no longer shown as installed');
  });

  it('keeps the hosted setup path when this Mac has no safe local recommendation', async () => {
    allowed.getOllamaStatus.mockResolvedValue(ollamaStatus() as never);
    allowed.getSystemInfo.mockResolvedValue({ total_ram_bytes: 4 * 1024 ** 3 } as never);
    const goto = vi.fn();
    await mount(goto);
    expect(container.textContent).toContain('No safe local-model recommendation');
    expect(container.textContent).not.toContain('Download qwen3');
    await act(async () => { modelButton('Set up a hosted provider').click(); });
    expect(goto).toHaveBeenCalledWith('keys');
  });
});
