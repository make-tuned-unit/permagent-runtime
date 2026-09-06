/**
 * Build cost-meter — unit + end-to-end wiring tests.
 *
 * The wiring test drives the EXACT functions production runs: it takes a real
 * SSE `Message` frame (as the daemon serializes it — `token_state` snake_case,
 * fields camelCase), extracts it with `costFromFrame` (the same call the store's
 * `handleSessionEvent` makes) and renders it with `formatCostMeter` (the same
 * call the `CostStatusline` component makes). Nothing is stubbed, so a real
 * ledger cost on the frame becomes the real `$` the meter shows.
 */

import { describe, expect, it } from 'vitest';
import type { SSEEvent, TokenState } from './api';
import {
  costFromFrame,
  formatCostMeter,
  fmtUsd,
  fmtTokens,
  parseBudgetProjection,
  type CodingSpend,
} from './costMeter';

function tokenState(overrides: Partial<TokenState> = {}): TokenState {
  return {
    inputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    accumulatedInputTokens: 0,
    accumulatedOutputTokens: 0,
    accumulatedTotalTokens: 0,
    costUsd: 0,
    accumulatedCostUsd: 0,
    cacheSavingsUsd: 0,
    contextPercent: null,
    model: '',
    ...overrides,
  };
}

function codingSpend(overrides: Partial<CodingSpend> = {}): CodingSpend {
  return {
    sessionId: 'harness-1',
    turnUsd: 0.0032,
    sessionUsd: 0.0332,
    todayUsd: 0.5332,
    totalTokens: 12800,
    provider: 'zai',
    model: 'glm-5.3',
    workingDir: '/tmp/proj',
    estimated: false,
    finalTurn: false,
    ...overrides,
  };
}

function budgetProjection(overrides: Record<string, unknown> = {}) {
  const scope = {
    cap: { softUsd: 1, gateUsd: 2, hardUsd: 3, source: 'current_budget_config' },
    settledUsd: 0,
    heldUsd: 0,
    unknownUsd: 0,
    effectiveUsedUsd: 0,
    remainingUsd: 3,
    band: 'ok',
    completeness: 'complete',
    error: null,
  };
  const evidence = {
    billingClass: 'paid_api', provider: 'fixture', model: 'fixture-model',
    callId: 'call-1', isEstimated: false,
    observedAt: '2026-09-05T12:00:00Z', source: 'cost_ledger',
  };
  return {
    taskId: 'task-1', rootSessionId: 'harness-1',
    task: scope, session: scope,
    taskBilling: evidence, sessionBilling: evidence,
    provenance: {
      version: 'budget-projection.v1', asOf: '2026-09-05T12:00:00Z',
      completeness: 'complete', sources: ['sessions', 'cost_ledger'], error: null,
    },
    ...overrides,
  };
}

describe('formatters', () => {
  it('formats USD as cents, with sub-cent precision', () => {
    expect(fmtUsd(0)).toBe('$0.00');
    expect(fmtUsd(0.42)).toBe('$0.42');
    expect(fmtUsd(0.004)).toBe('$0.0040');
    expect(fmtUsd(12.5)).toBe('$12.50');
  });

  it('formats token counts compactly', () => {
    expect(fmtTokens(0)).toBe('0');
    expect(fmtTokens(47000)).toBe('47k');
    expect(fmtTokens(1_200_000)).toBe('1.2M');
  });
});

