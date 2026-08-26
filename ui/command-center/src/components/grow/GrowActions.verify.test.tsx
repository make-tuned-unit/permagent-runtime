/**
 * @vitest-environment jsdom
 *
 * The rendered half of the action → verify → measure loop
 * (docs/proposals/grow-action-outcome-loop.md).
 *
 * `growActions.verify.test.ts` pins the rules against the source, which catches
 * a refactor that deletes them. This renders the component, which catches the
 * other failure: rules that are present in the code and never reach the screen
 * — a verdict whose rationale is in a branch that never runs, a percentage that
 * leaks past its guard, a Verify button that spins forever because the promise
 * rejected.
 *
 * Rendered directly rather than through GrowView (FunnelPanel.test.tsx's shape,
 * not GrowView.consume.test.tsx's): no 250ms swap fade to settle, and the
 * assertions are about this component rather than the tab shell.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({
  apiFetch,
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { GrowActions } from './GrowView';
import { getThemedColors } from '../../styles/tokens';
import type { Project } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

const project: Project = {
  id: 'p1', slug: 'p1', name: 'GrocerySaver', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [], metadataJson: {},
  createdAt: '2026-08-01T00:00:00Z', updatedAt: '2026-08-01T00:00:00Z',
  lastOpenedAt: '2026-08-01T00:00:00Z',
};

type Identity = {
  id: string;
  status: string;
  targetMetric: string | null;
  targetDir: string | null;
  verifiedBy: string | null;
  verifiedAt: string | null;
  outcomes: unknown[];
};

function actionsPayload(identity: Partial<Identity> | null) {
  return {
    actions: [{
      title: 'Expand the grocery-stores post',
      evidence: '8 of 37 pageviews land on it',
      recommendation: 'Expand and structure it',
      steps: [],
      // "none" on purpose: the proposal's `self` fallback row (:105). An action
      // with no artifact must still offer verification.
      artifactKind: 'none',
      artifact: null,
      category: 'aeo',
      impact: 'high',
      confidence: 'medium',
      identity: identity && {
        id: 'act-1', status: 'suggested', targetMetric: null, targetDir: null,
        verifiedBy: null, verifiedAt: null, outcomes: [], ...identity,
      },
    }],
    archived: [],
    dismissed: [],
    generatedAt: '2026-08-11T10:00:00Z',
    reason: null,
    periodDays: 30,
    droppedForNoTarget: 0,
    droppedAsRestatement: 0,
    droppedAsAlreadyPresent: 0,
  };
}

/**
 * Route the mock by URL, the way the real daemon would.
 *
 * `actions` may be a QUEUE — an array of payloads consumed one per GET, the
 * last one repeating once exhausted — rather than a single static payload.
 * A mock that always answers `/growth-actions` the same way cannot represent
 * a refetch at all, which is exactly why the same-tab bug this file guards
 * against (`ActionVerify.verify()` never calling `onChanged()`) shipped
 * without a test catching it: every existing test here only ever needed one
 * GET's worth of board.
 */
function routeTo(actions: unknown, verify?: unknown, reopen?: unknown) {
  const queue = Array.isArray(actions) ? [...actions] : null;
  apiFetch.mockImplementation((url: string) => {
    if (url.includes('/verify')) {
      return verify instanceof Error ? Promise.reject(verify) : Promise.resolve(verify);
    }
    if (url.includes('/reopen')) {
      return reopen instanceof Error ? Promise.reject(reopen) : Promise.resolve(reopen ?? {});
    }
    if (url.includes('/growth-actions')) {
      if (queue) return Promise.resolve(queue.length > 1 ? queue.shift() : queue[0]);
      return Promise.resolve(actions);
    }
    return Promise.resolve({});
  });
}

/**
 * The board payload from a SECOND (or later) GET, after `onChanged`
 * refetches. `render_board` (growth_actions.rs) builds `actions` and
 * `tracking` from the same durable rows split by status, so all a test has to
 * do to represent "the daemon now sees this row differently" is put it in the
 * other array — nothing here is a second source of truth.
 */
