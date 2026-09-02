/**
 * One module owns glass.
 *
 * `backdrop-filter` had spread to ~30 files with values from `blur(4px)` to
 * `blur(24px) saturate(140%)`, and most of them were not glass at all: they
 * paired the filter with an OPAQUE `colors.surface`, so the blur was painted
 * over by the fill on the very next composite. Inert, and not free — each one
 * still forced its own full compositing pass, because the browser cannot batch
 * filter layers.
 *
 * So this gate is two rules at once, which is why it is worth having:
 *   - a correctness rule (glass belongs to the floating control layer; content
 *     surfaces are opaque), and
 *   - a performance ceiling (a handful of glass surfaces is the budget).
 *
 * `components/common/Glass.tsx` and the `GlassSurface` tokens are the only
 * places allowed to name the property. Everything else uses `<Glass>` or
 * `glassSurface()`, or is opaque.
 */

import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = fileURLToPath(new URL('..', import.meta.url));

/** The property-assignment form only — prose about `backdropFilter` is fine. */
const USE = /(backdropFilter|WebkitBackdropFilter|["']?backdrop-filter["']?)\s*:/gi;

/** The two modules that define the material. */
const OWNERS = ['styles/tokens.ts', 'components/common/Glass.tsx'];

/**
 * Hand-rolled glass that is NOT this lane's to migrate, frozen at its current
 * count. Each entry names the screen lane that owns the file: that lane
 * converts these to the tokens when it redesigns its screen, and drops the
 * entry. Shrinking a count passes; growing one fails, which is the asymmetry
 * that matters — the list can only get shorter.
 *
 * Nothing here is endorsed. It is a debt register with names on it.
 *
 * R16 (world HUD chrome) paid off HudShell, WorldView (Agora return),
 * WorldHUD, AgentPicker, AgentCharacterV2 nameplate, and CanvasLegend —
 * those entries are gone. Remaining world/** glass belongs to other surfaces.
 */
const LANE_OWNED: Record<string, { max: number; lane: string }> = {
  // Brain's cluster (was 8, including its own local `glass` object) is GONE:
  // R13 converted BrainView.tsx and BrainList.tsx to `glassSurface()` / the
  // theme's `glass` tokens as part of the screen's Liquid Glass pass.
  // World HUD chrome (HudShell, WorldView, WorldHUD, AgentPicker,
  // AgentCharacterV2 nameplate) is GONE: R16 put the HUD panels on the glass
  // tokens — the one place glass over content is correct, the canvas being the
  // content. CanvasLegend's two are GONE with it: the shared canvas overlay
  // legend draws on the glass tokens, so it no longer needs a hand-rolled entry.
  // PersonDetailModal's two are GONE (R12): the drawer was a hand-rolled
  // second modal, and it is a `DetailModal placement="contained"` now — a
  // modal body is content by Apple's own list, so it is simply opaque.
  'components/chat/SessionPicker.tsx': { max: 1, lane: "R4' chat dock/launcher" },
  // MeetingRecorder's two are GONE (R4' voice surfaces): the floating panel
  // takes `glassSurface()`, and the picker modal — a modal BODY, which is
  // content by Apple's own list — is simply opaque, over the theme's `veil`.
  //
  // R16 paid off world HUD chrome + CanvasLegend (HudShell, WorldHUD,
  // AgentPicker, WorldView Agora return, AgentCharacterV2 nameplate).
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

function uses(): Map<string, number> {
  const found = new Map<string, number>();
  for (const file of sourceFiles(SRC)) {
    const n = (readFileSync(file, 'utf8').match(USE) ?? []).length;
    if (n > 0) found.set(file.slice(SRC.length).split(sep).join('/'), n);
  }
  return found;
}

describe('backdrop-filter', () => {
  it('is written only by the Glass primitive and its tokens', () => {
    const strays = [...uses().keys()]
      .filter(f => !OWNERS.includes(f) && !(f in LANE_OWNED))
      .sort();

    expect(
      strays,
      'use <Glass>, spread glassSurface(), or make the surface opaque — a bare '
      + 'backdropFilter over an opaque fill blurs nothing and still costs a pass',
    ).toEqual([]);
  });

  it('does not let a lane-owned cluster grow while it waits its turn', () => {
    const found = uses();
    const grown = Object.entries(LANE_OWNED)
      .map(([file, { max, lane }]) => ({ file, lane, max, now: found.get(file) ?? 0 }))
      .filter(e => e.now > e.max)
      .map(e => `${e.file}: ${e.now} > ${e.max} (${e.lane})`);

    expect(grown, 'this file is waiting on a screen lane — do not add more glass to it').toEqual([]);
  });

  it('names a real owning lane for every entry it excuses', () => {
    // A debt register whose entries have no owner is just a suppression list.
    for (const [file, { lane }] of Object.entries(LANE_OWNED)) {
      expect(lane, `${file} needs an owning lane`).toMatch(/\w/);
    }
  });
});
