import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { getXtermTheme } from './xtermTheme';

const TERMINAL_TSX = readFileSync(new URL('./Terminal.tsx', import.meta.url), 'utf8');
const THEME_TS = readFileSync(new URL('./xtermTheme.ts', import.meta.url), 'utf8');

describe('xterm color contract', () => {
  it('keeps minimumContrastRatio at 1 so TUI palettes are not washed to the default fg', () => {
    expect(TERMINAL_TSX).toMatch(/minimumContrastRatio:\s*1/);
  });

  it('uses the Permagent cyan token for dark-theme ANSI cyan', () => {
    expect(THEME_TS).toContain("cyan: '#00D5FF'");
    expect(typeof getXtermTheme).toBe('function');
  });
});