function boardState(over: { actions?: unknown[]; tracking?: unknown[] } = {}) {
  return {
    actions: [],
    tracking: [],
    archived: [],
    dismissed: [],
    generatedAt: '2026-08-11T10:00:00Z',
    reason: null,
    periodDays: 30,
    droppedForNoTarget: 0,
    droppedAsRestatement: 0,
    droppedAsAlreadyPresent: 0,
    ...over,
  };
}

/** One board row, reusing `actionsPayload`'s row shape with a caller-chosen
 *  identity — so a test can move the SAME row between lists rather than
 *  duplicating the title/evidence/steps fields it doesn't care about. */
function row(identity: Record<string, unknown>) {
  return { ...actionsPayload({}).actions[0], identity };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(ui: React.ReactElement) {
  await act(async () => root.render(ui));
}

function button(text: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent?.includes(text));
  expect(found, `no button matching "${text}"`).toBeTruthy();
  return found as HTMLButtonElement;
}

/**
 * A colour as the DOM will report it.
 *
 * The tokens are hex (`#DC2626`) and `el.style.color` reads back `rgb(220, 38,
 * 38)`, so comparing the two directly is always false — an assertion that can
 * never fail, guarding the rule that most needs guarding. Round-tripping the
 * token through a real element makes the comparison meaningful.
 */
function asRendered(hex: string): string {
  const probe = document.createElement('div');
  probe.style.color = hex;
  expect(probe.style.color, `${hex} is not a colour the DOM parsed`).not.toBe('');
  return probe.style.color;
}

