/** @vitest-environment jsdom */

/**
 * Settings → Models → Routing: the per-role routing surface over
 * GET/PUT/DELETE /api/cost-router/roles. Verifies:
 *   1. every role row renders with its label + the right effective badge
 *      (configured / derived / session model (no fit)), fit indicator, warnings;
 *   2. Override → Save calls PUT with the right role + body and swaps in the row;
 *   3. Clear calls DELETE with the right role;
 *   4. the KB-stale note appears only when the daemon says the snapshot is stale.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getRoutingRoles: vi.fn(),
    setRoutingRole: vi.fn(),
    clearRoutingRole: vi.fn(),
  },
  apiFetch: vi.fn(),
}));

import { api, type RoutingRoleRow, type RoutingRolesResponse } from '../../lib/api';
import { RoutingSection, effectiveModel } from './RoutingSection';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getRoutingRoles = vi.mocked(api.getRoutingRoles);
const setRoutingRole = vi.mocked(api.setRoutingRole);
const clearRoutingRole = vi.mocked(api.clearRoutingRole);
let container: HTMLDivElement;
let root: Root;

const row = (over: Partial<RoutingRoleRow> & Pick<RoutingRoleRow, 'role' | 'label'>): RoutingRoleRow => ({
  description: `${over.label} work`,
  configured: null,
  recommended: null,
  floor_met: true,
  warnings: [],
  reason: '',
  ...over,
});

const fixture = (stale = false): RoutingRolesResponse => ({
  roles: [
    row({ role: 'orchestrate', label: 'Orchestrate', recommended: { provider: 'anthropic', model: 'claude-opus-4-1' } }),
    row({
      role: 'edit', label: 'Edit',
      configured: { provider: 'openai', model: 'gpt-5.6' },
      recommended: { provider: 'anthropic', model: 'claude-sonnet-5' },
    }),
    row({
      role: 'mechanical', label: 'Mechanical',
      recommended: { provider: 'ollama', model: 'qwen3-coder:30b' },
      confidence: 'family_estimate',
      floor_met: false,
      warnings: ['below the capability floor — best available'],
    }),
    row({ role: 'review', label: 'Review' }),
    row({ role: 'local', label: 'Local' }),
  ],
  kb: { snapshot_date: '2026-07-15', stale },
  discovered: { providers: ['anthropic', 'ollama'], local_models: ['qwen3-coder:30b'] },
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getRoutingRoles.mockReset().mockResolvedValue(fixture());
  setRoutingRole.mockReset();
  clearRoutingRole.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => { root.render(<RoutingSection />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

const byId = (id: string) => container.querySelector(`[data-testid="${id}"]`) as HTMLElement | null;

describe('effectiveModel', () => {
  it('prefers configured, then derived, then session', () => {
    expect(effectiveModel(row({ role: 'edit', label: 'Edit', configured: { provider: 'a', model: 'b' }, recommended: { provider: 'c', model: 'd' } })))
      .toEqual({ kind: 'configured', model: { provider: 'a', model: 'b' } });
    expect(effectiveModel(row({ role: 'edit', label: 'Edit', recommended: { provider: 'c', model: 'd' } })))
      .toEqual({ kind: 'derived', model: { provider: 'c', model: 'd' } });
    expect(effectiveModel(row({ role: 'edit', label: 'Edit' }))).toEqual({ kind: 'session', model: null });
  });
});

describe('RoutingSection', () => {
  it('renders every role with its effective badge, fit, and warnings', async () => {
    await mount();
    expect(getRoutingRoles).toHaveBeenCalledTimes(1);
    for (const role of ['orchestrate', 'edit', 'mechanical', 'review', 'local']) {
      expect(byId(`routing-row-${role}`)).toBeTruthy();
    }
    // Badges: derived (orchestrate, mechanical), configured (edit), session (review, local).
    expect(byId('routing-row-orchestrate')!.querySelector('[data-testid="routing-badge-derived"]')).toBeTruthy();
    expect(byId('routing-row-edit')!.querySelector('[data-testid="routing-badge-configured"]')).toBeTruthy();
    expect(byId('routing-row-review')!.querySelector('[data-testid="routing-badge-session"]')?.textContent).toBe('session model (no fit)');
    expect(byId('routing-row-local')!.querySelector('[data-testid="routing-badge-session"]')).toBeTruthy();

    // The configured row shows the hand-set model AND what the router would have derived.
    expect(byId('routing-row-edit')!.textContent).toContain('openai/gpt-5.6');
    expect(byId('routing-row-edit')!.textContent).toContain('derived would be anthropic/claude-sonnet-5');

    // Under-fit + warning + family-estimate label on mechanical.
    expect(byId('routing-fit-mechanical')?.textContent).toBe('under-fit');
    expect(byId('routing-fit-orchestrate')?.textContent).toBe('fits');
    expect(byId('routing-row-mechanical')!.textContent).toContain('below the capability floor');
    expect(byId('routing-row-mechanical')!.textContent).toContain('family estimate');

    // Session rows have no fit indicator (nothing was picked to judge).
    expect(byId('routing-fit-review')).toBeNull();

    // Honest copy + discovery footer.
    expect(container.textContent).toContain('Hand-set roles win.');
    expect(container.textContent).toContain('Discovered providers: anthropic, ollama');
    // Not stale → no note.
    expect(byId('routing-kb-stale')).toBeNull();
  });

  it('Override → Save calls PUT for the right role, prefilled from the recommendation', async () => {
    setRoutingRole.mockResolvedValue(
      row({ role: 'orchestrate', label: 'Orchestrate', configured: { provider: 'anthropic', model: 'claude-opus-4-1' }, recommended: { provider: 'anthropic', model: 'claude-opus-4-1' } }),
    );
    await mount();
    await act(async () => { byId('routing-override-orchestrate')!.click(); });
    const providerInput = byId('routing-provider-orchestrate')!.querySelector('input') as HTMLInputElement;
    const modelInput = byId('routing-model-orchestrate')!.querySelector('input') as HTMLInputElement;
    expect(providerInput.value).toBe('anthropic');
    expect(modelInput.value).toBe('claude-opus-4-1');

    await act(async () => { byId('routing-save-orchestrate')!.click(); });
    await act(async () => { await Promise.resolve(); });
    expect(setRoutingRole).toHaveBeenCalledWith('orchestrate', { provider: 'anthropic', model: 'claude-opus-4-1' });
    // The returned row replaces the old one: badge flips to configured.
    expect(byId('routing-row-orchestrate')!.querySelector('[data-testid="routing-badge-configured"]')).toBeTruthy();
  });

  it('Clear calls DELETE for the right role and drops back to derived', async () => {
    clearRoutingRole.mockResolvedValue(
      row({ role: 'edit', label: 'Edit', recommended: { provider: 'anthropic', model: 'claude-sonnet-5' } }),
    );
    await mount();
    // Only the configured row offers Clear.
    expect(byId('routing-clear-orchestrate')).toBeNull();
    await act(async () => { byId('routing-clear-edit')!.click(); });
    await act(async () => { await Promise.resolve(); });
    expect(clearRoutingRole).toHaveBeenCalledWith('edit');
    expect(setRoutingRole).not.toHaveBeenCalled();
    expect(byId('routing-row-edit')!.querySelector('[data-testid="routing-badge-derived"]')).toBeTruthy();
    expect(byId('routing-row-edit')!.textContent).toContain('anthropic/claude-sonnet-5');
  });

  it('shows the KB-stale note when the daemon flags the snapshot', async () => {
    getRoutingRoles.mockResolvedValue(fixture(true));
    await mount();
    expect(byId('routing-kb-stale')?.textContent).toContain('2026-07-15');
  });

  it('surfaces a load failure instead of an empty section', async () => {
    getRoutingRoles.mockRejectedValue(new Error('daemon down'));
    await mount();
    expect(container.textContent).toContain("Couldn't load routing: daemon down");
  });
});
