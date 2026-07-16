// The load-bearing truth of the World View (bible §4): REAL agent state in →
// correct BODY out. resolvePose/resolveVisual are the pure extraction of
// AgentCharacterV2's per-frame decision — if these are right, an agent's
// posture and color always match the real signal it was handed.

import { describe, expect, it } from 'vitest';
import {
  resolvePose,
  resolveVisual,
  STATE_VISUALS,
  TENDING_VISUAL,
  TENDING_COLOR,
  type Engagement,
} from './poses';
import { STATE } from '../shared/palette';

describe('resolvePose — state→body mapping', () => {
  it('idle stands (idle pose) regardless of non-engaged engagement', () => {
    expect(resolvePose('idle', 'none')).toBe('idle');
    expect(resolvePose('idle', 'seated')).toBe('idle');
  });

  it('available is the alert stance', () => {
    expect(resolvePose('available', 'none')).toBe('available');
  });

  it('working seated at an anchor sits; standing leans; unseated reads alert', () => {
    // The distinction that keeps "working" honest: an agent still WALKING to
    // its seat (engaged 'none') must not already be hunched over a bench.
    expect(resolvePose('working', 'seated')).toBe('seatedWork');
    expect(resolvePose('working', 'standing')).toBe('standWork');
    expect(resolvePose('working', 'none')).toBe('available');
  });

  it('error is the slump wherever the agent stands', () => {
    expect(resolvePose('error', 'none')).toBe('error');
    expect(resolvePose('error', 'seated')).toBe('error');
    expect(resolvePose('error', 'tending')).toBe('error');
  });

  it('tending shows ONLY when the HUD makes no stronger claim (§4)', () => {
    // Tending is ambient site-work between tasks. Real working/error for the
    // user always takes the body — a tending agent must never mask a failure.
    expect(resolvePose('idle', 'tending')).toBe('tending');
    expect(resolvePose('available', 'tending')).toBe('tending');
    expect(resolvePose('working', 'tending')).toBe('available'); // working wins (no seat yet)
    expect(resolvePose('error', 'tending')).toBe('error'); // error wins
  });

  it('is total over every (state × engagement) pair', () => {
    const states = ['idle', 'available', 'working', 'error'] as const;
    const engagements: Engagement[] = ['none', 'seated', 'standing', 'tending'];
    for (const s of states) {
      for (const e of engagements) {
        expect(typeof resolvePose(s, e)).toBe('string');
      }
    }
  });
});

describe('resolveVisual — color register', () => {
  it('follows the HUD color for every non-tending pose (color LAW §2)', () => {
    expect(resolveVisual('idle', resolvePose('idle', 'none')).color).toBe(STATE.idle);
    expect(resolveVisual('available', resolvePose('available', 'none')).color).toBe(STATE.available);
    expect(resolveVisual('working', resolvePose('working', 'seated')).color).toBe(STATE.working);
    expect(resolveVisual('error', resolvePose('error', 'none')).color).toBe(STATE.error);
  });

  it('tending overrides HUD color with its warm-gray register, never amber', () => {
    const v = resolveVisual('idle', resolvePose('idle', 'tending'));
    expect(v).toBe(TENDING_VISUAL);
    expect(v.color).toBe(TENDING_COLOR);
    expect(v.color).not.toBe(STATE.working); // the whole point: tending ≠ amber
  });

  it('a working agent that is tending-engaged still reads amber (work wins)', () => {
    const pose = resolvePose('working', 'tending'); // → available (no seat yet)
    expect(resolveVisual('working', pose)).toBe(STATE_VISUALS.working);
  });
});
