import { Vector3 } from 'three';
import { habitatWalkable, HABITAT_SOLIDS, type Point3 } from './habitatLayout';

/** Low warm sun bearing — matches the golden-hour panorama and ForumSky sunrise. */
export const HABITAT_SUN_DIR = new Vector3(-0.977, 0.094, 0.191).normalize();

/** Sun centre measured in the accepted panorama; Three sphere U reverses X. */
export const HABITAT_PANORAMA_SUN_U = 0.350;
export const HABITAT_PANORAMA_ROTATION_Y = Math.atan2(HABITAT_SUN_DIR.z, -HABITAT_SUN_DIR.x) - 2 * Math.PI * HABITAT_PANORAMA_SUN_U;

/**
 * FAL vista asset paths (relative to `public/`). Codex drops generated files here;
 * until they exist the scene uses procedural sky and stays fully walkable.
 */
/** Codex FAL lane drops files under `public/world/vista/` — see
 *  `reports/world-production/ASSET_INTEGRATION_NOTE_2026-10-07.md`. */
export const HABITAT_VISTA_ASSETS = {
  manifest: 'world/vista/manifest.json',
  panoramaPrimary: 'world/vista/golden-hour-panorama-4096.jpg',
  panoramaFallback: 'world/vista/golden-hour-panorama-2048.jpg',
  cards: {
    cliffWaterfall: 'world/vista/cards/cliff-waterfall.png',
    lakeMist: 'world/vista/cards/lake-mist.png',
    ringArc: 'world/vista/cards/ring-arc.png',
    floatingIslet: 'world/vista/cards/floating-islet.png',
  },
} as const;

export type HabitatHorizonCardKind = keyof typeof HABITAT_VISTA_ASSETS.cards;

export type HabitatHorizonCard = {
  id: string;
  kind: HabitatHorizonCardKind;
  position: Point3;
  width: number;
  height: number;
  /** Y rotation so the card faces the terrace. */
  rotationY: number;
  /** Camera-parallax strength; frozen when reduced motion is on. */
  parallax: number;
};

/** Distant 2.5D cards along the terrace-facing horizon — outside every walk bound. */
export const HABITAT_HORIZON_CARDS: HabitatHorizonCard[] = [
  { id: 'cliff-waterfall', kind: 'cliffWaterfall', position: [-38, 14, -52], width: 48, height: 28, rotationY: 0.48, parallax: 0.08 },
  { id: 'floating-islet', kind: 'floatingIslet', position: [42, 18, -38], width: 28, height: 20, rotationY: -1.05, parallax: 0.06 },
];

const cardFootprint = (card: HabitatHorizonCard, margin = 0) => ({
  minX: card.position[0] - card.width / 2 - margin,
  maxX: card.position[0] + card.width / 2 + margin,
  minZ: card.position[2] - margin,
  maxZ: card.position[2] + margin,
});

/** Cards must sit beyond solids and off every walkable sample in their footprint. */
export function horizonCardOutsideWalkBounds(card: HabitatHorizonCard, sampleStep = 4, margin = 2) {
  const { minX, maxX, minZ, maxZ } = cardFootprint(card, margin);
  for (let x = minX; x <= maxX; x += sampleStep) {
    for (let z = minZ; z <= maxZ; z += sampleStep) {
      if (habitatWalkable(x, z)) return false;
    }
  }
  for (const solid of HABITAT_SOLIDS) {
    const dx = Math.max(Math.abs(card.position[0] - solid.at[0]) - solid.size[0] / 2 - card.width / 2, 0);
    const dz = Math.max(Math.abs(card.position[2] - solid.at[2]) - solid.size[2] / 2, 0);
    if (Math.hypot(dx, dz) < margin) return false;
  }
  return true;
}

export function habitatVistaAssetUrl(relativePath: string) {
  return `${import.meta.env.BASE_URL}${relativePath}`;
}

export const HABITAT_DAY_PANORAMA_URLS = [
  habitatVistaAssetUrl(HABITAT_VISTA_ASSETS.panoramaPrimary),
  habitatVistaAssetUrl(HABITAT_VISTA_ASSETS.panoramaFallback),
] as const;
