/**
 * Which providers first-run setup can connect, derived from the daemon's
 * provider registry rather than a hand-kept list.
 *
 * The wizard offers the providers it can finish in one screen: one secret API
 * key, every other required setting already defaulted, no sign-in flow.
 * Everything else (OAuth sign-ins, CLI-backed agents, providers that need an
 * endpoint or a project id, custom endpoints) is set up in Settings, and the
 * step says so. No provider is pre-selected and none is recommended.
 */
import type { api } from '../../lib/api';

export type ProviderEntry = Awaited<ReturnType<typeof api.getProviders>>[number];

/** The provider id the daemon's local-model routes (/api/ollama/*) serve. */
export const LOCAL_RUNTIME_PROVIDER = 'ollama';

export interface KeyProvider {
  name: string;
  displayName: string;
  /** Config key the API key is stored under. */
  secretKey: string;
  defaultModel: string;
  setupSteps: string[];
  isConfigured: boolean;
  isDefault: boolean;
}

export function keyProviders(list: ProviderEntry[]): KeyProvider[] {
  const out: KeyProvider[] = [];
  for (const p of list) {
    if (p.name === LOCAL_RUNTIME_PROVIDER) continue;
    const keys = p.metadata?.config_keys ?? [];
    if (keys.some(k => k.oauth_flow)) continue;
    const secrets = keys.filter(k => k.secret);
    if (secrets.length !== 1) continue;
    const secret = secrets[0];
    const unmet = keys.some(k => k !== secret && k.required && (k.default === undefined || k.default === null || k.default === ''));
    if (unmet) continue;
    out.push({
      name: p.name,
      displayName: p.metadata.display_name || p.name,
      secretKey: secret.name,
      defaultModel: p.metadata.default_model || '',
      setupSteps: p.metadata.setup_steps ?? [],
      isConfigured: p.is_configured,
      isDefault: !!p.is_default,
    });
  }
  return out.sort((a, b) => a.displayName.localeCompare(b.displayName));
}

export function hasLocalRuntime(list: ProviderEntry[]): boolean {
  return list.some(p => p.name === LOCAL_RUNTIME_PROVIDER);
}

const URL_RE = /(https?:\/\/[^\s)'"]+)/;

/** Split a setup step into text and (at most one) link, for rendering. */
export function splitStepLink(step: string): { before: string; url: string | null; after: string } {
  const m = URL_RE.exec(step);
  if (!m) return { before: step, url: null, after: '' };
  return { before: step.slice(0, m.index), url: m[1], after: step.slice(m.index + m[1].length) };
}