function select(label: string, value: string) {
  const el = container.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;
  const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('Verify change', () => {
  // ── The agent owns the prediction ──────────────────────────────────
  //
  // Pre-registration is unchanged; WHO writes it is the point. The agent
  // recommended the action, so the agent states what it expects to move — that
  // is the claim the 7/14/28-day sweep grades it against. Asking the user for
  // it left no prediction of the agent's to be right or wrong.

  it('states the agent’s own prediction instead of asking the user for one', async () => {
    routeTo(actionsPayload({ targetMetric: 'sessions', targetDir: 'up' }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('I expect this to move');
    expect(container.textContent).not.toContain('Say what this should move');
    expect(
      container.querySelector('select[aria-label="Target metric"]'),
      'the user must not be asked to author a prediction the agent already made',
    ).toBeNull();
  });

  it('verifies against the agent’s prediction, not a user-entered one', async () => {
    routeTo(actionsPayload({ targetMetric: 'bounce_rate', targetDir: 'down' }), {
      verified: true,
      identity: {
        id: 'act-1', status: 'measuring',
        targetMetric: 'bounce_rate', targetDir: 'down',
        verifiedBy: 'self', verifiedAt: '2026-08-14T10:00:00Z', outcomes: [],
      },
      checks: [],
      reason: null,
    });
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('I did this').click(); });

    const call = apiFetch.mock.calls.find(([url]: [string]) => url.includes('/verify'));
    expect(call, 'no verify request was sent').toBeTruthy();
    expect(JSON.parse(call![1].body)).toMatchObject({
      targetMetric: 'bounce_rate',
      targetDir: 'down',
    });
  });

  /// A metric with no direction is not a prediction — "bounce rate moves" is
  /// true whichever way it goes. A half-filled pair must fall back to asking
  /// rather than silently assuming "up", which would score a worsening bounce
  /// rate as a success.
  it('falls back to asking when the agent named a metric but no direction', async () => {
    routeTo(actionsPayload({ targetMetric: 'sessions', targetDir: null }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).not.toContain('I expect this to move');
    expect(container.querySelector('select[aria-label="Target direction"]')).toBeTruthy();
  });

  /// Deliberately the reverse of what this file asserted until 2026-08-19.
  ///
  /// There WAS a "Measure something else" button here that revealed the metric
  /// selects and let the user substitute their own target for the agent's. It
  /// is gone, and the reversal is a product decision rather than a refactor:
  /// the target is the AGENT's prediction and this loop exists to grade the
  /// agent, so measuring a claim the agent never made produces a verdict about
  /// nobody — the exact unfalsifiability the pre-registration gate was built to
  /// stop. Now that the generator is required to name a target (an untargeted
  /// action is dropped rather than persisted), the selects are reachable only
  /// by a row that genuinely has none.
  it('will not let the user replace the agent’s prediction', async () => {
    routeTo(actionsPayload({ targetMetric: 'sessions', targetDir: 'up' }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.querySelector('select[aria-label="Target metric"]')).toBeNull();
    expect(container.querySelector('select[aria-label="Target direction"]')).toBeNull();
    expect(
      Array.from(container.querySelectorAll('button'))
        .find((b) => b.textContent?.includes('Measure something else')),
      'the override control is back',
    ).toBeUndefined();

    // The only thing offered is measuring against the agent's own claim, and
    // nothing on screen can reveal the selects.
    expect(button('I did this').disabled).toBe(false);
    await act(async () => { button('I did this').click(); });
    expect(container.querySelector('select[aria-label="Target metric"]')).toBeNull();
  });

  it('will not check anything until the claim is pre-registered', async () => {
    routeTo(actionsPayload({}));
    await render(<GrowActions project={project} colors={colors} />);

    // The backend refuses a verify with no target (growth_actions.rs:1011-1018).
    // The UI states the reason instead of firing a request that 400s.
    expect(container.textContent).toContain('Say what this should move before checking it');
    expect(button('Verify change').disabled).toBe(true);

    select('Target metric', 'sessions');
    expect(button('Verify change').disabled, 'a metric alone cannot be scored').toBe(true);
    select('Target direction', 'up');
    expect(button('Verify change').disabled).toBe(false);
  });

  it('posts the pre-registered target to the action’s verify route', async () => {
    routeTo(actionsPayload({}), {
      verified: true,
      identity: {
        id: 'act-1', status: 'verified', targetMetric: 'sessions', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-08-11T10:00:00Z', outcomes: [],
      },
      checks: [],
      reason: null,
    });
    await render(<GrowActions project={project} colors={colors} />);

    select('Target metric', 'sessions');
    select('Target direction', 'up');
    await act(async () => { button('Verify change').click(); });

    expect(apiFetch).toHaveBeenCalledWith(
      '/api/projects/p1/growth-actions/act-1/verify',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ targetMetric: 'sessions', targetDir: 'up' }),
      },
    );
  });

  it('renders a failed check as a reason and offers self-attestation', async () => {
    routeTo(actionsPayload({}), {
      verified: false,
      identity: null,
      checks: [{
        id: 'git_commit',
        label: 'A commit in this project’s repo since the action was issued',
        passed: false,
        detail: 'This project has no root path, so there is no repo to read.',
      }],
      reason: 'Nothing could confirm the change landed.',
    });
    await render(<GrowActions project={project} colors={colors} />);

    select('Target metric', 'pageviews');
    select('Target direction', 'up');
    await act(async () => { button('Verify change').click(); });

    // "Could not confirm" is not "not done" — the checks say which it was.
    expect(container.textContent).toContain('Nothing could confirm the change landed');
    expect(container.textContent).toContain('no root path');
    expect(button('It did land').disabled).toBe(false);
  });

  it('renders a thrown fetch instead of leaving the button checking', async () => {
    routeTo(actionsPayload({}), new Error('500 daemon unreachable'));
    await render(<GrowActions project={project} colors={colors} />);

    select('Target metric', 'sessions');
    select('Target direction', 'up');
    await act(async () => { button('Verify change').click(); });

    expect(container.textContent).toContain('Could not run the check');
    expect(container.textContent).toContain('500 daemon unreachable');
    expect(container.textContent).not.toContain('Checking…');
  });

  it('says so when the action has no saved record to verify against', async () => {
    // `persist` swallows database failures so a hiccup costs a Verify button
    // rather than the advice (growth_actions.rs:492-493). A silently missing
    // button is indistinguishable from a feature that was never built.
    routeTo(actionsPayload(null));
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('no saved record yet');
  });
});

