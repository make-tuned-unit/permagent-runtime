// Web-search providers (v1). Single source of truth for the two MCP search
// connectors and their key-setup metadata — consumed by the wizard step, the
// Settings "Search & tools" section, and the agent-guided setup skill.
//
// Transport: both are Stdio MCP servers. The shipped .app has no system Node/npx
// on PATH, so it CANNOT run them via `npx` (#358). Instead the build bundles a
// Node runtime + the pre-installed server packages under the app's resources
// (scripts/copy-mcp-runtime.sh → Contents/Resources/mcp-runtime), and we launch
// each server by invoking the bundled `node` directly against the package's
// entry JS — no registry fetch and no PATH dependency. In browser dev (no Tauri
// bundle) we fall back to `npx`, which works because the dev box has Node.
//
// The API key is stored as a secret via /config/upsert (keychain) under
// `keyName`, and the MCP entry reads it back through `env_keys` — the key never
// lives in the extension config.
//
// v1 = the agent auto-selects per query from tool name+description (no routing
// layer, no usage tracking). Tier-aware routing is a deferred v2.

import { api, type ExtensionQuery } from './api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Subdir of Contents/Resources holding the bundled Node + server packages. */
const MCP_RUNTIME_RESOURCE = 'mcp-runtime';

export interface SearchProvider {
  /** Stable id used as the extension key + UI key. */
  id: string;
  /** Display name (also the extension `name`). */
  displayName: string;
  /** One-line description shown in Settings and used as the MCP description. */
  description: string;
  /** Secret config key — stored in keychain, read back via env_keys. */
  keyName: string;
  /** Provider's API-key page — opened in the in-app browser during setup. */
  keyPageUrl: string;
  /** Short label for the key-page link/button. */
  keyPageLabel: string;
  /** Entry JS relative to the bundled mcp-runtime dir (direct-node launch). */
  entryPath: string;
  /** npx args used only in browser dev (no bundled runtime). */
  npxArgs: string[];
}

export const SEARCH_PROVIDERS: SearchProvider[] = [
  {
    id: 'brave-search',
    displayName: 'Brave Search',
    description: 'Web + local search via the Brave Search API.',
    keyName: 'BRAVE_API_KEY',
    keyPageUrl: 'https://api-dashboard.search.brave.com/app/keys',
    keyPageLabel: 'Brave Search API keys',
    entryPath: 'node_modules/@modelcontextprotocol/server-brave-search/dist/index.js',
    npxArgs: ['-y', '@modelcontextprotocol/server-brave-search'],
  },
  {
    id: 'tavily',
    displayName: 'Tavily Web Search',
    description: 'AI-optimized web search and content extraction via Tavily.',
    keyName: 'TAVILY_API_KEY',
    keyPageUrl: 'https://app.tavily.com/',
    keyPageLabel: 'Tavily API keys',
    entryPath: 'node_modules/tavily-mcp/build/index.js',
    npxArgs: ['-y', 'tavily-mcp'],
  },
];

export function getSearchProvider(id: string): SearchProvider | undefined {
  return SEARCH_PROVIDERS.find(p => p.id === id);
}

/**
 * Resolve how to launch a provider's MCP server: the bundled Node runtime in
 * the shipped app (cmd = absolute bundled node, args = absolute entry JS), or
 * `npx` in browser dev. Falls back to npx if the bundled runtime can't be
 * resolved for any reason.
 */
async function resolveLaunch(p: SearchProvider): Promise<{ cmd: string; args: string[] }> {
  if (isTauri) {
    try {
      const { resolveResource, join } = await import('@tauri-apps/api/path');
      const runtimeDir = await resolveResource(MCP_RUNTIME_RESOURCE);
      const node = await join(runtimeDir, 'node');
      const entry = await join(runtimeDir, p.entryPath);
      return { cmd: node, args: [entry] };
    } catch {
      // fall through to npx
    }
  }
  return { cmd: 'npx', args: p.npxArgs };
}

/**
 * Build the ExtensionQuery for a provider's MCP connector. Keyed to the keychain
 * secret via env_keys — so the entry is inert until the user adds the key, and
 * outbound only happens once enabled + key present (the v1 egress opt-in:
 * enable-flag + key presence, surfaced in Settings → Tools & MCPs).
 */
export async function buildSearchExtensionQuery(
  p: SearchProvider,
  enabled = true,
): Promise<ExtensionQuery> {
  const { cmd, args } = await resolveLaunch(p);
  return {
    name: p.displayName,
    enabled,
    config: {
      type: 'stdio',
      name: p.displayName,
      description: p.description,
      cmd,
      args,
      env_keys: [p.keyName],
      timeout: 300,
    },
  };
}

/**
 * Store a provider's API key as a keychain secret and register+enable its MCP
 * connector (which reads the key back via env_keys). Shared by the wizard step
 * and the Settings "Search & tools" section so the two never diverge.
 */
export async function saveAndEnableSearchProvider(p: SearchProvider, key: string): Promise<void> {
  await api.upsertConfig(p.keyName, key.trim(), true);
  await api.addExtension(await buildSearchExtensionQuery(p, true));
}
