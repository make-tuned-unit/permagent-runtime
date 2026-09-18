/**
 * No colour is written by hand on the settings surface, and no glass is worn
 * by it.
 *
 * Two rules, both from the design directive, both cheap to hold and expensive
 * to notice the absence of:
 *
 *   1. **Every colour comes from the theme.** A hex literal or an `rgba()` is
 *      a colour that cannot answer to a theme, and Permagent has three. The
 *      specific failure this catches is the silver theme: `rgba(255,255,255,
 *      0.05)` is a perfectly good hover fill on the void and is *invisible* on
 *      the pearl, so a control loses its hover state on one theme only, which
 *      nobody sees unless they were looking. `colors.fillHover` carries the
 *      theme's own ink and reads correctly on all three.
 *
 *   2. **No `backdrop-filter` anywhere here.** Settings is the CONTENT layer,
 *      and Apple's rule for it has no exceptions: *"Don't use Liquid Glass in
 *      the content layer."* Glass belongs to the floating control layer. The
 *      app-wide gate in `styles/backdropFilter.test.ts` owns this across the
 *      tree; this restates it for the surface, so a settings pane cannot
 *      acquire glass by being added to that file's debt register.
 *
 * `styles/textScale.test.ts` and `styles/radiusScale.test.ts` already cover
 * type and radii app-wide, so those are not repeated here.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../..', import.meta.url));
const SCANNED = ['components/settings', 'components/history'];

/**
 * The one place on this surface where a colour is not a token, with the reason.
 *
 * A QR symbol is read by a camera hunting maximum luminance contrast. Themed
 * inks — even the near-black and near-white ones — cut the contrast ratio
 * enough that phones fail to lock on, and a pairing code that will not scan is
 * a broken feature, not a themed one. Pure `#fff` / `#000`, in every theme.
 */
const EXEMPT: Record<string, { colors: readonly string[]; why: string }> = {
  'components/settings/SettingsView.tsx': {
    colors: [
      // The pairing QR code.
      '#fff', '#000',
      // The four theme swatches on Appearance. Each one DEPICTS a theme's own
      // palette and has to keep depicting it while you are looking at a
      // different theme — a swatch drawn from `useTheme().colors` would render
      // four identical rectangles in whichever theme is active, which is the
      // one thing the control exists not to do. These are the literal values in
      // `tokens.ts`'s own theme gradients.
      '#F8FAFC', '#D8DEE8', '#0B1220', '#1E2433', '#8D44AE', '#00BFEF', '#8B5CFF',
    ],
    why: 'the pairing QR code (a camera needs literal black on literal white to scan) and the four theme swatches, which must depict each theme regardless of the active one',
  },
};

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules') continue;
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) { sourceFiles(full, out); continue; }
    if (!/\.tsx?$/.test(entry)) continue;
    if (/\.(test|spec)\.tsx?$/.test(entry)) continue;
    out.push(full);
  }
  return out;
}

function scanned(): { rel: string; source: string }[] {
  const out: { rel: string; source: string }[] = [];
  for (const root of SCANNED) {
    for (const file of sourceFiles(join(SRC, ...root.split('/')))) {
      out.push({ rel: relative(SRC, file).split(sep).join('/'), source: readFileSync(file, 'utf8') });
    }
  }
  return out;
}

const HEX = /#([0-9a-fA-F]{3,8})\b/g;
const RGBA = /\brgba?\(\s*\d/g;

/**
 * Each line of a source file with its comments removed, so what is left is
 * only what the browser sees.
 *
 * This has to be stateful rather than line-shaped. `#167` and `#628` are issue
 * numbers, this codebase cites them constantly, and they cite them in the
 * MIDDLE of block comments — a rule that only recognises a comment by how its
 * line starts misses the second line of every `/* … *\/`. A gate that fires on
 * prose is a gate somebody turns off, so the block state is tracked properly.
 *
 * Strings are not tracked, deliberately: a `#` inside a string literal is
 * exactly what this gate is hunting for.
 */
export function codeLines(source: string): string[] {
  const out: string[] = [];
  let inBlock = false;
  for (const raw of source.split('\n')) {
    let line = raw;
    let code = '';
    while (line.length > 0) {
      if (inBlock) {
        const close = line.indexOf('*/');
        if (close === -1) { line = ''; break; }
        inBlock = false;
        line = line.slice(close + 2);
        continue;
      }
      const open = line.indexOf('/*');
      const lineComment = line.indexOf('//');
      if (lineComment !== -1 && (open === -1 || lineComment < open)) {
        code += line.slice(0, lineComment);
        line = '';
        break;
      }
      if (open === -1) { code += line; line = ''; break; }
      code += line.slice(0, open);
      inBlock = true;
      line = line.slice(open + 2);
    }
    out.push(code);
  }
  return out;
}

describe('the settings palette', () => {
  it('writes no colour by hand', () => {
    const offenders: string[] = [];
    for (const { rel, source } of scanned()) {
      const exempt = EXEMPT[rel]?.colors ?? [];
      codeLines(source).forEach((line, i) => {
        for (const m of line.matchAll(HEX)) {
          if (exempt.includes(`#${m[1]}`)) continue;
          offenders.push(`${rel}:${i + 1}  #${m[1]}`);
        }
        for (const m of line.matchAll(RGBA)) {
          offenders.push(`${rel}:${i + 1}  ${m[0]}…`);
        }
      });
    }
    expect(
      offenders,
      'use a token from useTheme().colors — a literal cannot answer to the three themes, '
        + 'and white-on-white is invisible on silver',
    ).toEqual([]);
  });

  it('wears no glass, because it is the content layer', () => {
    const offenders: string[] = [];
    for (const { rel, source } of scanned()) {
      codeLines(source).forEach((line, i) => {
        if (/(backdropFilter|WebkitBackdropFilter|["']?backdrop-filter["']?)\s*:/.test(line)
          || /\bbackdrop-blur\b/.test(line)) {
          offenders.push(`${rel}:${i + 1}  ${line.trim()}`);
        }
      });
    }
    expect(
      offenders,
      'Settings is content and content is opaque (HIG: "Don\'t use Liquid Glass in the content layer"). '
        + 'If a surface needs to feel raised, change its fill or its spacing — not its transparency.',
    ).toEqual([]);
  });

  it('names a reason for every exemption, and keeps none that is unused', () => {
    for (const [rel, { colors, why }] of Object.entries(EXEMPT)) {
      expect(why.length, `${rel}: an exemption without a reason is a hole`).toBeGreaterThan(20);
      const file = scanned().find(f => f.rel === rel);
      expect(file, `${rel} is exempted but not scanned — stale entry`).toBeTruthy();
      for (const c of colors) {
        const used = codeLines(file!.source).some(l => l.includes(c));
        expect(used, `${rel} no longer uses ${c} — delete the exemption`).toBe(true);
      }
    }
  });
});
