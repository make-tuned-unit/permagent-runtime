import { describe, expect, it } from 'vitest';
import { normalizeThemePref, type ThemePref } from './tokens';

describe('theme preferences', () => {
  it.each<ThemePref>(['dark', 'aurora', 'silver', 'system'])(
    'accepts the supported %s preference',
    preference => expect(normalizeThemePref(preference)).toBe(preference),
  );

  it('migrates the former slate preference to silver', () => {
    expect(normalizeThemePref('slate')).toBe('silver');
  });

  it('falls back safely for missing or invalid stored preferences', () => {
    expect(normalizeThemePref(null)).toBe('dark');
    expect(normalizeThemePref('unknown')).toBe('dark');
  });
});
