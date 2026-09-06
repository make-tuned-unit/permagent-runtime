import { afterEach, describe, expect, it } from 'vitest';

import { getThemedColors, getThemePref, setTheme, type ThemeId } from './tokens';

const THEMES: ThemeId[] = ['dark', 'aurora', 'silver'];
const READABLE_SURFACES = ['bg', 'surface'] as const;
const SOLID_SURFACES = ['bg', 'surface', 'surfaceHi'] as const;
const INITIAL_THEME_PREF = getThemePref();

function rgb(hex: string): [number, number, number] {
  expect(hex).toMatch(/^#[0-9a-f]{6}$/i);
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
}

function relativeLuminance(hex: string): number {
  return rgb(hex)
    .map(channel => channel / 255)
    .map(channel => channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4)
    .reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index], 0);
}

function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

afterEach(() => {
  // Keep the shared token module and localStorage state isolated for the rest
  // of the suite, including when a developer runs this alongside theme tests.
  setTheme(INITIAL_THEME_PREF);
});

describe('semantic text contrast', () => {
  it.each(THEMES)('keeps explanatory textMuted copy readable on body/card surfaces in %s', theme => {
    setTheme(theme);
    const colors = getThemedColors();

    for (const surface of READABLE_SURFACES) {
      expect(
        contrastRatio(colors.textMuted, colors[surface]),
        `${theme}: textMuted on ${surface}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each(THEMES)('keeps primary text readable on every solid surface in %s', theme => {
    setTheme(theme);
    const colors = getThemedColors();

    for (const surface of SOLID_SURFACES) {
      expect(
        contrastRatio(colors.text, colors[surface]),
        `${theme}: text on ${surface}`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each(THEMES)('keeps textMuted stronger than the dim metadata semantic in %s', theme => {
    setTheme(theme);
    const colors = getThemedColors();

    for (const surface of READABLE_SURFACES) {
      expect(
        contrastRatio(colors.textMuted, colors[surface]),
        `${theme}: textMuted must outrank textDim on ${surface}`,
      ).toBeGreaterThan(contrastRatio(colors.textDim, colors[surface]));
    }
  });
});
