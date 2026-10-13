import { describe, expect, it } from 'vitest';
import { goalStage } from './format';

describe('goalStage verification honesty', () => {
  const ok = (extra: object) => ({ phase: 'complete' as const, reviewDecisions: { status: 'ok' as const, count: 2, truncated: false, ...extra } });
  it('a review record alone is not acceptance', () => { expect(goalStage(ok({}))).toBe('complete_review_record'); });
  it('an explicit accepted verdict is verified', () => { expect(goalStage(ok({ verdict: 'accepted' }))).toBe('verified'); });
  it('a null verdict is not verified', () => { expect(goalStage(ok({ verdict: null }))).toBe('complete_review_record'); });
  it('no review is complete_unreviewed', () => { expect(goalStage({ phase: 'complete', reviewDecisions: null })).toBe('complete_unreviewed'); });
});
