/**
 * The Financier tab is gated on the `finance` platform extension — the same
 * bit Settings → Agents, Settings → Features, and a new session all read.
 *
 * Until that bit is on, the tab must not appear in the sidebar, must not
 * render, and must not be a navigation target. Showing it while the capability
 * is off would be a surface that looks live and does nothing; hiding it is
 * the honest state.
 *
 * Kept free of React so the rule is unit-testable without mounting the app.
 */

export const FINANCE_EXTENSION_KEY = 'finance';
export const FINANCIER_TOOL = 'financier';

/** True when a workspace layout tree hosts the Financier panel. */
export function layoutHostsFinancier(node: unknown): boolean {
  if (!node || typeof node !== 'object') return false;
  const n = node as { type?: string; tool?: string; children?: unknown[] };
  if (n.type === 'panel') return n.tool === FINANCIER_TOOL;
  if (n.type === 'split' && Array.isArray(n.children)) {
    return n.children.some(layoutHostsFinancier);
  }
  return false;
}

/**
 * The tab is shown only on an explicit yes. `null` (not yet read) and `false`
 * both hide it — fail closed, so a money tab never appears because we have
 * not asked yet.
 */
export function financierTabIsVisible(financeEnabled: boolean | null): boolean {
  return financeEnabled === true;
}

/** Workspaces the sidebar / renderer may show, given the finance bit. */
export function visibleWorkspaces<T extends { layoutJson: unknown }>(
  workspaces: T[],
  financeEnabled: boolean | null,
): T[] {
  if (financierTabIsVisible(financeEnabled)) return workspaces;
  return workspaces.filter(w => !layoutHostsFinancier(w.layoutJson));
}

/** First non-Financier workspace to land on when the tab is hidden. */
export function fallbackWorkspaceId<T extends { id: string; isDefault?: boolean; layoutJson: unknown }>(
  workspaces: T[],
): string | undefined {
  const home = workspaces.find(w => w.isDefault && !layoutHostsFinancier(w.layoutJson));
  if (home) return home.id;
  return workspaces.find(w => !layoutHostsFinancier(w.layoutJson))?.id;
}
