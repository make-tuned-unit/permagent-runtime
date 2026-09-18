/**
 * The Orb's canvas is the one surface in the app that cannot read a CSS
 * variable, and it was the file that showed it: four rgb triplets typed by
 * hand, so the Orb wore the dark theme's brand colours on every theme —
 * including the pearl one, where a magenta halo over near-white is a different
 * picture entirely.
 *
 * The fix is a derivation, and a derivation is testable: every stop has to
 * trace back to a token, and switching the theme has to move the ramp.
 */
import { describe, expect, it } from 'vitest';

import { paletteStops } from './VoiceOrb';
import type { ThemeColors } from '../../styles/useTheme';

const channels = (hex: string): [number, number, number] => {
  const s = hex.replace('#', '');
  return [
    parseInt(s.substring(0, 2), 16),
    parseInt(s.substring(2, 4), 16),
    parseInt(s.substring(4, 6), 16),
  ];
};

/** Only the four fields the ramp derives from need to be real. */
const theme = (over: Partial<ThemeColors>): ThemeColors => ({
  cyan: '#00D5FF', purple: '#8D44AE', purpleBright: '#A855CC', text: '#FFFFFF',
  ...over,
} as ThemeColors);

describe('orb palette', () => {
  it('anchors its ends and its middle on real tokens', () => {
    const c = theme({});
    const stops = paletteStops(c);
    expect(stops).toHaveLength(4);
    expect(stops[0]).toEqual(channels(c.cyan));
    expect(stops[2]).toEqual(channels(c.purple));
  });

  it('keeps the derived blue step near the hand-written one it replaces', () => {
    // The literal was [64, 120, 255]. mix(cyan, purple, 0.45) lands at
    // [63, 148, 219] on the dark themes — same step in the ramp, now derived.
    const blue = paletteStops(theme({}))[1];
    expect(blue.map(Math.round)).toEqual([63, 148, 219]);
    // Close enough to the literal that this is a derivation, not a re-pick.
    const wasBlue = [64, 120, 255];
    for (let i = 0; i < 3; i++) expect(Math.abs(blue[i] - wasBlue[i])).toBeLessThan(40);
  });

  it('pulls the hot end toward the theme INK, so it lightens on dark and darkens on light', () => {
    const onDark = paletteStops(theme({ text: '#FFFFFF' }))[3];
    const onLight = paletteStops(theme({ text: '#1E2530' }))[3];
    const bright = channels('#A855CC');
    // Toward white on the void…
    expect(onDark[0]).toBeGreaterThan(bright[0]);
    // …and toward graphite on the pearl, where lighter would mean fainter.
    expect(onLight[0]).toBeLessThan(bright[0]);
  });

  it('moves when the theme moves', () => {
    const dark = paletteStops(theme({}));
    const silver = paletteStops(theme({
      cyan: '#00BFEF', purple: '#8B5CFF', purpleBright: '#9B6FFF', text: '#1E2530',
    }));
    expect(silver).not.toEqual(dark);
    expect(silver[0]).toEqual(channels('#00BFEF'));
  });
});
