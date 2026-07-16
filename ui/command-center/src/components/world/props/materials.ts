// W2 prop library — three-tier material singletons.
// WORLD_VIEW_BIBLE.md §1: every surface is Stone, Metal, or Light. Prop families
// share these singletons; new materials require a bible amendment.
//
// Module-lifetime singletons (never disposed). One material = one shader
// program = batchable draw calls across every prop family.
//
// All hex comes from shared/palette.ts — never inline (§2).

import * as THREE from 'three';
import { ENV, STATE } from '../shared/palette';

// ─── Tier 1: STONE (structure) — matte, never emissive ──────────────────────

export const stoneMarble = new THREE.MeshStandardMaterial({
  color: ENV.marble,
  roughness: 0.35,
  metalness: 0.05,
});

export const stoneDark = new THREE.MeshStandardMaterial({
  color: ENV.darkStone,
  roughness: 0.3,
  metalness: 0.15,
});

/**
 * Raw rock: rougher and flatter than the composed dark stone — banked rubble,
 * boulders, crude footings, unhewn mass. Same darkStone hex (no new palette entry);
 * the roughness change carries the "lived-in, tool-marks-left-honest" read.
 */
export const stoneRough = new THREE.MeshStandardMaterial({
  color: ENV.darkStone,
  roughness: 0.95,
  metalness: 0,
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

export const lightCyan = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonCyan,
  emissiveIntensity: 1.6,
  roughness: 0.4,
  metalness: 0,
});

export const lightAmber = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.4,
  roughness: 0.4,
  metalness: 0,
});

export const lightViolet = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.violet,
  emissiveIntensity: 1.6,
  roughness: 0.4,
  metalness: 0,
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
export const lightAmberWork = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.1,
  roughness: 0.5,
  metalness: 0,
});

/**
 * Tending register (the third agent state): gray-warm, unhurried,
 * NEVER amber. Survey lines, banked-material markers, the tending sledge's
 * load-glow. A cool desaturated value lerped off idle gray so it reads as
 * "ambient site activity", distinct from both HUD amber and intelligence cyan.
 */
const tendingColor = new THREE.Color(STATE.idle).lerp(new THREE.Color(ENV.bronze), 0.3);
export const lightTending = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: tendingColor,
  emissiveIntensity: 0.9,
  roughness: 0.6,
  metalness: 0,
});

/**
 * Dedicated pulse instances — animated by exactly one owner each so the pulse
 * never leaks into unrelated props sharing the base singleton.
 * BrainShelves owns lightVioletShard; AutomateSteles owns lightAmberTick.
 */
export const lightVioletShard = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.violet,
  emissiveIntensity: 1.7,
  roughness: 0.4,
  metalness: 0,
});

export const lightAmberTick = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.5,
  roughness: 0.4,
  metalness: 0,
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
export const lightErrorTick = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: STATE.error,
  emissiveIntensity: 1.3,
  roughness: 0.4,
  metalness: 0,
});

export const lightIdleTick = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: STATE.idle,
  emissiveIntensity: 0.55,
  roughness: 0.5,
  metalness: 0,
});

export const lightLantern = new THREE.MeshStandardMaterial({
  color: ENV.deepVoid,
  emissive: ENV.neonAmber,
  emissiveIntensity: 1.2,
  roughness: 0.5,
  metalness: 0,
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

export const bookRamp: readonly THREE.MeshStandardMaterial[] = [
  lerp(ENV.darkStone, ENV.marbleVein, 0.25),
  lerp(ENV.darkStone, ENV.marbleVein, 0.55),
  lerp(ENV.darkStone, ENV.bronze, 0.3),
  lerp(ENV.darkStone, ENV.bronze, 0.55),
  lerp(ENV.darkStone, ENV.violet, 0.25),
  lerp(ENV.darkStone, ENV.neonCyan, 0.18),
  lerp(ENV.darkStone, ENV.gunmetal, 0.5),
  lerp(ENV.darkStone, ENV.marble, 0.2),
].map(
  (color) => new THREE.MeshStandardMaterial({ color, roughness: 0.85, metalness: 0 })
);