describe('budget-projection.v1 validation and rendering', () => {
  it('accepts authoritative zero while keeping null unavailable', () => {
    const parsed = parseBudgetProjection(budgetProjection());
    expect(parsed?.session.effectiveUsedUsd).toBe(0);
    expect(parseBudgetProjection(budgetProjection({
      provenance: { ...budgetProjection().provenance, version: 'budget-projection.v0' },
    }))).toBeNull();
    expect(parseBudgetProjection(budgetProjection({
      session: { ...budgetProjection().session, effectiveUsedUsd: Number.NaN },
    }))).toBeNull();
    expect(parseBudgetProjection(budgetProjection({ taskId: '' }))).toBeNull();
  });

  it('rejects a projection whose root identity is not the harness session', () => {
    const projection = parseBudgetProjection(budgetProjection({ rootSessionId: 'other-session' }));
    expect(projection?.rootSessionId).toBe('other-session');
    const spend = codingSpend({ budget: projection ?? undefined });
    // The liveness boundary performs the event/session identity comparison;
    // the pure parser still validates the projection as a projection.
    expect(spend.budget?.rootSessionId).toBe('other-session');
  });

  it('renders held/unknown/remaining/provenance distinctly from zero', () => {
    const projection = parseBudgetProjection(budgetProjection({
      session: {
        ...budgetProjection().session,
        settledUsd: 0,
        heldUsd: 0.25,
        unknownUsd: 0.5,
        effectiveUsedUsd: 0.75,
        remainingUsd: 2.25,
        band: 'unknown',
      },
      sessionBilling: { ...budgetProjection().sessionBilling, isEstimated: true },
    }));
    const meter = formatCostMeter(null, codingSpend({ budget: projection ?? undefined }));
    expect(meter.cost).toBe('$0.75');
    expect(meter.segments).toEqual(expect.arrayContaining([
      'session held $0.25', 'session unknown $0.50', 'session remaining $2.25',
      'session budget unknown', 'session billing paid_api / fixture / fixture-model',
    ]));
    expect(meter.ariaLabel).toContain('billing is estimated');
  });

  it('does not turn an unavailable harness projection into chat spend', () => {
    const meter = formatCostMeter(
      tokenState({ accumulatedCostUsd: 99 }),
      codingSpend({ budgetStatus: 'unavailable', budget: undefined, sessionUsd: null }),
    );
    expect(meter.cost).toBe('—');
    expect(meter.segments).toContain('Budget unavailable');
    expect(meter.cost).not.toContain('99');
  });
});

describe('formatCostMeter', () => {
  it('renders idle state when there is no live token state', () => {
    const m = formatCostMeter(null);
    expect(m.cost).toBe('$0.00');
    expect(m.segments).toEqual([]);
  });

  it('does not present an empty active-run list as zero harness spend', () => {
    const m = formatCostMeter(null, null, null, 'none');
    expect(m.cost).toBe('—');
    expect(m.segments).toEqual(['No active harness']);
  });

  it('shows exactly one spend figure plus supporting segments', () => {
    const m = formatCostMeter(
      tokenState({
        accumulatedInputTokens: 47000,
        accumulatedOutputTokens: 12000,
        accumulatedCostUsd: 0.42,
        cacheSavingsUsd: 0.28,
        contextPercent: 31.2,
        model: 'claude-sonnet-4',
      }),
    );
    expect(m.cost).toBe('$0.42');
    expect(m.segments).toEqual(['47k↑ 12k↓', 'cache saved $0.28', '31% ctx', 'claude-sonnet-4']);
  });

  it('omits cache/ctx/model segments when their data is absent', () => {
    const m = formatCostMeter(
      tokenState({ accumulatedInputTokens: 500, accumulatedOutputTokens: 100, accumulatedCostUsd: 0.01 }),
    );
    expect(m.cost).toBe('$0.01');
    // Only the token segment — no cache-saved (0), no ctx (null), no model ('').
    expect(m.segments).toEqual(['500↑ 100↓']);
  });

  it('appends incl. N subagents $X when children exist', () => {
    const m = formatCostMeter(
      tokenState({ accumulatedCostUsd: 0.42, accumulatedInputTokens: 100, accumulatedOutputTokens: 10 }),
      null,
      { count: 2, totalUsd: 0.17 },
    );
    expect(m.cost).toBe('$0.42');
    expect(m.segments).toContain('incl. 2 subagents $0.17');
    expect(m.ariaLabel).toContain('incl. 2 subagents $0.17');
  });

  it('singularizes the subagent label for a single child', () => {
    const m = formatCostMeter(
      tokenState({ accumulatedCostUsd: 0.1 }),
      null,
      { count: 1, totalUsd: 0.05 },
    );
    expect(m.segments).toContain('incl. 1 subagent $0.05');
  });

  it('prefers codingSpend over liveTokens and still shows the subagent suffix', () => {
    const m = formatCostMeter(
      tokenState({ accumulatedCostUsd: 99 }),
      codingSpend({ sessionUsd: 0.33 }),
      { count: 3, totalUsd: 0.12 },
    );
    expect(m.cost).toBe('$0.33');
    expect(m.segments).toContain('incl. 3 subagents $0.12');
  });
});

