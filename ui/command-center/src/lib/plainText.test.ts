import { describe, expect, it } from 'vitest';
import { plainText } from './plainText';

describe('plainText', () => {
  it('drops markdown markers and collapses whitespace', () => {
    expect(plainText('Report\n## Findings\n- **[HIGH] NEW** `lodash` [docs](http://x)')).toBe('Report Findings - [HIGH] NEW lodash docs');
  });
});
