// W2 prop library — three-tier material singletons.
// WORLD_VIEW_BIBLE.md §1: every surface is Stone, Metal, or Light. Prop families
// share these singletons; new materials require a bible amendment.
//
// Module-lifetime singletons (never disposed). One material = one shader
// program = batchable draw calls across every prop family.
//
// All hex comes from shared/palette.ts — never inline (§2).
//
// Stone and Light are MeshLambertMaterial, not MeshStandardMaterial. The scene
// carries fifteen lights and this is measured fill-rate bound, so the per-fragment
// cost of Standard's GGX + multiscatter BRDF is real money paid on every pixel of
// every wall, floor, dome, spine, and inlay in the world. Stone never has a
// specular highlight worth paying for — under the near-black ambient and dense
// fog this scene runs under, a matte surface's spec lobe is invisible anyway —
// and Light tier is emissive-only, so neither tier has any use for roughness or
// metalness in the first place. Metal keeps Standard on purpose: metalGunmetal and
// metalBronze are the one tier where the mid-metalness spec highlight is the whole
// point, so do not "fix" them by converting to Lambert, and do not add roughness
// or metalness back onto anything below — Lambert doesn't have those properties
// and three will warn on every one it's handed.

import * as THREE from 'three';
import { ENV, STATE } from '../shared/palette';

// ─── Tier 1: STONE (structure) — matte, never emissive ──────────────────────

export const stoneMarble = new THREE.MeshLambertMaterial({
  color: ENV.marble,
});

export const stoneDark = new THREE.MeshLambertMaterial({
  color: ENV.darkStone,
});

/**
 * Raw rock: banked rubble, boulders, crude footings, unhewn mass. Same
 * darkStone hex as stoneDark (no new palette entry), and Lambert has no
 * roughness to carry the "lived-in, tool-marks-left-honest" read — so the
 * distinction moved into the geometry instead, where it always belonged.
 * Flat shading drops the smoothed vertex normals and lights each triangle as
 * the facet it actually is, which is what unhewn stone looks like. It is a
 * shader define rather than another lighting term, so it reads as rougher
 * without costing anything, and it is a truer answer than a roughness value
 * on a scene that has no texture maps at all (bible §1 corollaries).
 */
export const stoneRough = new THREE.MeshLambertMaterial({
  color: ENV.darkStone,
  flatShading: true,
});

// ─── Tier 2: METAL (mechanism) — mid metalness, no emissive ─────────────────

export const metalGunmetal = new THREE.MeshStandardMaterial({
  color: ENV.gunmetal,
  metalness: 0.7,
  roughness: 0.35,
});

export const metalBronze = new THREE.MeshStandardMaterial({
  color: ENV.bronze,
  metalness: 0.75,
  roughness: 0.4,
});

// ─── Tier 3: LIGHT (intelligence) — the only emissive tier, intensity ≤ 2.0 ─
// Engraved channels/inlays/seams; never free-floating signage (§1).

export const lightCyan = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonCyan,
  emissiveIntensity: 1.6,
});

export const lightAmber = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.4,
});

export const lightViolet = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.violet,
  emissiveIntensity: 1.6,
});

/**
 * Slim amber WORK-light. Warm but DIM — the Librarian's lamp aesthetic,
 * deliberately below the focal ceiling so a fixture reads as one dark mass +
 * one accent.
 *
 * SEMANTIC NOTE: this is a WORK-light, not HUD amber.
 * HUD amber (STATE.working) is "working-for-the-user" and belongs to AGENTS only
 * (visors, crown gems, station ambience). A lamp on a crane is just a lamp.
 */
export const lightAmberWork = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.1,
});

/**
 * Tending register (the third agent state): gray-warm, unhurried,
 * NEVER amber. Survey lines, banked-material markers, the tending sledge's
 * load-glow. A cool desaturated value lerped off idle gray so it reads as
 * "ambient site activity", distinct from both HUD amber and intelligence cyan.
 */
const tendingColor = new THREE.Color(STATE.idle).lerp(new THREE.Color(ENV.bronze), 0.3);
export const lightTending = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: tendingColor,
  emissiveIntensity: 0.9,
});

/**
 * Dedicated pulse instances — animated by exactly one owner each so the pulse
 * never leaks into unrelated props sharing the base singleton.
 * BrainShelves owns lightVioletShard; AutomateSteles owns lightAmberTick.
 */
export const lightVioletShard = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.violet,
  emissiveIntensity: 1.7,
});

export const lightAmberTick = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.5,
});

/**
 * Additive Light-tier singletons (environment-truth pass, same additive
 * precedent as lightTending/lightAmberWork above). Each animated instance is
 * owned by exactly one component so a pulse never leaks across props:
 *
 *   lightErrorTick — HUD-red status tier for props that report a REAL failed
 *     state (a schedule that errored/missed on the Horologium; a failed task's
 *     dying ember on the benches). Same §2 error hex as the agents' channel.
 *   lightIdleTick — gray idle tier: a real-but-dormant unit (an idle or paused
 *     schedule). Reads as "present, not claiming work".
 *   lightLantern — the colonnade lantern cores; intensity is owned by
 *     ColonnadeLanterns, driven by the (real-clock) time of day.
 */
export const lightErrorTick = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: STATE.error,
  emissiveIntensity: 1.3,
});

export const lightIdleTick = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: STATE.idle,
  emissiveIntensity: 0.55,
});

export const lightLantern = new THREE.MeshLambertMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.2,
});

// Holo surfaces — the sanctioned transparency use (§1 corollaries).

export const holoCyan = new THREE.MeshBasicMaterial({
  color: ENV.neonCyan,
  transparent: true,
  opacity: 0.18,
  side: THREE.DoubleSide,
  depthWrite: false,
});

export const holoViolet = new THREE.MeshBasicMaterial({
  color: ENV.violet,
  transparent: true,
  opacity: 0.16,
  side: THREE.DoubleSide,
  depthWrite: false,
});

// ─── Book spine ramp (Stone tier) ────────────────────────────────────────────
// Eight fixed matte tones derived by lerping palette constants — no new hex.
// Buckets keep the mezzanine book wall at 8 draw calls for ~390 spines while
// preserving the "varied spines" read of the legacy per-mesh wall.

function lerp(a: string, b: string, t: number): THREE.Color {
  return new THREE.Color(a).lerp(new THREE.Color(b), t);
}

export const bookRamp: readonly THREE.MeshLambertMaterial[] = [
  lerp(ENV.darkStone, ENV.marbleVein, 0.25),
  lerp(ENV.darkStone, ENV.marbleVein, 0.55),
  lerp(ENV.darkStone, ENV.bronze, 0.3),
  lerp(ENV.darkStone, ENV.bronze, 0.55),
  lerp(ENV.darkStone, ENV.violet, 0.25),
  lerp(ENV.darkStone, ENV.neonCyan, 0.18),
  lerp(ENV.darkStone, ENV.gunmetal, 0.5),
  lerp(ENV.darkStone, ENV.marble, 0.2),
].map((color) => new THREE.MeshLambertMaterial({ color }));