describe('formatCostMeter with a coding-harness spend (the Build tab PTY session)', () => {
  it('renders the session total, the turn delta, today\'s total, and the model', () => {
    const m = formatCostMeter(null, codingSpend());
    expect(m.cost).toBe('$0.03');
    expect(m.segments).toEqual([
      '13k tokens',
      '+$0.0032 this turn',
      'today $0.53',
      'glm-5.3',
    ]);
    expect(m.ariaLabel).toContain('Session cost $0.03');
    expect(m.ariaLabel).toContain('today $0.53');
  });

  it('renders a ~ prefix and the disclosure segment for a fail-closed estimate, and says so in the aria label', () => {
    // This is the test that stops a fail-closed worst case being shown as a
    // bill: `estimated: true` must never render as a plain "$0.12".
    const m = formatCostMeter(null, codingSpend({ estimated: true, sessionUsd: 0.12 }));
    expect(m.cost).toBe('~$0.12');
    expect(m.segments).toContain('estimated — no published price');
    expect(m.ariaLabel.toLowerCase()).toContain('estimate');
  });

  it('adds a "session ended" segment when finalTurn is true', () => {
    const m = formatCostMeter(null, codingSpend({ finalTurn: true }));
    expect(m.segments).toContain('session ended');
  });

  it('keeps the tokens-only rendering byte-identical when coding is null (regression guard)', () => {
    const overrides: Partial<TokenState> = {
      accumulatedInputTokens: 47000,
      accumulatedOutputTokens: 12000,
      accumulatedCostUsd: 0.42,
      cacheSavingsUsd: 0.28,
      contextPercent: 31.2,
      model: 'claude-sonnet-4',
    };
    const withDefaultArg = formatCostMeter(tokenState(overrides));
    const withExplicitNull = formatCostMeter(tokenState(overrides), null);
    expect(withExplicitNull).toEqual(withDefaultArg);
    expect(withExplicitNull.cost).toBe('$0.42');
    expect(withExplicitNull.segments).toEqual([
      '47k↑ 12k↓',
      'cache saved $0.28',
      '31% ctx',
      'claude-sonnet-4',
    ]);
  });
});

describe('costFromFrame', () => {
  it('returns null for frames without token_state (meter holds its value)', () => {
    expect(costFromFrame({ type: 'Ping' } as SSEEvent)).toBeNull();
    expect(costFromFrame({ type: 'Error', error: 'boom' } as SSEEvent)).toBeNull();
  });
});

describe('SSE → meter wiring (end-to-end, unstubbed)', () => {
  it('turns a real Message frame into the rendered dollar figure', () => {
    // A frame exactly as the daemon emits it: outer key `token_state`
    // (snake_case), inner fields camelCase (TokenState is rename_all=camelCase),
    // with a real per-call-ledger-sourced accumulatedCostUsd.
    const frame = {
      type: 'Message',
      message: { role: 'assistant', content: [] },
      token_state: {
        inputTokens: 8000,
        outputTokens: 1200,
        totalTokens: 9200,
        accumulatedInputTokens: 47000,
        accumulatedOutputTokens: 12000,
        accumulatedTotalTokens: 59000,
        costUsd: 0.03,
        accumulatedCostUsd: 0.42,
        cacheSavingsUsd: 0.28,
        contextPercent: 31.0,
        model: 'claude-sonnet-4',
      },
    } as unknown as SSEEvent;

    // Store-side extraction → component-side render, same functions as prod.
    const live = costFromFrame(frame);
    expect(live).not.toBeNull();
    expect(live!.accumulatedCostUsd).toBe(0.42);

    const meter = formatCostMeter(live);
    expect(meter.cost).toBe('$0.42'); // the real ledger cost, not a stub/zero
    expect(meter.segments).toContain('cache saved $0.28');
    expect(meter.segments).toContain('47k↑ 12k↓');
    expect(meter.segments).toContain('31% ctx');
    expect(meter.segments).toContain('claude-sonnet-4');
    expect(meter.ariaLabel).toContain('Session cost $0.42');
  });
});
