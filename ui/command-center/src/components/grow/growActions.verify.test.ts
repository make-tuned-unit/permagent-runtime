/**
 * The honesty rules of the action → verify → measure loop, pinned against the
 * source because every one of them is a one-character edit away from being
 * silently undone and none of them fails loudly when it is.
 *
 * From docs/proposals/grow-action-outcome-loop.md:
 *
 *  - `verified_by` is shown on the card, because "'Verified from a commit' and
 *    'you told me so' are different claims and must not look identical"
 *    (:107-109). A styling refactor that collapses the two branches into one
 *    chip loses the distinction while still rendering something plausible.
 *  - "'Inconclusive' is the expected outcome, not a failure. It must be the
 *    visually neutral default, not a sad grey state, or there is pressure to
 *    manufacture verdicts" (:46-48). Reaching for `colors.danger` here is the
 *    natural instinct and the wrong one.
 *  - "A feature that says 'this helped, +12%' off 40 pageviews is not
 *    measuring; it is pattern-matching noise and presenting it as evidence"
 *    (:35-39) — so a delta may not render beside a verdict that does not rest
 *    on it.
 *  - The rationale is body text, always: the column is NOT NULL by design
 *    (proposal:85) and a verdict whose reasoning hides in a tooltip cannot be
 *    argued with.
 */
import { describe, expect, it } from 'vitest';

import { GROW_SOURCE as SOURCE } from './growSource';

/** The body of a named top-level function in GrowView.tsx. */
function fn(name: string): string {
  const at = SOURCE.indexOf(`function ${name}(`);
  expect(at, `${name} not found`).toBeGreaterThan(-1);
  const next = SOURCE.indexOf('\nfunction ', at + 1);
  const nextExported = SOURCE.indexOf('\nexport function ', at + 1);
  const ends = [next, nextExported].filter((i) => i > -1);
  return SOURCE.slice(at, ends.length ? Math.min(...ends) : SOURCE.length);
}

