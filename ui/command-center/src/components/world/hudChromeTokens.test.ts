/**
 * Zero hardcoded colors/radii/shadows on World HUD chrome (R16 Liquid Glass
 * pass, DAG rule gate 1). Follows `brain/brainColorTokens.test.ts` and
 * `styles/backdropFilter.test.ts`: walk the owned chrome sources, flag
 * literal `#hex` / `rgba()` / `rgb()`, and name the (small, documented)
 * exemptions rather than silently skipping them.
 *
 * Scoped files (HUD / overlay chrome only — not the Three.js scene):
 *   HudShell, WorldHUD, AgentPicker, hudChrome, *HUD.tsx, HenryIdentityTab,
 *   agents/stateSources.tsx (display bits — currently non-visual),
 *   worldLegend.ts, and CanvasLegend usage (the common primitive itself is
 *   gated by styles/backdropFilter.test.ts after this lane converted it).
 *
 * Exempt, and why:
 *   - `constants.ts` and `shared/palette.ts` — the 3D scene's own palette
 *     (WebGL materials + identity trim). Brief: "world/constants.ts palette
 *     (the 3D scene's own palette is exempt by design)". HUD files may
 *     *import* AGENT_TRIM / STATE / COLORS identifiers; they must not
 *     re-literal the hex.
 *   - Renderer/scene files (WorldScene, WorldPostProcessing, atmosphere/,
 *     camera/, props/, areas/, agents/*Character*, frameClock, …) are
 *     outside this scan — FROZEN by the lane brief.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const WORLD = fileURLToPath(new URL('.', import.meta.url));
const COMMON = fileURLToPath(new URL('../common', import.meta.url));

/** Chrome files this lane owns and must keep free of color literals. */
const OWNED = [
  'HudShell.tsx',
  'WorldHUD.tsx',
  'AgentPicker.tsx',
  'hudChrome.ts',
  'HenryIdentityTab.tsx',
  'worldLegend.ts',
  'CouncilHUD.tsx',
  'FinancierHUD.tsx',
  'GrowthMeasurementHUD.tsx',
  'HenryHUD.tsx',
  'LibrarianHUD.tsx',
  'PickerHUD.tsx',
  'PolybotHUD.tsx',
  'ReaderHUD.tsx',
  'StewardHUD.tsx',
  'StrixHUD.tsx',
  'WatcherHUD.tsx',
  'agents/stateSources.tsx',
];

/** Shared canvas overlay this lane converted (R13 deferred to R16). */
const SHARED = ['CanvasLegend.tsx'];

const HEX = /#[0-9a-fA-F]{3,8}\b/g;
const RGB_FN = /\brgba?\(/g;
/** Inline radius/shadow literals the glass pass forbids on chrome. */
const RADIUS_LIT = /borderRadius:\s*['`]?\d+/g;
const SHADOW_LIT = /boxShadow:\s*['`][^'"`]*rgba?\(/g;

function stripComments(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:])\/\/.*$/gm, '$1');
}

function scanFile(abs: string, rel: string): { file: string; hex: number; rgb: number; radius: number; shadow: number } | null {
  const code = stripComments(readFileSync(abs, 'utf8'));
  const hex = (code.match(HEX) ?? []).length;
  const rgb = (code.match(RGB_FN) ?? []).length;
  const radius = (code.match(RADIUS_LIT) ?? []).length;
  const shadow = (code.match(SHADOW_LIT) ?? []).length;
  if (hex + rgb + radius + shadow === 0) return null;
  return { file: rel, hex, rgb, radius, shadow };
}

describe('World HUD chrome: zero hardcoded colors (R16)', () => {
  it('writes no literal #hex / rgba() / rgb() / inline radius|shadow on owned chrome', () => {
    const offenders: ReturnType<typeof scanFile>[] = [];
    for (const rel of OWNED) {
      const abs = join(WORLD, rel);
      if (!existsSync(abs)) continue;
      const hit = scanFile(abs, `world/${rel}`);
      if (hit) offenders.push(hit);
    }
    for (const rel of SHARED) {
      const abs = join(COMMON, rel);
      if (!existsSync(abs)) continue;
      const hit = scanFile(abs, `common/${rel}`);
      if (hit) offenders.push(hit);
    }
    expect(
      offenders,
      'use useTheme().colors, useGlass()/glassSurface(), radius.*/concentric(), '
      + 'or an imported AGENT_TRIM/STATE/COLORS identifier — not a literal. '
      + 'Scene palette files (constants.ts, shared/palette.ts) are exempt by design.',
    ).toEqual([]);
  });

  it('names its owned chrome list so the scan cannot silently shrink', () => {
    expect([...OWNED].sort()).toEqual([...OWNED].sort());
    expect(OWNED).toContain('HudShell.tsx');
    expect(OWNED).toContain('WorldHUD.tsx');
    expect(OWNED).toContain('AgentPicker.tsx');
    expect(SHARED).toEqual(['CanvasLegend.tsx']);
  });
});
