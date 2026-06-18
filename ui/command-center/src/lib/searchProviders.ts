// Web-search providers (v1). Single source of truth for the two MCP search
// connectors and their key-setup metadata — consumed by the wizard step, the
// Settings "Search & tools" section, and the agent-guided setup skill.
//
// Transport: both ship as the goose-supported Stdio (npx) MCP servers, matching
// documentation/docs/mcp/{brave,tavily}-mcp.md. The API key is stored as a
// secret via /config/upsert (keychain) under `keyName`, and the MCP entry reads
// it back through `env_keys` — the key never lives in the extension config.
//
// v1 = the agent auto-selects per query from tool name+description (no routing
// layer, no usage tracking). Tier-aware routing is a deferred v2.

import { api, type ExtensionQuery } from './api';

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
}

export const SEARCH_PROVIDERS: SearchProvider[] = [
  {
    id: 'brave-search',
    displayName: 'Brave Search',
    description: 'Web + local search via the Brave Search API.',
    keyName: 'BRAVE_API_KEY',
    keyPageUrl: 'https://api-dashboard.search.brave.com/app/keys',
    keyPageLabel: 'Brave Search API keys',
  },
  {
    id: 'tavily',
    displayName: 'Tavily Web Search',
    description: 'AI-optimized web search and content extraction via Tavily.',
    keyName: 'TAVILY_API_KEY',
    keyPageUrl: 'https://app.tavily.com/',
    keyPageLabel: 'Tavily API keys',
  },
];

export function getSearchProvider(id: string): SearchProvider | undefined {
  return SEARCH_PROVIDERS.find(p => p.id === id);
}

/**
 * Build the ExtensionQuery for a provider's MCP connector. Stdio/npx, keyed to
 * the keychain secret via env_keys — so the entry is inert until the user adds
 * the key, and outbound only happens once enabled + key present (the v1 egress
 * opt-in: enable-flag + key presence, surfaced in Settings → Tools & MCPs).
 */
export function buildSearchExtensionQuery(p: SearchProvider, enabled = true): ExtensionQuery {
  const args =
    p.id === 'brave-search'
      ? ['-y', '@modelcontextprotocol/server-brave-search']
      : ['-y', 'tavily-mcp'];
  return {
    name: p.displayName,
    enabled,
    config: {
      type: 'stdio',
      name: p.displayName,
      description: p.description,
      cmd: 'npx',
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
  await api.addExtension(buildSearchExtensionQuery(p, true));
}
