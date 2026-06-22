// Zone registry — WORLD_VIEW_BIBLE.md §3 area map. W1-owned.
// Five zones off the rotunda; thresholds punched through the colonnade.
// Display labels follow the naming law: product tab names EXACTLY where one
// exists (Build, Brain, Automate, Mesh); the Lab keeps "Lab".

import { ENV } from '../shared/palette';
import type { AreaId } from '../shared/anchors';

export type ZoneId = Exclude<AreaId, 'hall'>;

export interface ZoneDef {
  id: ZoneId;
  /** Display label — naming law (§3). */
  label: string;
  /** World angle of the threshold direction from hall center (radians). */
  angle: number;
  /** Distance from hall center to the zone room center (1u = 1m). */
  centerDist: number;
  /** The zone's accent color (floor seam, landmark emissive). */
  accent: string;
}

export const ZONES: ZoneDef[] = [
  { id: 'build',       label: 'Build',    angle: 0,                 centerDist: 24,   accent: ENV.neonAmber },
  { id: 'brain',       label: 'Brain',    angle: Math.PI / 2,       centerDist: 24,   accent: ENV.violet },
  { id: 'lab',         label: 'Lab',      angle: -Math.PI / 2,      centerDist: 24,   accent: ENV.neonCyan },
  { id: 'automate',    label: 'Automate', angle: Math.PI,           centerDist: 24,   accent: ENV.bronze },
  { id: 'antechamber', label: 'Mesh',     angle: (Math.PI * 5) / 4, centerDist: 26.9, accent: ENV.horizonBlue },
];

/**
 * Colonnade angles whose column is punched out. The satellite zone thresholds
 * were removed (the rooms behind them are gone — the direction is to grow the
 * rotunda itself), so the colonnade is a clean unbroken ring except for the one
 * opening the Mesh Stargate stands in.
 */
export const PUNCHED_COLUMN_ANGLES = ZONES.filter((z) => z.id === 'antechamber').map((z) => z.angle);

/** Radius of the colonnade line (columns sit at ROTUNDA_RADIUS - 1). */
export const COLONNADE_R = 14;

/** Camera distance to a zone's room center that triggers its lazy load. */
export const ZONE_LOAD_DISTANCE = 20;

export function zoneCenter(z: ZoneDef): [number, number, number] {
  return [Math.cos(z.angle) * z.centerDist, 0, Math.sin(z.angle) * z.centerDist];
}

export function isPunchedAngle(angle: number): boolean {
  return PUNCHED_COLUMN_ANGLES.some((a) => {
    let d = angle - a;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    return Math.abs(d) < 0.01;
  });
}
