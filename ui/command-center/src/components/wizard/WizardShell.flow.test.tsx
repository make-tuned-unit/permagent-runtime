// @vitest-environment jsdom
/**
 * First-run setup end to end against a mocked background service: the hosted
 * key path (rejected, then proved live), the local-model path, the no-model
 * path, the save-failure banner and its Retry, resume from a draft, Escape =
 * Back, focus moving into each step, and the listbox keyboard contract.
 *
 * jsdom does not perform a browser's implicit form submission, so "Enter"
 * here is the form's submit event — the same handler a real Enter reaches.
 * The real-keyboard walk (Tab / Space / arrows / Enter / Escape in Chrome) is
 * in the lane's Playwright evidence.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const m = vi.hoisted(() => ({
  getProviders: vi.fn(), upsertConfig: vi.fn(), setProvider: vi.fn(), checkProvider: vi.fn(),
  getSecretSources: vi.fn(), testSecretSource: vi.fn(), setSecretSource: vi.fn(),
  getOllamaStatus: vi.fn(), startOllama: vi.fn(), pullOllamaModel: vi.fn(), getSystemInfo: vi.fn(),
  getDevRoots: vi.fn(), checkDevRoot: vi.fn(), getConfig: vi.fn(), readSecretConfig: vi.fn(),
  getLibrarianSchedule: vi.fn(), setLibrarianSchedule: vi.fn(), probeExtension: vi.fn(), addExtension: vi.fn(),
  apiFetch: vi.fn(),
  stash: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: {
    getProviders: m.getProviders, upsertConfig: m.upsertConfig, setProvider: m.setProvider, checkProvider: m.checkProvider,
    getSecretSources: m.getSecretSources, testSecretSource: m.testSecretSource, setSecretSource: m.setSecretSource,
    getOllamaStatus: m.getOllamaStatus, startOllama: m.startOllama, pullOllamaModel: m.pullOllamaModel, getSystemInfo: m.getSystemInfo,
    getDevRoots: m.getDevRoots, checkDevRoot: m.checkDevRoot, getConfig: m.getConfig, readSecretConfig: m.readSecretConfig,
    getLibrarianSchedule: m.getLibrarianSchedule, setLibrarianSchedule: m.setLibrarianSchedule,
    probeExtension: m.probeExtension, addExtension: m.addExtension,
  },
  apiFetch: m.apiFetch,
}));
vi.mock('../../lib/wizardIntent', () => ({ stashWizardIntent: m.stash }));
vi.mock('../mobius/Mobius', () => ({ Mobius: () => null }));
vi.mock('../../styles/useTheme', async () => {
  const { getThemedColors } = await import('../../styles/tokens');
  return { useTheme: () => ({ colors: getThemedColors(), reduceMotion: true, theme: 'dark' }) };
});

import { WizardShell } from './WizardShell';
import { DRAFT_KEY } from './wizardDraft';

const providers = (configured: Record<string, boolean> = {}, def = '') => [
  { name: 'acme', provider_type: 'Builtin', is_configured: !!configured.acme, is_default: def === 'acme',
    metadata: { name: 'acme', display_name: 'Acme AI', description: '', default_model: 'acme-swift', known_models: [],
      config_keys: [{ name: 'ACME_API_KEY', required: true, secret: true }], setup_steps: ['Go to https://keys.example'] } },
  { name: 'ollama', provider_type: 'Builtin', is_configured: true, is_default: def === 'ollama',
    metadata: { name: 'ollama', display_name: 'Ollama', description: '', default_model: 'x', known_models: [],
      config_keys: [{ name: 'OLLAMA_HOST', required: true, secret: false, default: 'localhost' }] } },
];

let container: HTMLDivElement;
let root: Root;
let onComplete: ReturnType<typeof vi.fn>;
let config: Record<string, unknown>;

beforeEach(() => {
  localStorage.clear();
  Object.values(m).forEach(fn => fn.mockReset());
  config = { wizard_complete: false };
  m.getProviders.mockImplementation(async () => providers({ acme: 'ACME_API_KEY' in config }, String(config.GOOSE_PROVIDER ?? '')));
  m.getSecretSources.mockRejectedValue(new Error('no managers'));
  m.upsertConfig.mockImplementation(async (key: string, value: unknown) => { config[key] = value; return 'ok'; });
  m.setProvider.mockImplementation(async (p: string, model: string) => { config.GOOSE_PROVIDER = p; config.GOOSE_MODEL = model; });
  m.checkProvider.mockResolvedValue({ live: true, model: 'acme-swift', models_available: 9, note: null });
  m.getDevRoots.mockResolvedValue({ confirmed: [], discovered: ['/home/u/code'], home: '/home/u' });
  m.getConfig.mockImplementation(async () => ({ config }));
  m.readSecretConfig.mockResolvedValue(null);
  m.getLibrarianSchedule.mockResolvedValue({ enabled: false, start_time: '02:00', duration_minutes: 60, model: 'm', run_if_launched_in_window: false });
  m.getOllamaStatus.mockResolvedValue({ reachable: true, installed: [{ name: 'small:3b', size: 2e9, digest: 'd', modified_at: '' }], running: [] });
  m.getSystemInfo.mockResolvedValue({ total_ram_bytes: 16 * 1024 ** 3 });
  m.apiFetch.mockImplementation(async (path: string, init?: RequestInit) => {
    if (path === '/api/agent/identity' && (!init || !init.method)) {
      return { first_name: 'Aria', last_name: null, nickname: null, voice_id: 'voice-1' };
    }
    return {};
  });
  onComplete = vi.fn();
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const flush = async () => { await act(async () => { for (let i = 0; i < 8; i += 1) await Promise.resolve(); }); };
const text = () => container.textContent ?? '';
const progress = () => container.querySelector('[aria-live="polite"]')?.textContent;
const heading = () => container.querySelector('main h1')?.textContent;
function byText<T extends Element = HTMLElement>(selector: string, label: string | RegExp): T {
  const el = Array.from(container.querySelectorAll<T>(selector)).find(e => (typeof label === 'string' ? e.textContent?.trim() === label : label.test(e.textContent ?? '')));
  if (!el) throw new Error(`no ${selector} with ${label}; text: ${text().slice(0, 400)}`);
  return el;
}
const click = async (el: Element) => { await act(async () => { (el as HTMLElement).click(); }); await flush(); };
const submit = async () => {
  const form = container.querySelector('main form') as HTMLFormElement;
  await act(async () => { form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })); });
  await flush();
};
const primaryLabel = () => (container.querySelector('main form button[type="submit"]') as HTMLButtonElement).textContent?.trim();
const typeInto = async (input: HTMLInputElement | HTMLTextAreaElement, value: string) => {
  const proto = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')!.set!;
  await act(async () => { setter.call(input, value); input.dispatchEvent(new Event('input', { bubbles: true })); });
};
const radio = (value: string) => container.querySelector(`input[type="radio"][value="${value}"]`) as HTMLInputElement;
const key = async (el: Element, k: string) => {
  await act(async () => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); });
  await flush();
};

async function start() {
  act(() => root.render(<WizardShell onComplete={onComplete} />));
  await flush();
}

async function chooseAcmeAndTest(keyText: string) {
  await click(radio('hosted'));
  await click(container.querySelector('button[aria-label="Provider"]')!);
  await click(byText('[role="option"]', /Acme AI/));
  await typeInto(container.querySelector('input[type="password"]') as HTMLInputElement, keyText);
  await submit();
}

describe('WizardShell', () => {
  it('hosted key: rejected key blocks, a working key is proved live, and setup saves the persona', async () => {
    await start();
    expect(progress()).toBe('Step 1 of 6 · Welcome');
    expect(document.activeElement?.textContent).toContain('Get started');
    await submit();
    expect(progress()).toBe('Step 2 of 6 · Model');
    // Focus moved into the new step, on its first choice.
    expect((document.activeElement as HTMLInputElement).value).toBe('hosted');
    expect(primaryLabel()).toBe('Continue');
    expect((container.querySelector('main form button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);

    m.checkProvider.mockRejectedValueOnce(new Error("Provider 'acme' check failed: the provider rejected this key (401 invalid)"));
    await chooseAcmeAndTest('bad-key');
    expect(m.upsertConfig).toHaveBeenCalledWith('ACME_API_KEY', 'bad-key', true);
    expect(m.setProvider).toHaveBeenCalledWith('acme', 'acme-swift');
    expect(text()).toContain('The provider rejected this key (401 invalid)');
    expect(primaryLabel()).toBe('Test key');
    expect(progress()).toBe('Step 2 of 6 · Model');

    await typeInto(container.querySelector('input[type="password"]') as HTMLInputElement, 'good-key');
    await submit();
    expect(text()).toContain('Key works · acme-swift');
    expect(primaryLabel()).toBe('Continue');
    // The key no longer lives in the form once stored.
    expect((container.querySelector('input[type="password"]') as HTMLInputElement).value).toBe('');
    await submit();

    expect(progress()).toBe('Step 3 of 6 · Agent');
    expect((document.activeElement as HTMLInputElement).placeholder).toBe('Aria');
    await click(radio('direct'));
    expect(text()).toContain("Hi, I'm Aria. Tell me what you need and I'll get straight to it.");
    await submit();

    expect(progress()).toBe('Step 4 of 6 · Code folders');
    expect(text()).toContain('/home/u/code');
    await submit();
    expect(m.upsertConfig).toHaveBeenCalledWith('dev_roots', ['/home/u/code']);

    expect(progress()).toBe('Step 5 of 6 · Extras');
    expect(primaryLabel()).toBe('Skip for now');
    await submit();

    expect(progress()).toBe('Step 6 of 6 · Review');
    expect(text()).toContain('Acme AI · acme-swift');
    expect(text()).toContain('Aria · Direct');
    expect(text()).not.toContain("can't reply yet");
    await typeInto(container.querySelector('textarea') as HTMLTextAreaElement, 'Plan my week');
    await submit();

    const put = m.apiFetch.mock.calls.find(([, init]) => (init as RequestInit | undefined)?.method === 'PUT');
    expect(put).toBeTruthy();
    const body = JSON.parse((put![1] as RequestInit).body as string);
    expect(body).toMatchObject({
      first_name: 'Aria', traits: ['direct', 'concise'], voice_id: 'voice-1',
      opening_greeting: "Hi, I'm Aria. Tell me what you need and I'll get straight to it.",
    });
    expect(m.upsertConfig).toHaveBeenCalledWith('wizard_complete', true);
    expect(m.stash).toHaveBeenCalledWith('Plan my week');
    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(DRAFT_KEY)).toBeNull();
  });

  it('local model: an installed model is checked before Continue', async () => {
    await start();
    await submit();
    await click(radio('local'));
    expect(text()).toContain('small:3b');
    expect(primaryLabel()).toBe('Use this model');
    m.checkProvider.mockResolvedValueOnce({ live: true, model: 'small:3b', models_available: 1, note: null });
    await submit();
    expect(m.setProvider).toHaveBeenCalledWith('ollama', 'small:3b');
    expect(m.checkProvider).toHaveBeenCalledWith('ollama');
    expect(text()).toContain('Ready · small:3b runs on this Mac');
    await submit();
    expect(progress()).toBe('Step 3 of 6 · Agent');
    // Nothing was started or downloaded without being asked.
    expect(m.startOllama).not.toHaveBeenCalled();
    expect(m.pullOllamaModel).not.toHaveBeenCalled();
  });

  it('local model: Ollama not running is said plainly and only started on request', async () => {
    m.getOllamaStatus.mockResolvedValue({ reachable: false, installed: [], running: [] });
    await start();
    await submit();
    await click(radio('local'));
    expect(text()).toContain("Ollama isn't running on this Mac");
    expect((container.querySelector('main form button[type="submit"]') as HTMLButtonElement).disabled).toBe(true);
    expect(m.startOllama).not.toHaveBeenCalled();
  });

  it('no key and no local model: finishes with the limitation stated on the review', async () => {
    await start();
    await submit();
    await click(radio('later'));
    expect(primaryLabel()).toBe('Continue without a model');
    await submit();
    await submit(); // agent: defaults
    await click(byText('button', /I don't write code/));
    expect(m.upsertConfig).toHaveBeenCalledWith('dev_roots', []);
    await submit(); // extras: skip
    expect(progress()).toBe('Step 6 of 6 · Review');
    expect(text()).toContain('Not connected');
    expect(text()).toContain("No model is connected, so Aria can't reply yet");
    await submit();
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('save failure shows the banner; Retry saves and finishes', async () => {
    localStorage.setItem(DRAFT_KEY, JSON.stringify({ step: 5, name: 'Nova', toneId: 'warm', firstMessage: '', modelChoice: 'later' }));
    let fail = true;
    m.apiFetch.mockImplementation(async (path: string, init?: RequestInit) => {
      if (init?.method === 'PUT') {
        if (fail) { fail = false; throw new Error('disk is read-only'); }
        return {};
      }
      return { first_name: 'Aria', last_name: null, nickname: null, voice_id: null };
    });
    await start();
    expect(progress()).toBe('Step 6 of 6 · Review');
    await submit();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Couldn't save your setup (disk is read-only)");
    expect(onComplete).not.toHaveBeenCalled();
    expect(m.upsertConfig).not.toHaveBeenCalledWith('wizard_complete', true);
    await click(byText('button', 'Retry'));
    expect(m.upsertConfig).toHaveBeenCalledWith('wizard_complete', true);
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('resumes an interrupted run at its step with its choices', async () => {
    localStorage.setItem(DRAFT_KEY, JSON.stringify({ step: 2, name: 'Nova', toneId: 'curious', firstMessage: '', modelChoice: 'hosted' }));
    await start();
    expect(progress()).toBe('Step 3 of 6 · Agent');
    expect((container.querySelector('main input[type="text"], main input:not([type])') as HTMLInputElement).value).toBe('Nova');
    expect(radio('curious').checked).toBe(true);
    expect(text()).toContain("Hi, I'm Nova. What are we exploring today?");
  });

  it('Escape goes back a step, keeps what was typed, and Change on the review returns to the review', async () => {
    localStorage.setItem(DRAFT_KEY, JSON.stringify({ step: 2, name: '', toneId: '', firstMessage: '', modelChoice: 'later' }));
    await start();
    const name = document.activeElement as HTMLInputElement;
    await typeInto(name, 'Orbit');
    await key(name, 'Escape');
    expect(progress()).toBe('Step 2 of 6 · Model');
    await submit(); // "decide later" was restored from the draft
    expect(progress()).toBe('Step 3 of 6 · Agent');
    expect((document.activeElement as HTMLInputElement).value).toBe('Orbit');
    await submit(); await submit(); await submit(); // agent → folders (2 found) → extras → review
    expect(progress()).toBe('Step 6 of 6 · Review');
    await click(container.querySelector('button[aria-label="Change agent"]')!);
    expect(progress()).toBe('Step 3 of 6 · Agent');
    await submit();
    expect(progress()).toBe('Step 6 of 6 · Review');
  });

  it('listbox: arrows open and move, Enter chooses, Escape closes without leaving the step', async () => {
    m.getProviders.mockResolvedValue([
      ...providers(),
      { ...providers()[0], name: 'beta', metadata: { ...providers()[0].metadata, name: 'beta', display_name: 'Beta Cloud', config_keys: [{ name: 'BETA_KEY', required: true, secret: true }] } },
    ]);
    await start();
    await submit();
    await click(radio('hosted'));
    const trigger = container.querySelector('button[aria-label="Provider"]') as HTMLButtonElement;
    await key(trigger, 'ArrowDown');
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement?.textContent).toContain('Acme AI');
    await key(document.activeElement!, 'ArrowDown');
    expect(document.activeElement?.textContent).toContain('Beta Cloud');
    await key(document.activeElement!, 'Escape');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(progress()).toBe('Step 2 of 6 · Model');
    await key(trigger, 'ArrowDown');
    await key(document.activeElement!, 'End');
    await key(document.activeElement!, 'Enter');
    expect(trigger.textContent).toContain('Beta Cloud');
    expect(document.activeElement).toBe(trigger);
  });
});
