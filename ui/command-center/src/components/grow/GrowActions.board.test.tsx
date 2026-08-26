/**
 * @vitest-environment jsdom
 *
 * The board: what the panel renders now that the durable `growth_actions` rows
 * — not the wholesale-overwritten metadata bag — are the source of truth for it.
 *
 * `GrowActions.verify.test.tsx` covers the verify → measure half. This file
 * covers the six defects the 2026-08-19 review found, and every test here fails
 * against the code as it stood before it:
 *
 *  1. an in-flight action the last review did not re-emit vanished from the
 *     panel while the sweep was still measuring it (`hydrate` only decorated
 *     what the NEW cache contained);
 *  2. the user was asked to author the agent's prediction, and could overrule
 *     one the agent had already made;
 *  4. a passing check rendered as a bare badge — the checks block sat inside
 *     `result && !result.verified`;
 *  5. there was no archive control anywhere, in any state;
 *  6. nothing about another project's measured outcomes ever reached the UI.
 *
 * Rendered directly rather than through GrowView, for the reason
 * `GrowActions.verify.test.tsx` gives: no swap fade to settle, and the
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

type Identity = Record<string, unknown>;

/** An action as `render_board` assembles it: row fields always, prose maybe. */
function action(over: Record<string, unknown> = {}, identity: Identity | null = {}) {
  return {
    title: 'Expand the grocery-stores post',
    evidence: '8 of 37 pageviews land on it',
    recommendation: 'Expand and structure it',
    steps: [],
    artifactKind: 'none',
    artifact: null,
    category: 'aeo',
    impact: 'high',
    confidence: 'medium',
    identity: identity && {
      id: 'act-1', status: 'suggested', targetMetric: null, targetDir: null,
      verifiedBy: null, verifiedAt: null, outcomes: [], ...identity,
    },
    ...over,
  };
}

function payload(over: Record<string, unknown> = {}) {
  return {
    actions: [action()],
    archived: [],
    dismissed: [],
    tracking: [],
    generatedAt: '2026-08-19T10:00:00Z',
    reason: null,
    periodDays: 30,
    droppedForNoTarget: 0,
    droppedAsRestatement: 0,
    droppedAsAlreadyPresent: 0,
    generating: false,
    ...over,
  };
}

/** Route the mock by URL, the way the real daemon would. */
function routeTo(actions: unknown, verify?: unknown, status?: unknown) {
  apiFetch.mockImplementation((url: string) => {
    if (url.includes('/verify')) {
      return verify instanceof Error ? Promise.reject(verify) : Promise.resolve(verify);
    }
    if (url.includes('/status')) {
      return status instanceof Error ? Promise.reject(status) : Promise.resolve(status ?? {});
    }
    if (url.includes('/growth-actions')) return Promise.resolve(actions);
    return Promise.resolve({});
  });
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

function maybeButton(text: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll('button'))
    .find((b) => b.textContent?.includes(text));
}

function button(text: string): HTMLButtonElement {
  const found = maybeButton(text);
  expect(found, `no button matching "${text}"`).toBeTruthy();
  return found as HTMLButtonElement;
}

function getCalls(): string[] {
  return apiFetch.mock.calls
    .map(([url]: [string]) => url)
    .filter((u: string) => u.endsWith('/growth-actions'));
}

