/**
 * R14 browser chrome — design gates + webview-rect freeze.
 *
 * Gate 1: no hardcoded colours / radii / shadows in chrome files.
 * Gate 2/7: glass only via useGlass; content stays opaque; no backdropFilter.
 * Gate 6: syncBounds body is byte-identical to the pre-chrome baseline hash,
 *          and CHROME_GEOM holds the padding values that set the leftover.
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { concentric, radius, space, getThemedColors, setTheme } from '../../styles/tokens';
import {
  ADDRESS_RADIUS,
  CHIP_RADIUS,
  CHROME_GEOM,
  chromeBareVars,
  dangerWash,
} from './browserChrome';

const DIR = new URL('.', import.meta.url);
const CHROME_FILES = [
  'Browser.tsx',
  'BrowserTabs.tsx',
  'BookmarksBar.tsx',
  'browserChrome.ts',
] as const;

function read(name: string): string {
  return readFileSync(new URL(name, DIR), 'utf8');
}

/** Strip block + line comments so issue refs like `#790` are not colour hits. */
function withoutComments(src: string): string {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

describe('CHROME_GEOM freezes the paddings that set the webview leftover', () => {
  it('matches the pre-glass Tailwind values (px-3/py-2/py-1/gap-2/gap-1/gap-3)', () => {
    expect(CHROME_GEOM.toolbarPadY).toBe(8);
    expect(CHROME_GEOM.toolbarPadX).toBe(12);
    expect(CHROME_GEOM.toolbarGap).toBe(8);
    expect(CHROME_GEOM.bookmarksPadY).toBe(4);
    expect(CHROME_GEOM.bookmarksPadX).toBe(12);
    expect(CHROME_GEOM.bookmarksGap).toBe(4);
    expect(CHROME_GEOM.statusPadY).toBe(4);
    expect(CHROME_GEOM.statusPadX).toBe(12);
    expect(CHROME_GEOM.statusGap).toBe(12);
    expect(CHROME_GEOM.tabPadY).toBe(6);
    expect(CHROME_GEOM.tabPadX).toBe(12);
    expect(CHROME_GEOM.navIconPad).toBe(6);
    expect(CHROME_GEOM.chipPadY).toBe(2);
    expect(CHROME_GEOM.chipPadX).toBe(8);
  });

  it('is wired from the space scale (except the half-step chip pad)', () => {
    expect(CHROME_GEOM.toolbarPadY).toBe(space.md);
    expect(CHROME_GEOM.toolbarPadX).toBe(space.xl);
    expect(CHROME_GEOM.bookmarksPadY).toBe(space.xs);
    expect(CHROME_GEOM.statusGap).toBe(space.xl);
  });
});

describe('syncBounds behaviour is frozen (gate 6)', () => {
  it('keeps the syncBounds body byte-identical to the pre-R14 hash', () => {
    // Captured 2026-09-02 before any chrome restyle. A layout-affecting edit
    // that also rewrote syncBounds would fail here; a chrome-only edit that
    // leaves this region alone passes.
    const src = read('Browser.tsx');
    const body = src.slice(
      src.indexOf('const syncBounds = useCallback'),
      src.indexOf('syncBoundsRef.current = syncBounds'),
    );
    const hash = createHash('sha256').update(body).digest('hex');
    expect(hash).toBe('ba53557a82ccb075a8e9cf90ee575d8dc96db19bad5b9e207ea7baf2140e151e');
  });

  it('still measures containerRef (flex-1 content), not a chrome wrapper', () => {
    const src = read('Browser.tsx');
    expect(src).toMatch(/ref=\{containerRef\}[^>]*className="flex-1 min-h-0 relative"/);
    const sync = src.slice(
      src.indexOf('const syncBounds = useCallback'),
      src.indexOf('syncBoundsRef.current = syncBounds'),
    );
    expect(sync).toContain('containerRef.current.getBoundingClientRect()');
    expect(sync).toContain('update_browser_bounds');
  });

  it('does not put padding on containerRef', () => {
    const src = read('Browser.tsx');
    const marker = 'ref={containerRef}';
    const i = src.indexOf(marker);
    const tag = src.slice(i, i + 180);
    expect(tag).not.toMatch(/padding/);
    expect(tag).toContain('flex-1 min-h-0 relative');
  });
});

describe('glass language on chrome only (gates 1–4, 7)', () => {
  it('uses useGlass for chrome material and never names backdropFilter', () => {
    const browser = read('Browser.tsx');
    expect(browser).toContain("useGlass('glass')");
    expect(browser).toContain('chromeGlass');
    for (const name of CHROME_FILES) {
      const bare = withoutComments(read(name));
      expect(bare, name).not.toMatch(/backdropFilter\s*:/);
      expect(bare, name).not.toMatch(/WebkitBackdropFilter\s*:/);
      expect(bare, name).not.toMatch(/backdrop-filter\s*:/);
    }
  });

  it('keeps the page content layer opaque', () => {
    const browser = read('Browser.tsx');
    // Content div explicitly paints the theme bg (opaque), not glass.
    expect(browser).toMatch(/ref=\{containerRef\}[\s\S]{0,120}backgroundColor: colors\.bg/);
  });

  it('has zero hardcoded colours / rgba / boxShadow literals in chrome sources', () => {
    for (const name of CHROME_FILES) {
      const bare = withoutComments(read(name));
      expect(bare.match(/#[0-9a-fA-F]{3,8}\b/) ?? [], name).toEqual([]);
      expect(bare.match(/rgba?\(/) ?? [], name).toEqual([]);
      expect(bare.match(/boxShadow:\s*['"`]/) ?? [], name).toEqual([]);
      // Raw radius literals like `borderRadius: 8` — tokenised form only.
      expect(bare.match(/borderRadius:\s*\d+/) ?? [], name).toEqual([]);
    }
  });

  it('nests chip radius concentrically under the address field (D4)', () => {
    expect(ADDRESS_RADIUS).toBe(radius.sm);
    expect(CHIP_RADIUS).toBe(concentric(ADDRESS_RADIUS, CHROME_GEOM.chipPadY));
    expect(CHIP_RADIUS).toBe(4);
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
    for (const name of ['BrowserTabs.tsx', 'BookmarksBar.tsx'] as const) {
      expect(withoutComments(read(name))).not.toMatch(/red-\d+|amber-\d+|bg-white\//);
    }
  });

  it('saved-tabs menu is opaque elevated, not a second glass (D2)', () => {
    const src = read('BookmarksBar.tsx');
    expect(src).toContain('colors.elevationOverlay');
    expect(src).toContain('colors.surface');
    expect(src).not.toContain('useGlass');
  });

  it('keeps native title= tooltips until R3 ships a shared primitive', () => {
    // No Tooltip import from components/common — that lane owns the primitive.
    for (const name of CHROME_FILES) {
      expect(read(name)).not.toMatch(/from ['\"]\.\.\/common\/.*Tooltip/);
    }
    expect(read('Browser.tsx')).toContain('title="Back"');
    expect(read('BrowserTabs.tsx')).toContain('title="New tab (Cmd+T)"');
  });
});
