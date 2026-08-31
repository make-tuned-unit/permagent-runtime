/**
 * Four pedestals ring the World's centre and three of them navigate. The
 * fourth — the Lab — has no product tab behind it: it glides the camera for
 * about 700ms and lands nowhere. Structurally it is identical to the three
 * that arrive somewhere, so nothing on screen distinguished a destination from
 * a dead end until you had spent the glide finding out.
 *
 * WorldHUD still carries an amber branch keyed on "coming soon" in the tooltip
 * text, with nothing left in the app that produces that string — the warning
 * used to exist and was lost. This pins it back.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { STATIONS } from './constants';

describe('the Lab pedestal', () => {
  const lab = STATIONS.find(s => s.id === 'workbench');

  it('warns before the glide instead of after it', () => {
    expect(lab).toBeDefined();
    expect(lab!.tooltip).toContain('coming soon');
  });

  it('trips the HUD branch that has been waiting for it', () => {
    // WorldHUD tints the tooltip amber on exactly this substring. The two have
    // to keep agreeing, or the style check silently goes dead again.
    const hud = readFileSync(fileURLToPath(new URL('./WorldHUD.tsx', import.meta.url)), 'utf8');
    expect(hud).toContain("includes('coming soon')");
  });

  it('leaves the pedestals that do arrive somewhere unmarked', () => {
    for (const station of STATIONS) {
      if (station.id === 'workbench') continue;
      expect(station.tooltip, station.id).not.toContain('coming soon');
    }
  });
});
