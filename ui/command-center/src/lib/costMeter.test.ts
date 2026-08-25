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
import { costFromFrame, formatCostMeter, fmtUsd, fmtTokens, type CodingSpend } from './costMeter';

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

describe('formatCostMeter', () => {
  it('renders idle state when there is no live token state', () => {
    const m = formatCostMeter(null);
    expect(m.cost).toBe('$0.00');
    expect(m.segments).toEqual([]);
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
