import { describe, expect, it } from 'vitest';
import {
  FINANCE_EXTENSION_KEY,
  FINANCIER_TOOL,
  fallbackWorkspaceId,
  financierTabIsVisible,
  layoutHostsFinancier,
  visibleWorkspaces,
} from './financeGate';

const HOME = { id: 'home', isDefault: true, layoutJson: { type: 'panel', tool: 'dashboard' } };
const FINANCIER = { id: 'fin', isDefault: false, layoutJson: { type: 'panel', tool: FINANCIER_TOOL } };
const BUILD = { id: 'build', isDefault: false, layoutJson: { type: 'panel', tool: 'build' } };

describe('financeGate — the Financier tab is hidden until finance is on', () => {
  it('the gate key is the finance extension, not a second boolean', () => {
    expect(FINANCE_EXTENSION_KEY).toBe('finance');
    expect(FINANCE_EXTENSION_KEY).not.toBe('financier_enabled');
  });

  it('recognises a Financier workspace, including inside a split', () => {
    expect(layoutHostsFinancier(FINANCIER.layoutJson)).toBe(true);
    expect(layoutHostsFinancier(HOME.layoutJson)).toBe(false);
    expect(layoutHostsFinancier({
      type: 'split',
      children: [HOME.layoutJson, FINANCIER.layoutJson],
    })).toBe(true);
  });

  it('hides the tab when finance is off, and while the bit is still unread', () => {
    // REGRESSION: a money tab that renders while the capability is off is a
    // surface that looks live and is not. Fail closed on `null` too — not-yet-
    // asked is not "on".
    expect(financierTabIsVisible(false)).toBe(false);
    expect(financierTabIsVisible(null)).toBe(false);
    expect(financierTabIsVisible(true)).toBe(true);

    expect(visibleWorkspaces([HOME, FINANCIER, BUILD], false).map(w => w.id))
      .toEqual(['home', 'build']);
    expect(visibleWorkspaces([HOME, FINANCIER, BUILD], null).map(w => w.id))
      .toEqual(['home', 'build']);
    expect(visibleWorkspaces([HOME, FINANCIER, BUILD], true).map(w => w.id))
      .toEqual(['home', 'fin', 'build']);
  });

  it('lands on Home, not on a hidden Financier, when the tab is taken away', () => {
    expect(fallbackWorkspaceId([HOME, FINANCIER, BUILD])).toBe('home');
  });
});
