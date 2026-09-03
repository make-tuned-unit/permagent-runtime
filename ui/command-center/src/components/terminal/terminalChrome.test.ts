/**
 * R15 terminal chrome — design gates + PTY-rect freeze.
 *
 * Gate 1: no hardcoded colours / radii / shadows in chrome files.
 * Gate 2/7: glass only via useGlass; content stays opaque; no backdropFilter.
 * Gate 6: PTY attach/setup + terminalReattach are byte-identical to pre-chrome,
 *          and CHROME_GEOM holds the paddings that set the leftover.
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { concentric, radius, space, getThemedColors, setTheme } from '../../styles/tokens';
import {
  CHIP_BTN_RADIUS,
  CHIP_RADIUS,
  CHROME_GEOM,
  CHROME_RADIUS,
  DROP_RADIUS,
  chromeBareVars,
  dangerWash,
} from './terminalChrome';

const DIR = new URL('.', import.meta.url);
const CHROME_FILES = [
  'TerminalManager.tsx',
  'Terminal.tsx',
  'terminalChrome.ts',
] as const;

function read(name: string): string {
  return readFileSync(new URL(name, DIR), 'utf8');
}

/** Strip block + line comments so issue refs like `#557` are not colour hits. */
function withoutComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

describe('CHROME_GEOM freezes the paddings that set the PTY leftover', () => {
  it('matches the pre-glass Tailwind / inline values', () => {
    expect(CHROME_GEOM.tabPadY).toBe(6);
    expect(CHROME_GEOM.tabPadX).toBe(12);
    expect(CHROME_GEOM.tabGap).toBe(6);
    expect(CHROME_GEOM.railPadY).toBe(6);
    expect(CHROME_GEOM.popOutPadX).toBe(8);
    expect(CHROME_GEOM.newTabPadX).toBe(10);
    expect(CHROME_GEOM.dropMargin).toBe(8);
    expect(CHROME_GEOM.chipInset).toBe(8);
    expect(CHROME_GEOM.chipPadY).toBe(8);
    expect(CHROME_GEOM.chipPadX).toBe(12);
    expect(CHROME_GEOM.chipBtnPadY).toBe(4);
    expect(CHROME_GEOM.chipBtnPadX).toBe(8);
    expect(CHROME_GEOM.chipBtnGap).toBe(4);
  });

  it('is wired from the space scale', () => {
    expect(CHROME_GEOM.tabPadY).toBe(space.sm);
    expect(CHROME_GEOM.tabPadX).toBe(space.xl);
    expect(CHROME_GEOM.dropMargin).toBe(space.md);
    expect(CHROME_GEOM.chipBtnPadY).toBe(space.xs);
  });
});

describe('PTY attach / reattach behaviour is frozen (gate 6)', () => {
  it('keeps terminalReattach.ts byte-identical to the pre-R15 hash', () => {
    // Captured 2026-09-02 before any chrome restyle. Do not touch reattach /
    // minimize-recover logic from a visual lane.
    const src = read('terminalReattach.ts');
    const hash = createHash('sha256').update(src).digest('hex');
    expect(hash).toBe('95f5e668b243efea5229ecfbe0398a7d8e6ce285c9a3ddc989ea7c3b235addf9');
  });

  it('keeps the Terminal mount/setup effect (attach + fit + listeners) byte-identical', () => {
    const src = read('Terminal.tsx');
    const body = src.slice(
      src.indexOf('const term = new XTerm({'),
      src.indexOf('}, []); // Mount once'),
    );
    const hash = createHash('sha256').update(body).digest('hex');
    expect(hash).toBe('bd440f3e73bb584a7ef5b7c77b799b208e074ac5ffcdc6409c3c4af723214faa');
  });

  it('still measures the opaque pty-terminal container (no padding on it)', () => {
    const src = read('Terminal.tsx');
    const marker = 'ref={containerRef}';
    const i = src.indexOf(marker);
    const tag = src.slice(i, i + 160);
    expect(tag).toContain('className="pty-terminal h-full w-full"');
    expect(tag).toContain('backgroundColor: xtermBg');
    expect(tag).not.toMatch(/padding/);
    expect(tag).not.toMatch(/backdropFilter/);
  });

  it('keeps the flex-1 content leftover wrapper (no padding)', () => {
    const src = read('TerminalManager.tsx');
    expect(src).toMatch(/className="flex-1 min-h-0 relative"/);
    const i = src.indexOf('className="flex-1 min-h-0 relative"');
    const tag = src.slice(i, i + 120);
    expect(tag).not.toMatch(/padding/);
  });
});

