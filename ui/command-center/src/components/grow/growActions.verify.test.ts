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
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SOURCE = readFileSync(join(__dirname, 'GrowView.tsx'), 'utf8');

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
    const card = fn('GrowActions');
    const open = card.indexOf('{a.artifact && (');
    expect(open, 'the artifact block moved or was renamed').toBeGreaterThan(-1);

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
