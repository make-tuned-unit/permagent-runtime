/**
 * Decision Inbox — DecisionItem pure logic (tool_approval full-arguments view).
 *
 * `toolArgumentsText` is the informed-consent seam: the card's `detail` line
 * holds a clipped preview (tool_execution.rs marks clipping explicitly), and
 * this helper decides whether the FULL payload arguments are offered for
 * inspection before approving. Pinned here: kind gating, payload tolerance
 * (untrusted, S2), and plain-text JSON output.
 */

import { describe, expect, it, vi } from 'vitest';

// DecisionItem's import graph reaches browser-flavored modules (store,
// notifications, settings hooks); stub them so the pure helper is testable in
// vitest's node environment. Nothing stubbed is under test.
vi.mock('../../../lib/store', () => ({ useCommandCenter: () => undefined }));
vi.mock('../../../lib/notifications', () => ({ toast: () => undefined }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./client', () => ({ decisionsClient: {} }));

import { pushedRejectWarning, toolArgumentsText, effectTextFor } from './DecisionItem';
import type { Decision } from './types';

function decision(overrides: Partial<Decision>): Decision {
  return {
    id: 'd-1',
    kind: 'tool_approval',
    goal_id: null,
    project_id: null,
    tier: 2,
    headline: 'Approve tool call: developer__shell',
    detail: "The assistant is requesting approval to run the 'developer__shell' tool",
    payload: {
      session_id: 's-1',
      request_id: 'r-1',
      tool_name: 'developer__shell',
      arguments: { command: 'ls -la' },
    },
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    created_at: '2026-07-01T00:00:00Z',
    resolved_at: null,
    ...overrides,
  };
}

describe('toolArgumentsText', () => {
  it('pretty-prints the full arguments for a tool_approval', () => {
    const text = toolArgumentsText(decision({}));
    expect(text).toBe(JSON.stringify({ command: 'ls -la' }, null, 2));
    // The whole value, not a preview: a long tail survives intact.
    const long = 'x'.repeat(5000);
    const withTail = decision({
      payload: {
        session_id: 's-1', request_id: 'r-1', tool_name: 't',
        arguments: { command: long },
      },
    });
    expect(toolArgumentsText(withTail)).toContain(long);
  });

  it('is offered only on tool_approval decisions', () => {
    expect(toolArgumentsText(decision({ kind: 'approve_review' }))).toBeNull();
    expect(toolArgumentsText(decision({ kind: 'risk_gate' }))).toBeNull();
  });

  it('tolerates missing or degenerate payloads (untrusted input)', () => {
    expect(toolArgumentsText(decision({ payload: null }))).toBeNull();
    expect(toolArgumentsText(decision({ payload: {} }))).toBeNull();
    expect(
      toolArgumentsText(decision({ payload: { arguments: null } })),
    ).toBeNull();
    // Empty-but-present arguments are still shown — "no arguments" is itself
    // information the approver should see.
    expect(toolArgumentsText(decision({ payload: { arguments: {} } }))).toBe('{}');
  });
});

describe('pushedRejectWarning (informed reject, #458 §3e)', () => {
  it('warns when an approve_review goal was already pushed — names the target and says reject will not un-ship it', () => {
    const text = pushedRejectWarning('approve_review', 'origin/main');
    expect(text).toContain('origin/main');
    expect(text).toContain("won't un-ship it");
    expect(text).toContain('revert');
  });

  it('stays silent when the work was not pushed (worktree-only evidence)', () => {
    expect(pushedRejectWarning('approve_review', null)).toBeNull();
    expect(pushedRejectWarning('approve_review', undefined)).toBeNull();
    expect(pushedRejectWarning('approve_review', '')).toBeNull();
  });

  it('applies only to approve_review — never other kinds, even with a push target', () => {
    expect(pushedRejectWarning('risk_gate', 'origin/main')).toBeNull();
    expect(pushedRejectWarning('unblock', 'origin/main')).toBeNull();
    expect(pushedRejectWarning('tool_approval', 'origin/main')).toBeNull();
  });
});

// session_gate (S3, #429): a supervised terminal session blocked on a
// can_use_tool gate. The gated tool input rides payload.input (not
// payload.arguments) — same informed-consent contract: full input
// inspectable before ruling.
function sessionGateDecision(overrides: Partial<Decision>): Decision {
  return decision({
    kind: 'session_gate',
    headline: 'A terminal session wants to run Write',
    detail: "Supervised session sup-1 is blocked on a can_use_tool gate: Write",
    payload: {
      question: 'Allow the session to run Write?',
      target_session_id: 'sup-1',
      pty_session_id: 'pty-1',
      request_id: 'perm_1',
      tool_name: 'Write',
      input: { path: 'foo.txt', content: 'hello' },
      tool_use_id: 'tu_1',
      options: ['allow', 'deny'],
    },
    ...overrides,
  });
}

describe('toolArgumentsText — session_gate', () => {
  it('pretty-prints the full gated tool input from payload.input', () => {
    expect(toolArgumentsText(sessionGateDecision({}))).toBe(
      JSON.stringify({ path: 'foo.txt', content: 'hello' }, null, 2),
    );
  });

  it('reads input (not arguments) for session_gate, and vice versa', () => {
    // A session_gate payload without `input` offers nothing, even if a stray
    // `arguments` key exists (untrusted payload, S2).
    expect(
      toolArgumentsText(sessionGateDecision({ payload: { arguments: { a: 1 } } })),
    ).toBeNull();
    // Empty-but-present input is still shown — "no input" is information.
    expect(toolArgumentsText(sessionGateDecision({ payload: { input: {} } }))).toBe('{}');
  });

  it('tolerates missing or degenerate payloads (untrusted input)', () => {
    expect(toolArgumentsText(sessionGateDecision({ payload: null }))).toBeNull();
    expect(toolArgumentsText(sessionGateDecision({ payload: {} }))).toBeNull();
    expect(toolArgumentsText(sessionGateDecision({ payload: { input: null } }))).toBeNull();
  });
});

describe('effectTextFor council_action', () => {
  it('says approve files a board card and reject dismisses', () => {
    expect(effectTextFor('council_action', 'approve', 'Henry')).toContain('board card');
    expect(effectTextFor('council_action', 'reject', 'Henry')).toMatch(/dismiss/i);
  });
});
