/**
 * One icon library, one drawn set.
 *
 * This is the gate, not the fix. Three strategies competed here:
 * `react-icons/fi` (Feather), which already dominated; `lucide-react`, which
 * was declared and imported by nothing; and seventeen files that drew their
 * own `<path>` — four of them whole path TABLES that nobody had counted.
 *
 * The cost of a second glyph system is not aesthetic. `sidebar/icons.ts` — now
 * `common/icons/` — exists because three library glyphs were indistinguishable
 * from each other at 18px in the rail, and people clicked the wrong tab. The
 * same failure was live in Settings until this landed: its Models glyph was a
 * byte-for-byte copy of its Activity glyph. Two ad-hoc sets drift into each
 * other; one set and one library cannot.
 *
 * The rule: icons come from `react-icons/fi`, or from `common/icons/` — the
 * one ratified hand-drawn set, which earned its exemption by naming the
 * legibility failure it fixes, testing at the real rendered size, and writing
 * the reason down. A raw `<svg>` in a view file is a drawing or it is a review
 * failure, and the list below is where you say which.
 *
 * TO ADD AN ENTRY: it has to be a drawing — something with no name a glyph
 * could carry. A chevron is not a drawing. Ask whether Feather would have a
 * word for it; if it would, use Feather's.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('../../..', import.meta.url));

/** The ratified local set. It renders its own paths; that is its whole job. */
const RATIFIED_SET = 'components/common/icons';

/**
 * Raw `<svg>` that is a DRAWING rather than an icon, with the count it draws.
 * Exact counts, so a glyph cannot hide behind a chart.
 */
const DRAWINGS: Record<string, { count: number; why: string }> = {
  'components/grow/GrowthSparkline.tsx': {
    count: 1,
    why: 'the cumulative helped/hindered polyline — a data graphic',
  },
  'components/projects/MarketPanel.tsx': {
    count: 1,
    why: 'the per-row market history series — a data graphic',
  },
  'components/finance/FinanceView.tsx': {
    count: 1,
    why: 'the holdings P&L sparkline and its fitted trend line',
  },
  'components/dashboard/Echo.tsx': {
    count: 1,
    why: 'the 132x46 red string of memory — an animated signature mark with a '
      + 'gradient and a self-drawing dash offset, not a glyph',
  },
  'components/settings/agents/AgentPortrait.tsx': {
    count: 1,
    why: 'generated identity art: a 64x64 per-agent silhouette built from a '
      + 'variant spec, different for every agent',
  },
  'components/settings/SettingsView.tsx': {
    count: 1,
    why: 'a QR code — a crispEdges module matrix, drawn from the payload',
  },
  'components/dashboard/Dashboard.tsx': {
    count: 1,
    why: 'the card resize grip: two parallel diagonal strokes, a decorative '
      + 'corner mark, and Feather has no resize handle',
  },
  'components/sidebar/Sidebar.tsx': {
    count: 1,
    why: "the ratified set's own render site — it draws the paths that set owns",
  },
  'components/dashboard/cards/cardIcons.tsx': {
    count: 1,
    why: 'the telemetry/weather set. Three of its fourteen — fog, '
      + 'partly-cloudy and a RAM stick — have no Feather counterpart at all, '
      + 'and six of the eight weather glyphs share one hand-drawn cloud body, '
      + 'so a partial conversion would put two different clouds in one card '
      + 'and leave a rump set of orphans with no stated reason. It is already '
      + 'shaped the way a local set should be: one named module, keyed by the '
      + "daemon's own names, with its rationale at the top",
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

function withoutComments(src: string): string {
  const blank = (m: string) => m.replace(/[^\n]/g, ' ');
  return src.replace(/\/\*[\s\S]*?\*\//g, blank).replace(/^[ \t]*\/\/.*$/gm, blank);
}

function svgCounts(): Map<string, number> {
  const counts = new Map<string, number>();
  for (const file of sourceFiles(SRC)) {
    const rel = relative(SRC, file);
    if (rel.startsWith(RATIFIED_SET)) continue;
    const n = (withoutComments(readFileSync(file, 'utf8')).match(/<svg[\s>]/g) ?? []).length;
    if (n > 0) counts.set(rel, n);
  }
  return counts;
}

describe('one icon system', () => {
  it('draws no hand-rolled glyph outside the one ratified set', () => {
    const offenders: string[] = [];
    for (const [rel, n] of svgCounts()) {
      const allowed = DRAWINGS[rel]?.count ?? 0;
      if (n !== allowed) offenders.push(`${rel}: ${n} <svg>, ${allowed} declared`);
    }
    expect(
      offenders,
      'use react-icons/fi. A drawing — a chart, generated art, a matrix — goes '
        + 'in DRAWINGS with its reason; a named glyph does not.',
    ).toEqual([]);
  });

  it('keeps every declared drawing to its count', () => {
    const counts = svgCounts();
    const wrong = Object.entries(DRAWINGS)
      .map(([rel, { count }]) => {
        const n = counts.get(rel) ?? 0;
        return n === count ? null : `${rel}: declares ${count}, found ${n}`;
      })
      .filter(Boolean);
    expect(wrong, 'a deleted drawing deletes its entry; a new glyph is not hidden behind one')
      .toEqual([]);
  });

  it('imports icons from exactly one library', () => {
    const libraries = new Set<string>();
    for (const file of sourceFiles(SRC)) {
      const src = readFileSync(file, 'utf8');
      for (const m of src.matchAll(/^import (type )?.*from '(react-icons[^']*|lucide-react[^']*)';$/gm)) {
        // `import type { IconType } from 'react-icons'` is the package's own
        // type root, not a second glyph set — it is how a component says "this
        // prop is a Feather icon".
        if (m[1] && m[2] === 'react-icons') continue;
        libraries.add(m[2]);
      }
    }
    expect(
      [...libraries].sort(),
      'Feather is the library (U2 §3.4). A second icon package is a second '
        + 'visual vocabulary and a second supply-chain surface.',
    ).toEqual(['react-icons/fi']);
  });
});