describe('how it was verified', () => {
  const verifiedAt = '2026-08-11T10:00:00Z';

  it('names the commit when that is what was checked', async () => {
    routeTo(actionsPayload({
      status: 'verified', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: 'git', verifiedAt,
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('Verified from a commit');
    expect(container.textContent).not.toContain('your word');
  });

  it('does not let self-attestation read like a check', async () => {
    routeTo(actionsPayload({
      status: 'verified', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: 'self', verifiedAt,
    }));
    await render(<GrowActions project={project} colors={colors} />);
    // Proposal :107-109 — different claims, and they must not look identical.
    expect(container.textContent).toContain('your word, not a check');
    expect(container.textContent).not.toContain('Verified from a commit');
  });

  it('explains an empty outcome list as not yet due', async () => {
    routeTo(actionsPayload({
      status: 'measuring', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: 'git', verifiedAt,
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('Measuring. The first 7-day reading is due');
  });
});

describe('the verdict', () => {
  const verified = {
    status: 'judged', targetMetric: 'pageviews' as const, targetDir: 'up' as const,
    verifiedBy: 'git', verifiedAt: '2026-08-11T10:00:00Z',
  };

  it('renders an inconclusive verdict neutrally, with its reasoning and no number', async () => {
    routeTo(actionsPayload({
      ...verified,
      outcomes: [{
        windowDays: 7,
        verdict: 'inconclusive',
        rationale: '30 pageviews/week; a change under ~66% is indistinguishable from normal '
          + 'variance here.',
        // A delta IS present in the payload. Rendering it beside "not enough
        // data to say" is the proposal's named failure mode (:35-39), so the
        // guard has to hold with the number available, not merely absent.
        deltaPct: 0.12,
        confounders: [],
        judgedAt: '2026-08-19T00:00:00Z',
      }],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Not enough data to say');
    // The reasoning is always on screen; it is what makes the verdict arguable.
    expect(container.textContent).toContain('indistinguishable from normal variance');
    expect(container.textContent).not.toContain('+12%');
    // Early windows are read but labelled (proposal open decision 2).
    expect(container.textContent).toContain('provisional');

    // Neutral, not a failure state. Two things are checked because they fail
    // separately: nothing on the card is tinted with the danger colour, and the
    // verdict chip itself carries the body-copy tone rather than a dimmer one
    // than `no_effect` — "not a sad grey state" (proposal:47).
    const danger = asRendered(colors.danger);
    const tinted = Array.from(container.querySelectorAll<HTMLElement>('*'))
      .filter((el) => el.style.color === danger);
    expect(tinted.map((el) => el.textContent)).toEqual([]);

    const chip = Array.from(container.querySelectorAll<HTMLElement>('span'))
      .find((el) => el.textContent === 'Not enough data to say')!;
    expect(chip.style.color).toBe(asRendered(colors.textMuted));
    expect(chip.style.color).not.toBe(asRendered(colors.textDim));
  });

  it('shows the delta once a verdict actually rests on it', async () => {
    routeTo(actionsPayload({
      ...verified,
      outcomes: [{
        windowDays: 28,
        verdict: 'helped',
        rationale: 'Pageviews went from 900 to 1130 over 28 days, +26%, past the ~13% this '
          + 'project’s own variance allows.',
        deltaPct: 0.26,
        confounders: [],
        judgedAt: '2026-09-09T00:00:00Z',
      }],
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('Helped');
    expect(container.textContent).toContain('+26%');
    // The longest window is settled, so it carries no provisional caveat.
    expect(container.textContent).not.toContain('provisional');
  });

  it('names the overlapping action rather than claiming attribution', async () => {
    routeTo(actionsPayload({
      ...verified,
      outcomes: [{
        windowDays: 14,
        verdict: 'confounded',
        rationale: 'Another change was verified inside this window, so the two cannot be told '
          + 'apart.',
        deltaPct: 0.4,
        confounders: [{ id: 'act-2', title: 'Rewrite the hero copy' }],
        judgedAt: '2026-08-26T00:00:00Z',
      }],
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('Overlapped another change');
    expect(container.textContent).toContain('Rewrite the hero copy');
    // Attribution is unsolvable at this traffic; a percentage would imply it
    // was solved (proposal:126-128).
    expect(container.textContent).not.toContain('+40%');
  });
});

describe('same-tab refetch on verify', () => {
  // REGRESSION. The daemon now emits `project_changed` on verify, which bumps
  // `projectsRev` and refetches — but that is the OTHER tab's path (or this
  // tab's, after a round trip through the socket). The tab that fired the
  // verify itself has the answer sitting in its own POST response and used to
  // do nothing with it beyond `setResult`: the card stayed drawn from the
  // `actions` array in component state, which nothing here ever touched, so a
  // card that had genuinely verified on the daemon stayed visibly stuck in
  // the suggestion list until something else happened to remount the panel.
  it('a verified action leaves the suggestions and appears under Completed', async () => {
    const suggested = row({
      id: 'act-1', status: 'suggested', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: null, verifiedAt: null, outcomes: [],
    });
    const verifiedIdentity = {
      id: 'act-1', status: 'verified', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: 'git', verifiedAt: '2026-08-14T10:00:00Z',
      verifiedCommit: '8f2a1c334455667788990011223344556677889a',
      verifiedDetail: 'Commit 8f2a1c33 "Add FAQ block" changed src/pages/index.astro, which the '
        + 'action named as src/pages/index.astro.',
      outcomes: [],
    };

    routeTo(
      [
        boardState({ actions: [suggested] }),
        boardState({ actions: [], tracking: [row(verifiedIdentity)] }),
      ],
      { verified: true, identity: verifiedIdentity, checks: [], reason: null },
    );
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Expand the grocery-stores post');
    expect(container.textContent).not.toContain('Completed');

    await act(async () => { button('I did this').click(); });

    // Under Completed, carrying its receipt…
    expect(container.textContent).toContain('Completed (1)');
    expect(container.textContent).toContain('8f2a1c33');
    // …and gone from the suggestions: the title now appears exactly once,
    // where before this fix it would still also be sitting above as an
    // untouched suggestion.
    const titleHits = container.textContent!.match(/Expand the grocery-stores post/g) ?? [];
    expect(titleHits.length).toBe(1);
  });
});

describe('reopen', () => {
  it('posts to the reopen route and refetches the board', async () => {
    const tracked = row({
      id: 'act-1', status: 'verified', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: 'git', verifiedAt: '2026-08-14T10:00:00Z', outcomes: [],
    });
    const reopened = row({
      id: 'act-1', status: 'suggested', targetMetric: 'sessions', targetDir: 'up',
      verifiedBy: null, verifiedAt: null, outcomes: [],
    });

    routeTo(
      [
        boardState({ tracking: [tracked] }),
        boardState({ actions: [reopened] }),
      ],
      undefined,
      { id: 'act-1', status: 'suggested', verifiedAt: null, verifiedBy: null, outcomes: [] },
    );
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).toContain('Completed (1)');

    await act(async () => { button('Reopen').click(); });

    expect(apiFetch).toHaveBeenCalledWith(
      '/api/projects/p1/growth-actions/act-1/reopen',
      { method: 'POST' },
    );
    // The refetch put the row back under Actions, not left it sitting under a
    // Completed heading that the reopen route is supposed to empty out.
    expect(container.textContent).not.toContain('Completed (');
  });

  // The server's own guard (409, growth_actions.rs): reopening clears the
  // pivot an outcome was measured from, so once one exists Archive is the
  // only exit and the button is not offered rather than being offered and
  // refused every time it's pressed.
  it('offers no Reopen on a Completed card that already has a measured outcome', async () => {
    routeTo(boardState({
      tracking: [row({
        id: 'act-1', status: 'judged', targetMetric: 'pageviews', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-07-01T10:00:00Z',
        outcomes: [{
          windowDays: 28, verdict: 'helped',
          rationale: 'Pageviews went from 900 to 1130 over 28 days.',
          deltaPct: 0.26, confounders: [], judgedAt: '2026-07-30T00:00:00Z',
        }],
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Completed (1)');
    expect(
      Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes('Reopen')),
    ).toBeUndefined();
    // Archive is still the exit.
    expect(button('Archive')).toBeTruthy();
  });
});
