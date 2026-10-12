import { describe, expect, it } from 'vitest';
import { clampMaskedSecret } from './api';

describe('clampMaskedSecret', () => {
  it('keeps at most the first four characters of the daemon mask', () => {
    expect(clampMaskedSecret('01a03932****************')).toBe('01a0********************'.slice(0, 4) + '*'.repeat(20));
    expect(clampMaskedSecret('01a03932****')).toHaveLength(12);
  });
  it('leaves other mask shapes (ellipsis, trailing characters) alone', () => {
    expect(clampMaskedSecret('sk-…9f2')).toBe('sk-…9f2');
  });
  it('leaves short or empty masks alone', () => {
    expect(clampMaskedSecret('abcd****')).toBe('abcd****');
    expect(clampMaskedSecret('')).toBe('');
    expect(clampMaskedSecret(undefined)).toBeUndefined();
  });
});
