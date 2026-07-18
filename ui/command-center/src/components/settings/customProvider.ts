import type { CustomProviderPayload } from '../../lib/api';

/** The compatible engines a custom provider can target. Kept in sync with the
 *  backend `create_custom_provider` match arms (openai/anthropic/ollama). */
export type CustomProviderEngine = CustomProviderPayload['engine'];

export const ENGINE_OPTIONS: Array<{ value: CustomProviderEngine; label: string }> = [
  { value: 'openai_compatible', label: 'OpenAI-compatible' },
  { value: 'anthropic_compatible', label: 'Anthropic-compatible' },
  { value: 'ollama_compatible', label: 'Ollama-compatible' },
];

/** Raw form state captured from the Add-custom-provider modal. `models` is the
 *  free-text field (comma/newline separated) the user types. */
export interface CustomProviderForm {
  displayName: string;
  engine: CustomProviderEngine;
  apiUrl: string;
  apiKey: string;
  /** Comma- or newline-separated model ids. */
  models: string;
  requiresAuth: boolean;
}

export type BuildResult =
  | { ok: true; payload: CustomProviderPayload }
  | { ok: false; error: string };

export function emptyCustomProviderForm(): CustomProviderForm {
  return {
    displayName: '',
    engine: 'openai_compatible',
    apiUrl: '',
    apiKey: '',
    models: '',
    requiresAuth: true,
  };
}

/** Split the free-text model field into a clean, de-duplicated list. Accepts
 *  commas and newlines as separators, trims each entry, and drops blanks —
 *  order is preserved (first occurrence wins). */
export function parseModels(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of raw.split(/[\n,]/)) {
    const model = part.trim();
    if (model && !seen.has(model)) {
      seen.add(model);
      out.push(model);
    }
  }
  return out;
}

function isValidHttpUrl(value: string): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  return url.protocol === 'http:' || url.protocol === 'https:';
}

/** Validate the form and, on success, produce the exact wire payload for
 *  POST /config/custom-providers. Returns the first validation error otherwise
 *  so the modal can surface a single, actionable message. */
export function buildCustomProviderPayload(form: CustomProviderForm): BuildResult {
  const displayName = form.displayName.trim();
  if (!displayName) return { ok: false, error: 'Display name is required.' };

  const apiUrl = form.apiUrl.trim();
  if (!apiUrl) return { ok: false, error: 'An API URL is required.' };
  if (!isValidHttpUrl(apiUrl)) {
    return { ok: false, error: 'Enter a valid API URL starting with http:// or https://.' };
  }

  const models = parseModels(form.models);
  if (models.length === 0) return { ok: false, error: 'Add at least one model id.' };

  const apiKey = form.apiKey.trim();
  if (form.requiresAuth && !apiKey) {
    return { ok: false, error: 'An API key is required, or turn off "Requires API key".' };
  }

  return {
    ok: true,
    payload: {
      engine: form.engine,
      display_name: displayName,
      api_url: apiUrl,
      api_key: apiKey,
      models,
      requires_auth: form.requiresAuth,
    },
  };
}