describe('the verify control', () => {
  it('is rendered for every action, not only ones with an artifact', () => {
    // artifactKind "none" is the proposal's `self` fallback row (:105). Nesting
    // the control inside `{a.artifact && …}` would hide verification for
    // exactly the actions that have no deliverable to copy — and it would still
    // look right on every card that happens to carry a prompt, which is most of
    // them. So this walks the braces rather than comparing offsets: an
    // ActionVerify placed after the artifact block AND a second one inside it
    // both read as "after" to a positional check.
    // The card body moved out of GrowActions' map into its own top-level
    // component when the archived shelf started rendering the same card
    // read-only. The assertion is unchanged and still worth pinning.
    const card = fn('ActionCard');
    const open = card.indexOf("{lane === 'actions' && (");
    expect(open, 'the coding-agent prompt block moved or was renamed').toBeGreaterThan(-1);

    let depth = 0;
    let close = -1;
    for (let i = open; i < card.length; i += 1) {
      if (card[i] === '{') depth += 1;
      else if (card[i] === '}') {
        depth -= 1;
        if (depth === 0) { close = i; break; }
      }
    }
    expect(close, 'the artifact conditional never closes').toBeGreaterThan(open);

    const uses: number[] = [];
    for (let i = card.indexOf('<ActionVerify'); i > -1; i = card.indexOf('<ActionVerify', i + 1)) {
      uses.push(i);
    }
    expect(uses, 'the verify control is not rendered at all').toHaveLength(1);
    expect(uses[0] > open && uses[0] < close, 'verify is hidden inside the artifact block')
      .toBe(false);
  });

  it('pre-registers a target before it will check anything', () => {
    // The backend refuses a verify with no pre-registered metric
    // (growth_actions.rs:1011-1018). Offering the button anyway would turn that
    // refusal into a 400 the user has to decode.
    const v = fn('ActionVerify');
    expect(v).toContain('targetMetric');
    expect(v).toContain('targetDir');
    expect(v).toMatch(/disabled=\{busy \|\| !metric \|\| !dir\}/);
  });

  it('offers only the metrics the backend can actually measure', () => {
    // TargetMetric is a closed set (metrics.rs:41-47): device signatures merge
    // people and bots_excluded is a filter diagnostic, so neither can carry a
    // verdict. A free-text field here would make pre-registration decorative.
    const list = SOURCE.slice(SOURCE.indexOf('const TARGET_METRICS'));
    for (const metric of ['pageviews', 'sessions', 'aeo_visits', 'bounce_rate']) {
      expect(list).toContain(`'${metric}'`);
    }
    expect(SOURCE).not.toContain('device_signatures');
  });

  it('turns a thrown fetch into a rendered result, not a stuck button', () => {
    const v = fn('ActionVerify');
    expect(v).toMatch(/\.catch\(\(e\) => setResult\(/);
    expect(v).toContain('Could not run the check');
    expect(v).toMatch(/\.finally\(\(\) => setBusy\(false\)\)/);
  });

  // REGRESSION. A pass moves the card between two lists (Actions →
  // Completed), so `verify()` patching only its own `result` state left a
  // verified card visibly stuck in the suggestion list — the daemon had the
  // right answer and nothing on screen ever asked for it again. Mirrors "the
  // archive" describe's pin on `move()`'s `onChanged()` below.
  it('refetches the board on a pass, the same way move() does', () => {
    const v = fn('ActionVerify');
    const verify = v.slice(v.indexOf('const verify = useCallback'));
    const body = verify.slice(0, verify.indexOf('}, [projectId'));
    expect(body).toContain('onChanged()');
    // Not unconditional: a failed or self-attest-eligible check leaves the
    // card exactly where it is, so a fetch that did not verify has nothing to
    // refetch for.
    expect(body).toMatch(/if \(res\.verified\) onChanged\(\)/);
  });
});

describe('how it was verified is shown, not just that it was', () => {
  it('gives each verification method its own words', () => {
    const meta = fn('verifiedByMeta');
    expect(meta).toContain("case 'git'");
    expect(meta).toContain("case 'content'");
    expect(meta).toContain("case 'event'");
    expect(meta).toContain("case 'self'");
    // The two claims the proposal names explicitly (:107-109).
    expect(meta).toMatch(/Verified from a commit/);
    expect(meta).toMatch(/You told me it landed/);
  });

  it('styles self-attestation differently from a real check', () => {
    // Different words alone are not enough — they read the same at a glance.
    // `checked` drives border style and colour apart as well.
    expect(fn('verifiedByMeta')).toContain('checked: false');
    const v = fn('ActionVerify');
    expect(v).toContain('provenance.checked');
    expect(v).toMatch(/provenance\.checked \? 'solid' : 'dashed'/);
  });
});

describe('inconclusive is neutral, not a failure', () => {
  const meta = fn('verdictMeta');

  it('never tints a verdict other than hindered with the danger colour', () => {
    // `inconclusive` is the default branch, so an accidental danger tint here
    // would hit the most common outcome at this traffic.
    const dangerLines = meta.split('\n').filter((l) => l.includes('colors.danger'));
    expect(dangerLines).toHaveLength(1);
    expect(dangerLines[0]).toContain('hindered');
  });

  it('does not render inconclusive dimmer than a settled verdict', () => {
    // "not a sad grey state" (:47). `no_effect` — a real, settled finding — is
    // the dimmest thing here; inconclusive sits at body-copy weight.
    expect(meta).toMatch(/case 'no_effect'.*colors\.textDim/);
    expect(meta).toMatch(/default:.*colors\.textMuted/s);
  });

  it('reads as a statement about the data, not about the action', () => {
    expect(meta).toContain('Not enough data to say');
    expect(meta).not.toMatch(/[Ff]ail|[Ii]nvalid|[Ee]rror/);
  });
});

describe('the numbers only appear where a verdict rests on them', () => {
  it('suppresses the delta unless the verdict is helped or hindered', () => {
    const v = fn('ActionVerify');
    expect(v).toMatch(
      /showsDelta = \(o\.verdict === 'helped' \|\| o\.verdict === 'hindered'\)/,
    );
    expect(v).toContain('{showsDelta && (');
  });

  it('always renders the rationale as body text', () => {
    const v = fn('ActionVerify');
    expect(v).toContain('{o.rationale}');
    // Not a tooltip: a hidden justification cannot be argued with.
    expect(v).not.toMatch(/title=\{o\.rationale\}/);
  });

  it('labels windows shorter than 28 days as provisional', () => {
    // Proposal open decision 2: 7/14/28 evaluated progressively, "with early
    // windows explicitly labelled provisional".
    expect(fn('ActionVerify')).toContain('provisional');
  });

  it('explains an empty outcome list as not-due rather than nothing-found', () => {
    const v = fn('ActionVerify');
    expect(v).toContain('identity.outcomes.length === 0');
    expect(v).toContain('Measuring. The first');
  });
});

describe('the agent owns the prediction', () => {
  /**
   * The metric selects are the FALLBACK, not a control.
   *
   * There used to be a "Measure something else" button beside the agent's
   * prediction that revealed these selects and let the user substitute their
   * own target. That produces a verdict against a claim the agent never made —
   * the exact unfalsifiability the pre-registration gate exists to stop — and
   * it is one JSX line away from coming back. This walks the ternary rather
   * than grepping, because a select rendered ABOVE the ternary would satisfy
   * any positional check while being visible in both branches.
   */
  it('keeps the metric selects inside the branch for an action with no prediction', () => {
    const v = fn('ActionVerify');
    // The CONTROL, not the phrase: the source still names what was removed and
    // why, and that comment is the thing most likely to stop it coming back.
    expect(v, 'the override state is back').not.toContain('setOverriding');
    expect(v, 'the override control is back')
      .not.toContain('>Measure something else</button>');

    const head = '{predicted ? (';
    const at = v.indexOf(head);
    expect(at, 'the prediction branch moved or was renamed').toBeGreaterThan(-1);

    // Walk from the paren that opens the TRUE branch to its match; everything
    // after it is the false branch.
    let depth = 0;
    let i = at + head.length - 1;
    for (; i < v.length; i += 1) {
      if (v[i] === '(') depth += 1;
      else if (v[i] === ')') {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    const predictedBranch = v.slice(at, i);
    const unpredictedBranch = v.slice(i);

    expect(predictedBranch, 'the agent’s own prediction can be swapped out')
      .not.toContain('aria-label="Target metric"');
    expect(unpredictedBranch, 'the fallback selects are gone entirely')
      .toContain('aria-label="Target metric"');
  });

  it('sends the row’s own pre-registration rather than an empty string', () => {
    // The self-attest and re-check buttons now render for a PREDICTED action
    // too, where `metric`/`dir` are never filled in. Posting those empty would
    // take a 400 from `parse_target` and read on screen as a broken check.
    const v = fn('ActionVerify');
    expect(v).toContain('const targetBody =');
    expect(v).toMatch(/verify\(\{ \.\.\.targetBody\(\), selfAttested: true \}\)/);
  });
});

describe('the archive', () => {
  it('is never offered for an action nothing has happened to', () => {
    // `reject_pointless_archive` (growth_actions.rs) refuses this on the server
    // with a 400, so offering the button would turn a rule into an error the
    // user has to decode. Archiving is also what releases an action's text for
    // re-proposal, so filing away something never acted on would hand the same
    // advice back next review.
    const list = SOURCE.slice(SOURCE.indexOf('const ARCHIVABLE'));
    const decl = list.slice(0, list.indexOf(';'));
    expect(decl).toContain("'done'");
    expect(decl).toContain("'dismissed'");
    expect(decl, 'suggested must not be archivable').not.toContain("'suggested'");

    const card = fn('ActionCard');
    // Anchored to the declaration rather than searching the whole component for
    // the token: a bare `toContain('!readOnly')` matched any stray occurrence
    // anywhere in the body and would have survived the guard being moved off
    // this control entirely.
    const canArchive = card.slice(card.indexOf('const canArchive'));
    expect(canArchive.slice(0, canArchive.indexOf(';'))).toContain(
      '!readOnly && !!identity && ARCHIVABLE.includes(identity.status)',
    );
  });

  // REGRESSION for the user report of 2026-08-19: "some of the actions I am
  // seeing in the Grow tab are stale ones that I already ran. I should be able
  // to dismiss it."
  //
  // This was `identity.status === 'suggested'` — a claim about lifecycle, where
  // the need is about the list. A `done` card whose work the user no longer
  // cares about could only be ARCHIVED, and archiving is precisely what
  // releases an action's text for re-proposal, so filing stale advice away
  // handed the identical advice back on the next review. The gate is now the
  // durable row: if the Actions lane can render it, the Actions lane can post a
  // dismissal for it.
  it('lets any card on the Actions lane be dismissed, whatever its status', () => {
    const card = fn('ActionCard');
    const canDismiss = card.slice(card.indexOf('const canDismiss'));
    const decl = canDismiss.slice(0, canDismiss.indexOf(';'));
    expect(decl).toContain("lane === 'actions'");
    // The DURABLE row id, not the prose cache and not a status allowlist. The
    // four actions this project has carried since 2026-08-14 have no cache
    // entry left, and every control hangs off the identity.
    expect(decl).toContain('!!actionId');
    expect(decl, 'a status allowlist is what left `done` with no exit')
      .not.toContain("identity.status ===");
    expect(card).toContain("move('dismissed')");
  });

  it('posts both lifecycle moves to the one route', () => {
    const card = fn('ActionCard');
    expect(card).toContain("move('archived')");
    // One helper, one route, one body shape — so a second exit cannot drift on
    // to a second endpoint.
    const move = card.slice(card.indexOf('const move = useCallback'));
    const body = move.slice(0, move.indexOf('}, [project.id'));
    expect(body).toContain('/status`');
    expect(body).toContain('JSON.stringify({ status })');
    // The card cannot move itself between two lists; the parent re-reads.
    expect(body).toContain('onChanged()');
  });
});

describe('what the row says and what the cache says', () => {
  it('renders absent prose as nothing rather than as a guess', () => {
    // The durable row is the truth and has no column for evidence, impact or
    // confidence; those come from a prose cache a later review can prune.
    // Defaulting them to "medium" would be the same invention the backend
    // refuses when it declines to default a target metric.
    const card = fn('ActionCard');
    expect(card).toContain('{action.evidence && (');
    expect(card).toContain('{action.impact && action.confidence && (');
  });

  it('names the project every transferred result came from', () => {
    // A card that appears because a category worked elsewhere and will not say
    // where is not auditable, and is indistinguishable from a model flattering
    // its own suggestion.
    const card = fn('ActionCard');
    expect(card).toContain('ex.projectName');
    expect(card).toContain('ex.title');
    expect(card).toContain('transfer.segmentLabel');
  });
});
