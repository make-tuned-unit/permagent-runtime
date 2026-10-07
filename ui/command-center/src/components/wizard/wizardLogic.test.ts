// @vitest-environment jsdom
/**
 * The pure decisions behind first-run setup: which providers it can offer,
 * what a key check proved, what the review says, the persona it saves, and
 * how an interrupted run resumes.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/api', () => ({ api: {}, apiFetch: vi.fn() }));
import { keyProviders, hasLocalRuntime, splitStepLink, type ProviderEntry } from './providerChoices';
import { effectiveName, greetingFor, toneById, TONES, FALLBACK_AGENT_NAME } from './persona';
import { DRAFT_KEY, loadDraft, saveDraft, clearDraft } from './wizardDraft';
import { describeCheck, friendlyCheckError } from './StepModel';
import { summarize } from './StepReview';

function provider(name: string, display: string, keys: ProviderEntry['metadata']['config_keys'], extra: Partial<ProviderEntry> = {}): ProviderEntry {
  return {
    name, provider_type: 'Builtin', is_configured: false, is_default: false,
    metadata: { name, display_name: display, description: '', default_model: `${name}-m`, known_models: [], config_keys: keys },
    ...extra,
  };
}

describe('providerChoices', () => {
  const list = [
    provider('zeta', 'Zeta', [{ name: 'ZETA_KEY', required: true, secret: true }]),
    provider('alpha', 'Alpha', [{ name: 'ALPHA_KEY', required: true, secret: true }, { name: 'ALPHA_HOST', required: true, secret: false, default: 'https://a.example' }]),
    provider('needs-endpoint', 'Needs endpoint', [{ name: 'E_KEY', required: true, secret: true }, { name: 'E_HOST', required: true, secret: false }]),
    provider('signin', 'Sign-in', [{ name: 'S_TOKEN', required: true, secret: true, oauth_flow: true }]),
    provider('two-secrets', 'Two secrets', [{ name: 'A', required: true, secret: true }, { name: 'B', required: true, secret: true }]),
    provider('ollama', 'Ollama', [{ name: 'OLLAMA_HOST', required: true, secret: false, default: 'localhost' }]),
  ];

  it('offers only one-key providers it can finish in one screen, alphabetically, none pre-selected', () => {
    const offered = keyProviders(list);
    expect(offered.map(p => p.name)).toEqual(['alpha', 'zeta']);
    expect(offered.every(p => !p.isDefault)).toBe(true);
    expect(offered[0].secretKey).toBe('ALPHA_KEY');
  });

  it('knows whether the local runtime is available', () => {
    expect(hasLocalRuntime(list)).toBe(true);
    expect(hasLocalRuntime(list.filter(p => p.name !== 'ollama'))).toBe(false);
  });

  it('splits a setup step around its link', () => {
    expect(splitStepLink('Go to https://x.example/keys now')).toEqual({ before: 'Go to ', url: 'https://x.example/keys', after: ' now' });
    expect(splitStepLink('Click Create')).toEqual({ before: 'Click Create', url: null, after: '' });
  });
});

describe('key check results', () => {
  it('says "Key works" only for a live answer', () => {
    expect(describeCheck({ live: true, model: 'm-1', models_available: 3, note: null })).toEqual({ tone: 'ok', text: 'Key works · m-1' });
    const inconclusive = describeCheck({ live: false, model: 'm-1', models_available: 0, note: 'Network error: reset' });
    expect(inconclusive.tone).toBe('warn');
    expect(inconclusive.text).toContain('Network error: reset');
    expect(describeCheck(undefined).tone).toBe('warn');
  });

  it('strips the route prefix and starts with a capital', () => {
    expect(friendlyCheckError("Provider 'acme' check failed: the provider rejected this key (401)"))
      .toBe('The provider rejected this key (401)');
  });
});

describe('persona', () => {
  it('uses the default name only when the field is blank', () => {
    expect(effectiveName('  ', 'Aria')).toBe('Aria');
    expect(effectiveName(' Nova ', 'Aria')).toBe('Nova');
    expect(effectiveName('', '')).toBe(FALLBACK_AGENT_NAME);
  });

  it('writes a greeting that reads as one sentence per tone, with no repeated traits', () => {
    for (const t of TONES) {
      const g = greetingFor('Nova', t.id);
      expect(g.startsWith("Hi, I'm Nova. ")).toBe(true);
      expect(new Set(t.traits).size).toBe(t.traits.length);
    }
    const all = TONES.flatMap(t => t.traits);
    expect(new Set(all).size).toBe(all.length);
    expect(toneById('nope').id).toBe(TONES[0].id);
  });
});

describe('resume draft', () => {
  afterEach(() => localStorage.clear());

  it('round-trips and clamps the step', () => {
    saveDraft({ step: 4, name: 'Nova', toneId: 'warm', firstMessage: 'hi', modelChoice: 'local' });
    expect(loadDraft(6)).toEqual({ step: 4, name: 'Nova', toneId: 'warm', firstMessage: 'hi', modelChoice: 'local' });
    localStorage.setItem(DRAFT_KEY, JSON.stringify({ step: 99 }));
    expect(loadDraft(6).step).toBe(5);
    clearDraft();
    expect(localStorage.getItem(DRAFT_KEY)).toBeNull();
  });

  it('ignores malformed drafts field by field', () => {
    localStorage.setItem(DRAFT_KEY, '{not json');
    expect(loadDraft(6).step).toBe(0);
    localStorage.setItem(DRAFT_KEY, JSON.stringify({ step: 'two', name: 7, modelChoice: 'cloud' }));
    expect(loadDraft(6)).toEqual({ step: 0, name: '', toneId: '', firstMessage: '', modelChoice: null });
  });
});

describe('review summary', () => {
  const providers = [
    { name: 'acme', is_configured: true, metadata: { display_name: 'Acme AI' } },
    { name: 'ollama', is_configured: true, metadata: { display_name: 'Ollama' } },
  ];

  it('reports what the backend holds', () => {
    const f = summarize({
      config: { GOOSE_PROVIDER: 'acme', GOOSE_MODEL: 'acme-1', dev_roots: ['/a', '/b'] },
      providers, searchSaved: ['Search Co'], librarian: { enabled: true, model: 'small:3b' },
    });
    expect(f).toEqual({
      model: { connected: true, text: 'Acme AI · acme-1' },
      folders: '2 folders', search: 'Search Co', memory: 'On · small:3b',
    });
  });

  it('says plainly when no model is connected', () => {
    const f = summarize({ config: {}, providers, searchSaved: [], librarian: null });
    expect(f.model).toEqual({ connected: false, text: 'Not connected' });
    expect(f.folders).toBe('None');
    expect(f.memory).toBe('Off');
  });

  it('names a local model as on this Mac', () => {
    const f = summarize({ config: { GOOSE_PROVIDER: 'ollama', GOOSE_MODEL: 'small:3b' }, providers, searchSaved: [], librarian: null });
    expect(f.model.text).toBe('On this Mac · small:3b');
  });
});