describe('glass language on chrome only (gates 1–4, 7)', () => {
  it('uses useGlass for chrome material and never names backdropFilter', () => {
    const manager = read('TerminalManager.tsx');
    expect(manager).toContain("useGlass('glass')");
    expect(manager).toContain('chromeGlass');
    for (const name of CHROME_FILES) {
      const bare = withoutComments(read(name));
      expect(bare, name).not.toMatch(/backdropFilter\s*:/);
      expect(bare, name).not.toMatch(/WebkitBackdropFilter\s*:/);
      expect(bare, name).not.toMatch(/backdrop-filter\s*:/);
    }
  });

  it('keeps the PTY content layer opaque', () => {
    const term = read('Terminal.tsx');
    expect(term).toMatch(/ref=\{containerRef\}[\s\S]{0,120}backgroundColor: xtermBg/);
    const manager = read('TerminalManager.tsx');
    expect(manager).toMatch(/flex-1 min-h-0 relative"[\s\S]{0,80}backgroundColor: colors\.bg/);
  });

  it('has zero hardcoded colours / rgba / boxShadow literals in chrome sources', () => {
    for (const name of CHROME_FILES) {
      const bare = withoutComments(read(name));
      expect(bare.match(/#[0-9a-fA-F]{3,8}\b/) ?? [], name).toEqual([]);
      expect(bare.match(/rgba?\(/) ?? [], name).toEqual([]);
      expect(bare.match(/boxShadow:\s*['"`]/) ?? [], name).toEqual([]);
      expect(bare.match(/borderRadius:\s*\d+/) ?? [], name).toEqual([]);
    }
  });

  it('nests chrome + chip radii concentrically (D4)', () => {
    expect(CHROME_RADIUS).toBe(concentric(radius.glass, radius.glass));
    expect(DROP_RADIUS).toBe(radius.lg);
    expect(CHIP_RADIUS).toBe(radius.md);
    expect(CHIP_BTN_RADIUS).toBe(concentric(CHIP_RADIUS, CHROME_GEOM.chipBtnPadY));
    expect(CHIP_BTN_RADIUS).toBe(4);
  });

  it('chromeBareVars feeds fillHover/fillActive (D10)', () => {
    setTheme('dark');
    const colors = getThemedColors();
    const vars = chromeBareVars(colors) as Record<string, string>;
    expect(vars['--pa-btn-bg-hover']).toBe(colors.fillHover);
    expect(vars['--pa-btn-bg-active']).toBe(colors.fillActive);
  });

  it('dangerWash is a theme-derived tint, not a Tailwind red', () => {
    setTheme('dark');
    const colors = getThemedColors();
    expect(dangerWash(colors)).toBe(`${colors.danger}33`);
    expect(withoutComments(read('TerminalManager.tsx'))).not.toMatch(/red-\d+|bg-white\/|text-dark-/);
  });

  it('pending-prompt chip is opaque elevated, not a second glass (D2)', () => {
    const src = read('Terminal.tsx');
    expect(src).toContain('colors.elevationFloating');
    expect(src).toContain('colors.surfaceHi');
    expect(src).not.toContain('useGlass');
  });

  it('uses the shared Tooltip primitive on chrome tip controls', () => {
    expect(read('TerminalManager.tsx')).toMatch(/from ['"]\.\.\/common\/Tooltip['"]/);
    expect(read('TerminalManager.tsx')).toContain('Tooltip content="New terminal (Cmd+T)"');
    expect(read('TerminalManager.tsx')).toContain('Tooltip content="Pop out active terminal"');
    expect(read('TerminalManager.tsx')).not.toMatch(/\btitle="New terminal|\btitle="Pop out/);
  });
});
