import { describe, it, expect } from 'vitest';
import {
  buildCustomProviderPayload,
  parseModels,
  emptyCustomProviderForm,
  type CustomProviderForm,
} from './customProvider';

function form(overrides: Partial<CustomProviderForm> = {}): CustomProviderForm {
  return {
    displayName: 'My LLM',
    engine: 'openai_compatible',
    apiUrl: 'https://api.example.com',
    apiKey: 'sk-test',
    models: 'gpt-4o, gpt-4o-mini',
    requiresAuth: true,
    ...overrides,
  };
}

describe('parseModels', () => {
  it('splits on commas and newlines, trims, and drops blanks', () => {
    expect(parseModels('a, b\nc ,  \n d')).toEqual(['a', 'b', 'c', 'd']);
  });

  it('de-duplicates while preserving first-seen order', () => {
    expect(parseModels('a, b, a, c, b')).toEqual(['a', 'b', 'c']);
  });

  it('returns an empty array for blank input', () => {
    expect(parseModels('   \n , ')).toEqual([]);
  });
});

describe('buildCustomProviderPayload', () => {
  it('builds the wire payload for a valid form', () => {
    const res = buildCustomProviderPayload(form());
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.payload).toEqual({
      engine: 'openai_compatible',
      display_name: 'My LLM',
      api_url: 'https://api.example.com',
      api_key: 'sk-test',
      models: ['gpt-4o', 'gpt-4o-mini'],
      requires_auth: true,
    });
  });

  it('trims display name, url, and key', () => {
    const res = buildCustomProviderPayload(
      form({ displayName: '  My LLM  ', apiUrl: '  https://api.example.com  ', apiKey: '  sk-x  ' }),
    );
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.payload.display_name).toBe('My LLM');
    expect(res.payload.api_url).toBe('https://api.example.com');
    expect(res.payload.api_key).toBe('sk-x');
  });

  it('requires a display name', () => {
    const res = buildCustomProviderPayload(form({ displayName: '   ' }));
    expect(res).toEqual({ ok: false, error: 'Display name is required.' });
  });

  it('requires an API url', () => {
    const res = buildCustomProviderPayload(form({ apiUrl: '' }));
    expect(res).toEqual({ ok: false, error: 'An API URL is required.' });
  });

  it('rejects a non-http(s) url', () => {
    const res = buildCustomProviderPayload(form({ apiUrl: 'ftp://nope.example.com' }));
    expect(res.ok).toBe(false);
    if (res.ok) return;
    expect(res.error).toMatch(/http:\/\/ or https:\/\//);
  });

  it('rejects a malformed url', () => {
    const res = buildCustomProviderPayload(form({ apiUrl: 'not a url' }));
    expect(res.ok).toBe(false);
  });

  it('requires at least one model', () => {
    const res = buildCustomProviderPayload(form({ models: '  ,  \n ' }));
    expect(res).toEqual({ ok: false, error: 'Add at least one model id.' });
  });

  it('requires an API key when requiresAuth is true', () => {
    const res = buildCustomProviderPayload(form({ apiKey: '   ' }));
    expect(res.ok).toBe(false);
    if (res.ok) return;
    expect(res.error).toMatch(/API key is required/);
  });

  it('allows an empty key when auth is not required (e.g. local Ollama)', () => {
    const res = buildCustomProviderPayload(
      form({ engine: 'ollama_compatible', apiKey: '', requiresAuth: false, apiUrl: 'http://localhost:11434' }),
    );
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.payload.requires_auth).toBe(false);
    expect(res.payload.api_key).toBe('');
    expect(res.payload.engine).toBe('ollama_compatible');
  });

  it('passes the selected engine through unchanged', () => {
    const res = buildCustomProviderPayload(form({ engine: 'anthropic_compatible' }));
    expect(res.ok).toBe(true);
    if (!res.ok) return;
    expect(res.payload.engine).toBe('anthropic_compatible');
  });
});

describe('emptyCustomProviderForm', () => {
  it('defaults to an openai-compatible, auth-required form', () => {
    expect(emptyCustomProviderForm()).toEqual({
      displayName: '',
      engine: 'openai_compatible',
      apiUrl: '',
      apiKey: '',
      models: '',
      requiresAuth: true,
    });
  });
});