describe('the board is the durable rows, not the last review', () => {
  // The rendered half of defect 3. Keeping the action ON the board is the
  // server's job (`render_board` replaced `hydrate`, pinned in Rust); what the
  // panel owes is rendering a row whose PROSE the cache no longer holds. This
  // fails against the old card body, which printed the evidence rail and
  // "{impact} impact · {confidence} confidence" unconditionally — so a
  // prose-less row drew an empty bordered rail and the line " impact ·
  // confidence", which reads as a missing figure rather than as no figure.
  // Absent prose renders as nothing, for the same reason the backend refuses to
  // default a target metric: a guess in this panel is indistinguishable from a
  // measurement.
  it('renders an in-flight action the last review did not re-emit', async () => {
    routeTo(payload({
      actions: [action({ evidence: '', impact: '', confidence: '' }, {
        status: 'measuring', targetMetric: 'sessions', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-08-14T10:00:00Z',
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Expand the grocery-stores post');
    expect(container.textContent).toContain('Measuring. The first 7-day reading is due');
    // Absent prose renders as nothing, never as a guess: no empty evidence rail
    // and no invented "medium impact · medium confidence".
    expect(container.textContent).not.toContain('impact ·');
    const emptyRails = Array.from(container.querySelectorAll<HTMLElement>('div'))
      .filter((d) => d.style.borderLeft !== '' && d.textContent === '');
    expect(emptyRails, 'an empty evidence rail reads as a missing figure').toHaveLength(0);
  });
});

describe('the agent owns the prediction', () => {
  // REGRESSION for defect 2. The `Measure something else` button used to reveal
  // these selects for an action the agent HAD predicted, which produces a
  // verdict against a claim the agent never made.
  it('does not offer the metric dropdowns when the agent predicted a target', async () => {
    routeTo(payload({
      actions: [action({}, { targetMetric: 'sessions', targetDir: 'up' })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('I expect this to move');
    expect(container.querySelector('select[aria-label="Target metric"]')).toBeNull();
    expect(container.querySelector('select[aria-label="Target direction"]')).toBeNull();
    expect(maybeButton('Measure something else')).toBeUndefined();
  });

  // The seven rows that existed before the generator was required to predict
  // carry NULL targets, and backfilling them would grade a claim the agent
  // never made. They keep this path; it is now the legacy one, not the normal.
  it('still offers the dropdowns for an action that has no target', async () => {
    routeTo(payload({ actions: [action({}, { targetMetric: null, targetDir: null })] }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.querySelector('select[aria-label="Target metric"]')).toBeTruthy();
    expect(container.querySelector('select[aria-label="Target direction"]')).toBeTruthy();
    expect(button('Verify change').disabled, 'half a prediction is not one').toBe(true);
  });
});

describe('what the check found', () => {
  // REGRESSION for defect 4. The checks block sat inside
  // `result && !result.verified`, so a PASS showed a bare badge and threw away
  // the only thing the user could audit: which commit, which path.
  it('shows what the check found when verification passes', async () => {
    routeTo(
      payload({ actions: [action({}, { targetMetric: 'sessions', targetDir: 'up' })] }),
      {
        verified: true,
        identity: {
          id: 'act-1', status: 'verified', targetMetric: 'sessions', targetDir: 'up',
          verifiedBy: 'git', verifiedAt: '2026-08-19T10:00:00Z', outcomes: [],
        },
        checks: [{
          id: 'git_commit',
          label: 'A commit in this project’s repo since the action was issued',
          passed: true,
          detail: 'Commit 8f2a1c33 "Add FAQ block" changed src/pages/index.astro, which the '
            + 'action named as src/pages/index.astro.',
        }],
        reason: null,
      },
    );
    await render(<GrowActions project={project} colors={colors} />);
    await act(async () => { button('I did this').click(); });

    expect(container.textContent).toContain('What confirmed it');
    expect(container.textContent).toContain('8f2a1c33');
    expect(container.textContent).toContain('src/pages/index.astro');
  });

  // The evidence a check found is not persisted — there is no
  // `verified_detail` column — so a reload leaves the badge with nothing behind
  // it. Re-check is how it comes back, and the backend makes that call
  // read-only (`verify_mode`) so it cannot move the measurement pivot.
  it('recovers the evidence of an already-verified action without claiming more', async () => {
    routeTo(
      payload({
        actions: [action({}, {
          status: 'verified', targetMetric: 'sessions', targetDir: 'up',
          verifiedBy: 'self', verifiedAt: '2026-08-14T10:00:00Z',
        })],
      }),
      {
        verified: true,
        identity: {
          id: 'act-1', status: 'verified', targetMetric: 'sessions', targetDir: 'up',
          verifiedBy: 'self', verifiedAt: '2026-08-14T10:00:00Z', outcomes: [],
        },
        checks: [{
          id: 'git_commit',
          label: 'A commit in this project’s repo since the action was issued',
          passed: false,
          detail: 'This project has no root path, so there is no repo to read.',
        }],
        reason: null,
      },
    );
    await render(<GrowActions project={project} colors={colors} />);
    await act(async () => { button('Re-check').click(); });

    const call = apiFetch.mock.calls.find(([url]: [string]) => url.includes('/verify'));
    // The row's own pre-registration, never an empty string — the selects are
    // not rendered on a predicted action, so their state is blank.
    expect(JSON.parse(call![1].body)).toEqual({ targetMetric: 'sessions', targetDir: 'up' });

    // The action is verified on the user's word and stays that way, but four
    // failed checks must not be dressed up as corroboration.
    expect(container.textContent).toContain('your word, not a check');
    expect(container.textContent).toContain('What the checks found');
    expect(container.textContent).not.toContain('What confirmed it');
    expect(container.textContent).toContain('no root path');
    // Nothing to self-attest: it already is.
    expect(maybeButton('It did land')).toBeUndefined();
  });

  it('keeps the failure rendering and posts the real target when self-attesting', async () => {
    routeTo(
      payload({ actions: [action({}, { targetMetric: 'sessions', targetDir: 'up' })] }),
      {
        verified: false,
        identity: null,
        checks: [{
          id: 'git_commit',
          label: 'A commit in this project’s repo since the action was issued',
          passed: false,
          detail: 'This project has no root path, so there is no repo to read.',
        }],
        reason: 'Nothing could confirm the change landed.',
      },
    );
    await render(<GrowActions project={project} colors={colors} />);
    await act(async () => { button('I did this').click(); });

    expect(container.textContent).toContain('Nothing could confirm the change landed');
    expect(container.textContent).toContain('no root path');

    await act(async () => { button('It did land').click(); });
    const call = apiFetch.mock.calls.filter(([url]: [string]) => url.includes('/verify')).pop();
    // Taken from the identity, not from the empty select state. Posting
    // `targetMetric: ''` here would take a 400 from `parse_target` and read on
    // screen as a broken check.
    expect(JSON.parse(call![1].body)).toEqual({
      targetMetric: 'sessions', targetDir: 'up', selfAttested: true,
    });
  });
});

describe('the archive', () => {
  // REGRESSION for defect 5. No archived status, no route caller and no button
  // existed at all; an action the user had finished with stayed on the board
  // forever, and its text could never be released for re-proposal.
  it('archives an action and reloads the board', async () => {
    routeTo(payload({
      actions: [action({}, {
        status: 'judged', targetMetric: 'sessions', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-08-01T10:00:00Z',
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    const before = getCalls().length;
    await act(async () => { button('Archive').click(); });

    expect(apiFetch).toHaveBeenCalledWith(
      '/api/projects/p1/growth-actions/act-1/status',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ status: 'archived' }),
      },
    );
    // Archiving moves a card between two lists, so the parent has to re-read.
    expect(getCalls().length, 'the board was not refetched').toBe(before + 1);
  });

  it('does not offer Archive on a suggested action', async () => {
    // `reject_pointless_archive` refuses this with a 400, and offering the
    // button would turn a rule into an error the user has to decode. Archiving
    // is also what releases an action's text for re-proposal, so filing away
    // something never acted on would hand the same advice back next review.
    routeTo(payload({ actions: [action({}, { status: 'suggested' })] }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(maybeButton('Archive')).toBeUndefined();
  });

  it('lists archived actions in their own section, read-only', async () => {
    routeTo(payload({
      actions: [],
      archived: [action({ title: 'Rewrite the hero copy' }, {
        id: 'act-9', status: 'archived',
        targetMetric: 'pageviews', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-07-01T10:00:00Z',
        outcomes: [{
          windowDays: 28, verdict: 'helped',
          rationale: 'Pageviews went from 900 to 1130 over 28 days.',
          deltaPct: 0.26, confounders: [], judgedAt: '2026-07-30T00:00:00Z',
        }],
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Archived (1)');
    expect(container.textContent).toContain('Rewrite the hero copy');
    // Filing a card away must not destroy the data point it exists to keep.
    expect(container.textContent).toContain('Helped');
    expect(maybeButton('Archive')).toBeUndefined();
    expect(maybeButton('I did this')).toBeUndefined();
    // Only this one actually carries the readOnly claim: `Archive` is already
    // excluded because ARCHIVABLE omits 'archived', and `I did this` does not
    // render on a verified action either way. Asserted against the SAME action
    // rendered on the active list, so it pins the prop rather than the fixture.
    expect(maybeButton('Re-check')).toBeUndefined();
  });

  it('offers Re-check on that same action when it is not read-only', async () => {
    routeTo(payload({
      actions: [action({ title: 'Rewrite the hero copy' }, {
        id: 'act-9', status: 'judged',
        targetMetric: 'pageviews', targetDir: 'up',
        verifiedBy: 'git', verifiedAt: '2026-07-01T10:00:00Z',
        outcomes: [],
      })],
      archived: [],
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(maybeButton('Re-check')).toBeTruthy();
  });
});

describe('advice the user does not want', () => {
  // REGRESSION. `suggested` had NO exit: the server refuses to archive one
  // (archiving releases the text for re-proposal, so it would hand the same
  // advice back), and no control anywhere posted `dismissed`. Combined with the
  // active list becoming every non-archived row, the panel could only grow —
  // and past ~20 open cards the oldest work fell out of the generator's board
  // window entirely, which is the duplication defect this whole change exists
  // to close. This is the exit.
  it('offers a dismissal on a suggested action and posts it to the lifecycle route', async () => {
    routeTo(payload({ actions: [action({}, { status: 'suggested' })] }));
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('Not interested').click(); });

    const post = apiFetch.mock.calls.find(([u]: [string]) => u.includes('/status'));
    expect(post, 'dismissal must go through the one lifecycle route').toBeTruthy();
    expect(post[0]).toContain('/growth-actions/act-1/status');
    expect(JSON.parse(post[1].body)).toEqual({ status: 'dismissed' });
    // The board is re-read: dismissal moves a card between two lists, so the
    // card cannot patch itself.
    expect(getCalls().length).toBeGreaterThan(1);
  });

  // REGRESSION for the user report of 2026-08-19: "some of the actions I am
  // seeing in the Grow tab are stale ones that I already ran. I should be able
  // to dismiss it."
  //
  // The gate was `identity.status === 'suggested'`, so a `done` card — work the
  // user HAS already run — had no dismissal at all. Its only exit was Archive,
  // and archiving is what releases an action's text for re-proposal: filing the
  // stale card away handed the identical advice straight back on the next
  // review, which is precisely what happened on this project between 2026-08-14
  // and 2026-08-19.
  it('offers a dismissal on a card the user has already acted on', async () => {
    routeTo(payload({ actions: [action({}, { status: 'done' })] }));
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('Not interested').click(); });

    const post = apiFetch.mock.calls.find(([u]: [string]) => u.includes('/status'));
    expect(post[0]).toContain('/growth-actions/act-1/status');
    expect(JSON.parse(post[1].body)).toEqual({ status: 'dismissed' });
  });

  // The other half: a card whose prose the cache no longer holds. Four actions
  // on this user's real board have been there since 2026-08-14 with no entry
  // left in `metadata_json.growth_actions.prose`, so they render with no
  // evidence, no steps and no impact — and every control the panel offers hangs
  // off the identity, which comes from the durable ROW. A card the user can see
  // and cannot act on is the defect.
  it('offers a dismissal on a legacy card with no prose and no prediction', async () => {
    routeTo(payload({
      actions: [action(
        { evidence: '', steps: [], impact: '', confidence: '' },
        { status: 'suggested', targetMetric: null, targetDir: null },
      )],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('Not interested').click(); });

    const post = apiFetch.mock.calls.find(([u]: [string]) => u.includes('/status'));
    expect(post, 'a rendered row must always be dismissible').toBeTruthy();
    expect(JSON.parse(post[1].body)).toEqual({ status: 'dismissed' });
  });

  it('lists dismissed advice in its own section, not the archive', async () => {
    routeTo(payload({
      actions: [],
      dismissed: [action({ title: 'Rewrite the hero copy' }, {
        id: 'act-7', status: 'dismissed',
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Dismissed (1)');
    // Not folded into the archive: the two are opposites to the generator —
    // dismissed text stays on its board and can never be re-proposed, archived
    // text is released.
    expect(container.textContent).not.toContain('Archived (');
    expect(maybeButton('Not interested')).toBeUndefined();
  });

  // REGRESSION. The refusal body was `text/plain` and `apiFetch` parses every
  // error body as JSON, so the server's one refusal worth reading arrived as
  // the literal string "Unknown error". The daemon now sends
  // `{"message": ...}`; this pins that the card renders whatever it is given
  // rather than swallowing it.
  it('shows the server’s reason when a lifecycle move is refused', async () => {
    routeTo(
      payload({ actions: [action({}, { status: 'judged' })] }),
      undefined,
      new Error('Nothing has happened to this action yet, so there is nothing to file away.'),
    );
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('Archive').click(); });

    expect(container.textContent).toContain('Nothing has happened to this action yet');
    expect(container.textContent).not.toContain('Unknown error');
  });
});

describe('what worked elsewhere', () => {
  // REGRESSION for defect 6. Every learning read was `WHERE project_id = ?`, so
  // nothing another project measured could ever reach this panel — which is
  // what left low-traffic projects unable to learn anything at all.
  it('says where a category worked before, and on which project', async () => {
    routeTo(payload({
      actions: [action({
        transfer: {
          category: 'aeo',
          projects: 3, helped: 2, hindered: 0, noEffect: 1,
          medianDeltaPct: 0.18,
          segmentLabel: 'content site, under 100 views/wk, mostly search',
          segmentProjects: 2, segmentHelped: 1, segmentHindered: 0, segmentNoEffect: 1,
          examples: [{
            projectName: 'Evntally', title: 'Add FAQPage schema to the venue guides',
            verdict: 'helped', deltaPct: 0.22,
          }],
        },
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Worked on 2 of 3 other project(s)');
    // The segment is reported separately from the aggregate on purpose: merging
    // them hides an overall "helped" that quietly fails on projects like this.
    expect(container.textContent).toContain('on projects like this one, 1 of 2');
    expect(container.textContent).toContain('content site, under 100 views/wk, mostly search');

    // Provenance is mandatory: a card that appears because a category worked
    // elsewhere and will not say where is not auditable.
    const summary = container.querySelector('summary')!;
    await act(async () => { summary.click(); });
    expect(container.textContent).toContain('Evntally');
    expect(container.textContent).toContain('Add FAQPage schema to the venue guides');
    expect(container.textContent).toContain('+22%');
  });
});

describe('the suggestions that were dropped', () => {
  // Both guards silently withhold advice — the reword guard drops a suggestion
  // the user never sees, and an untargeted action is discarded outright.
  // Counting them out loud is the only thing that makes either auditable.
  it('says how many suggestions the last review dropped and why', async () => {
    routeTo(payload({ droppedForNoTarget: 1, droppedAsRestatement: 2, droppedAsAlreadyPresent: 1 }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Last review dropped 4 suggestion(s)');
    expect(container.textContent).toContain('2 restated something already on your board');
    expect(container.textContent).toContain('1 made no measurable prediction');
    expect(container.textContent).toContain('1 already in the repo');
  });

  it('says nothing when nothing was dropped', async () => {
    routeTo(payload());
    await render(<GrowActions project={project} colors={colors} />);
    expect(container.textContent).not.toContain('Last review dropped');
  });
});

// ── Tracking: what we changed, and whether it worked ─────────────────────────
//
// The user, 2026-08-19: "once a verified action was taken it should disappear
// from the list of Actions and go into a tracker view of changes we made that
// we are tracking to see how it impacted the success of the project."
//
// #1053 deliberately left `verified`/`measuring` on the active board so nothing
// in flight could silently vanish while the sweep still measured it. That
// guarantee is kept — these rows are MOVED, not hidden, and every one is still
// rendered with its evidence, its prediction, its baseline and its windows.

/** A verified action with a frozen baseline, as `render_board` sends it. */
function tracked(over: Record<string, unknown> = {}, identity: Identity = {}) {
  return action({ title: 'Rewrite the homepage', ...over }, {
    id: 'act-tracked',
    status: 'verified',
    targetMetric: 'bounce_rate',
    targetDir: 'down',
    verifiedBy: 'git',
    verifiedAt: '2026-08-19T12:00:00Z',
    outcomes: [],
    baseline: {
      metric: 'bounce_rate',
      dir: 'down',
      pivot: '2026-08-20',
      takenAt: '2026-08-19T12:00:00Z',
      windows: [
        { windowDays: 7, start: '2026-08-12', end: '2026-08-19', value: 0.47, denominator: 220 },
        { windowDays: 14, start: '2026-08-05', end: '2026-08-19', value: 0.44, denominator: 460 },
        { windowDays: 28, start: '2026-07-22', end: '2026-08-19', value: 0.42, denominator: 910 },
      ],
    },
    ...identity,
  });
}

describe('the tracking view', () => {
  it('renders verified work in its own section, not in the Actions list', async () => {
    routeTo(payload({ actions: [], tracking: [tracked()] }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Completed (1)');
    expect(container.textContent).toContain('Rewrite the homepage');
    // An empty Actions list beside a full Tracking list is not "nothing to
    // say" — saying the wrong one of those reads as data loss.
    expect(container.textContent).toContain('Nothing is waiting on you');
    expect(container.textContent).not.toContain('No review yet');
  });

  it('says what was done, how it was confirmed, and what it predicted', async () => {
    routeTo(payload({ actions: [], tracking: [tracked()] }));
    await render(<GrowActions project={project} colors={colors} />);

    // The verification KIND, in words — "verified from a commit" and "you told
    // me so" are different claims (proposal:107-109).
    expect(container.textContent).toContain('Verified from a commit');
    expect(container.textContent).toContain('bounce rate should go down');
  });

  it('shows the frozen baseline every window is measured against', async () => {
    routeTo(payload({ actions: [], tracking: [tracked()] }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('Measuring against');
    // A rate renders as a rate. "0.47" beside a pageview count reads as a
    // broken number rather than as 47%.
    expect(container.textContent).toContain('before: 47%');
    expect(container.textContent).toContain('of 220 sessions');
    expect(container.textContent).toContain('Windows start 2026-08-20');
  });

  it('walks all three windows, not just the first', async () => {
    // The old card said only "the first 7-day reading is due X", which tells
    // the user a 28-day verdict is not coming.
    routeTo(payload({ actions: [], tracking: [tracked()] }));
    await render(<GrowActions project={project} colors={colors} />);

    for (const days of [7, 14, 28]) {
      expect(container.textContent, `${days}-day window`).toContain(`${days}-day`);
    }
  });

  it('shows the verdict and its reason once the sweep produces one', async () => {
    routeTo(payload({
      actions: [],
      tracking: [tracked({}, {
        status: 'judged',
        outcomes: [{
          windowDays: 7,
          verdict: 'inconclusive',
          rationale: '38 sessions before, 41 after — too few to separate from noise.',
          deltaPct: null,
          confounders: [],
          judgedAt: '2026-08-27T00:00:00Z',
        }],
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    // `inconclusive` is the EXPECTED outcome at this traffic and must read as
    // neutral, with the numbers it rests on in body text (proposal:46-48).
    expect(container.textContent).toContain('Not enough data to say');
    expect(container.textContent).toContain('too few to separate from noise');
  });

  // Archiving a tracked action keeps measuring it; dismissing would drop a
  // live experiment into the pile of advice the user refused.
  it('offers Archive on a tracked card, and not a dismissal', async () => {
    routeTo(payload({ actions: [], tracking: [tracked()] }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(maybeButton('Archive')).toBeTruthy();
    expect(maybeButton('Not interested')).toBeUndefined();
  });

  // The receipt for "this shipped". A commit chip only when the passing check
  // was `git` — `verifiedCommit` is omitted from the JSON for every other
  // strategy (routes/growth_actions.rs), so a card verified by content, an
  // event, or the user's own word must never grow a sha it was never given.
  it('renders the short sha as a receipt when the card was verified from a commit', async () => {
    routeTo(payload({
      actions: [],
      tracking: [tracked({}, {
        verifiedCommit: '8f2a1c334455667788990011223344556677889a',
        verifiedDetail: 'Commit 8f2a1c33 "Add FAQ block" changed src/pages/index.astro, which '
          + 'the action named as src/pages/index.astro.',
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(container.textContent).toContain('8f2a1c33');
    // Shortened, not the full sha — the full 40 characters would be the kind
    // of noise the chip exists to avoid.
    expect(container.textContent).not.toContain('8f2a1c334455667788990011223344556677889a');
    expect(container.textContent).toContain('Commit 8f2a1c33 "Add FAQ block" changed src/pages/index.astro');
  });

  it('shows the strategy instead of a sha when there is no commit to name', async () => {
    routeTo(payload({
      actions: [],
      tracking: [tracked({}, { verifiedBy: 'self', verifiedCommit: null })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    // The chip's own words, not fabricated ones — "you told me it landed" is
    // the same self-attest copy `verifiedByMeta` already uses everywhere else
    // on the card, and reusing it is what keeps this claim honest.
    expect(container.textContent).toContain('your word, not a check');
    expect(container.textContent).not.toContain('commit ');
  });
});

// ── The background review ────────────────────────────────────────────────────
//
// The user, 2026-08-19: "on GetLadle I pressed Review my analytics and then
// clicked another tab, when I went back it looks like it stopped running. It
// should run in the background and there should be a loading animation on the
// button to show that the agent is crunching the numbers."
//
// The flag was `useState(false)` inside this component, which unmounts when the
// tab changes — so it was destroyed while the review carried on, and coming
// back showed an idle button over a running review whose result then landed in
// the database unseen.

describe('a review that outlives the tab', () => {
  it('shows it running on mount when the server says one is', async () => {
    // Nothing was clicked in THIS component. Its state is the server's.
    routeTo(payload({ generating: true, generationStartedAt: '2026-08-19T13:00:00Z' }));
    await render(<GrowActions project={project} colors={colors} />);

    const b = button('Reviewing your analytics…');
    expect(b.disabled).toBe(true);
    expect(b.getAttribute('aria-busy')).toBe('true');
    expect(container.textContent).toContain('This keeps running if you leave the tab');
  });

  it('puts a moving spinner on the button, not just a different word', async () => {
    routeTo(payload({ generating: true }));
    await render(<GrowActions project={project} colors={colors} />);

    // `pa-spin` is the app's spinner utility (index.css), which honours
    // prefers-reduced-motion through the global block there. A text swap alone
    // is indistinguishable from a stuck button.
    expect(button('Reviewing your analytics…').querySelector('.pa-spin')).toBeTruthy();
  });

  it('spins from the moment the click is registered, before the server answers', async () => {
    let release: (v: unknown) => void = () => {};
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/generate')) return new Promise((r) => { release = r; });
      if (url.includes('/growth-actions')) return Promise.resolve(payload({ actions: [] }));
      return Promise.resolve({});
    });
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('Review my analytics').click(); });
    // The POST has not resolved. Without a local bridge the button would sit
    // idle for the whole round trip.
    expect(maybeButton('Reviewing your analytics…')).toBeTruthy();

    await act(async () => { release(payload({ generating: true })); });
    expect(button('Reviewing your analytics…').disabled).toBe(true);
  });

  it('does not start a second review on a second click', async () => {
    routeTo(payload({ generating: true }));
    await render(<GrowActions project={project} colors={colors} />);

    const b = button('Reviewing your analytics…');
    await act(async () => { b.click(); b.click(); });

    const posts = apiFetch.mock.calls.filter(([u]: [string]) => u.includes('/generate'));
    expect(posts).toHaveLength(0);
  });

  it('reconciles to the finished result without a manual refresh', async () => {
    vi.useFakeTimers();
    try {
      let running = true;
      apiFetch.mockImplementation((url: string) => {
        if (url.includes('/growth-actions/generate')) return Promise.resolve(payload());
        if (url.includes('/growth-actions')) {
          return Promise.resolve(running
            ? payload({ actions: [], generating: true })
            : payload({ actions: [action({ title: 'Expand the pricing page' })] }));
        }
        return Promise.resolve({});
      });
      await render(<GrowActions project={project} colors={colors} />);
      expect(container.textContent).toContain('Reviewing your analytics…');

      // The review finishes on the daemon while nothing is pressed here.
      running = false;
      await act(async () => { await vi.advanceTimersByTimeAsync(5000); });

      expect(container.textContent).toContain('Expand the pricing page');
      expect(container.textContent).not.toContain('Reviewing your analytics…');
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops polling once the review is done', async () => {
    vi.useFakeTimers();
    try {
      routeTo(payload());
      await render(<GrowActions project={project} colors={colors} />);
      const before = getCalls().length;
      await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
      // Not a background poll of the panel: it only ticks while a review runs.
      expect(getCalls().length).toBe(before);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('actions grouped by category tab', () => {
  it('puts mixed actions under Measurement / Acquisition / UX tabs, not one tagged list', async () => {
    routeTo(payload({
      actions: [
        action({ title: 'Instrument search events', category: 'measurement' }, { id: 'm1' }),
        action({ title: 'List in a directory', category: 'acquisition' }, { id: 'a1' }),
        action({ title: 'Rewrite the hero', category: 'ux' }, { id: 'u1' }),
      ],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    expect(button('Measurement (1)')).toBeTruthy();
    expect(button('Acquisition (1)')).toBeTruthy();
    expect(button('UX (1)')).toBeTruthy();
    // Stable order, not first-seen: Measurement leads even though it is not
    // the first row, and that tab is selected so only its cards mount.
    expect(container.textContent).toContain('Instrument search events');
    expect(container.textContent).not.toContain('List in a directory');
    expect(container.textContent).not.toContain('Rewrite the hero');

    await act(async () => { button('UX (1)').click(); });
    expect(container.textContent).toContain('Rewrite the hero');
    expect(container.textContent).not.toContain('Instrument search events');
  });

  it('does not invent a tab for a category with no actions', async () => {
    routeTo(payload({
      actions: [action({ title: 'Add FAQPage schema', category: 'aeo' })],
    }));
    await render(<GrowActions project={project} colors={colors} />);
    expect(button('AEO (1)')).toBeTruthy();
    expect(maybeButton('Measurement')).toBeUndefined();
    expect(maybeButton('SEO')).toBeUndefined();
  });
});

describe('the coding-agent prompt', () => {
  it('wraps a drafted SEO post as a prompt, with a send-to-agent control', async () => {
    routeTo(payload({
      actions: [action({
        title: 'Publish a comparison post',
        artifactKind: 'post',
        artifact: 'Grocery delivery is expensive. Here is how to spend less.',
        category: 'seo',
      })],
    }));
    await render(<GrowActions project={project} colors={colors} />);

    await act(async () => { button('SEO (1)').click(); });
    expect(container.textContent).toContain('Prompt for your coding agent');
    expect(container.textContent).toContain('coding agent');
    expect(container.textContent).toContain('Grocery delivery is expensive');
    expect(container.textContent).toContain('GROW_ACTION_DONE act-1');
    expect(container.querySelector('select[aria-label="Coding agent"]')).toBeTruthy();
    expect(maybeButton('Send')).toBeTruthy();
    expect(maybeButton('Copy')).toBeTruthy();
  });
});
