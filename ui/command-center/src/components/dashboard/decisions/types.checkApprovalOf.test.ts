/**
 * checkApprovalOf — the seam that turns a plain `choice` decision into a
 * Tier-2 command-approval card (verification approval ladder). Pinned here:
 * kind/proposal gating and payload tolerance (untrusted, S2) so ordinary
 * choice cards are never mistaken for a command approval.
 */

import { describe, expect, it } from 'vitest';

import { checkApprovalOf } from './types';
import type { Decision } from './types';

function decision(overrides: Partial<Decision>): Decision {
  return {
    id: 'd-1',
    kind: 'choice',
    goal_id: null,
    project_id: null,
    tier: 2,
    headline: 'Choose an option',
    detail: '',
    payload: null,
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    created_at: '2026-08-24T00:00:00Z',
    resolved_at: null,
    ...overrides,
  };
}

const checkApprovalPayload = {
  question: 'Run `cargo xtask verify` for this check?',
  options: [
    { id: 'approve-once', label: 'Approve once' },
    { id: 'approve-and-allowlist', label: 'Approve and allowlist "cargo"' },
    { id: 'deny', label: 'Deny' },
  ],
  default: null,
  proposal: 'check_approval',
  check_approval: {
    command: 'cargo xtask verify',
    cwd: '/Users/j/Documents/dev/permagent-runtime',
    first_token: 'cargo',
    reason: 'Shell commands in verification checks require approval.',
    tier: 'user',
    project_id: 'proj-1',
  },
};

describe('checkApprovalOf', () => {
  it('returns the check_approval object for a check_approval choice decision', () => {
    const result = checkApprovalOf(decision({ payload: checkApprovalPayload }));
    expect(result).not.toBeNull();
    expect(result?.command).toBe('cargo xtask verify');
    expect(result?.cwd).toBe('/Users/j/Documents/dev/permagent-runtime');
    expect(result?.first_token).toBe('cargo');
  });

  it('returns null for an unrelated choice decision — ordinary choice cards are unaffected', () => {
    const ordinary = decision({
      payload: {
        question: 'Pick a plan',
        options: [{ id: 'a', label: 'Plan A' }, { id: 'b', label: 'Plan B' }],
        default: 'a',
      },
    });
    expect(checkApprovalOf(ordinary)).toBeNull();
  });

  it('returns null for a non-choice decision even carrying a check_approval-shaped payload', () => {
    expect(
      checkApprovalOf(decision({ kind: 'approve_review', payload: checkApprovalPayload })),
    ).toBeNull();
  });

  it('tolerates missing or malformed payloads (untrusted input)', () => {
    expect(checkApprovalOf(decision({ payload: null }))).toBeNull();
    expect(checkApprovalOf(decision({ payload: {} }))).toBeNull();
    expect(
      checkApprovalOf(decision({ payload: { proposal: 'check_approval' } })),
    ).toBeNull();
    expect(
      checkApprovalOf(decision({
        payload: { proposal: 'check_approval', check_approval: { cwd: '/x' } },
      })),
    ).toBeNull();
    expect(
      checkApprovalOf(decision({
        payload: { proposal: 'other_thing', check_approval: checkApprovalPayload.check_approval },
      })),
    ).toBeNull();
  });
});
