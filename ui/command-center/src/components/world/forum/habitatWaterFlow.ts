/**
 * Crafted-garden water-court flow contract.
 *
 * Coordinates match HabitatArchitecture / HabitatWater. Flow is downstream
 * toward +Z: upper basin → lip → cascade sheet → receiving rill → lower pool
 * under the bridge. Foam weight is one production formula: JS for tests and
 * GLSL text injected into the pool material must stay algebraically identical.
 */

export const WATER_COURT = {
  upper: { center: [4, 1.255, -8.05] as const, sizeXZ: [3.04, 3.22] as const },
  lower: { center: [4, 0.085, -2] as const, sizeXZ: [3.76, 5.94] as const },
  rill: { center: [4, 0.085, -5.51] as const, sizeXZ: [2.78, 1.10] as const },
  cascade: {
    centerX: 4,
    width: 2.76,
    lipY: 1.255,
    landY: 0.085,
    startZ: -6.43,
    fall: 1.17,
    outward: 0.39,
    rows: 20,
    cols: 24,
  },
  /**
   * Visual beds. Upper floor overlaps the hollow inner wall faces
   * (x 2.48–5.52, z −9.32–−6.68) by ~1 cm so translucent edges stay watertight.
   */
  beds: {
    upper: { center: [4, 0.88, -8] as const, size: [3.06, 0.08, 2.66] as const, topY: 0.92 },
    lower: { center: [4, -0.02, -2] as const, size: [3.70, 0.06, 5.88] as const, topY: 0.01 },
    rill: { center: [4, -0.02, -5.51] as const, size: [2.72, 0.06, 1.04] as const, topY: 0.01 },
  },
  foam: {
    impactZ: -5.85,
    upstreamMin: -0.28,
    downstreamMax: 2.8,
    channelHalf: 1.35,
    strikeSigma: 0.12,
    wakeSigma: 0.85,
    wakeGain: 0.55,
    mix: 0.38,
  },
} as const;

export type WaterPoint = { x: number; y: number; z: number };

/** GLSL smoothstep — used so the CPU foam reference matches the renderer. */
export function smoothstep(edge0: number, edge1: number, x: number): number {
  if (edge0 === edge1) return x < edge0 ? 0 : 1;
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

/** Bound-and-gate the simulation clock; never runs backward on resume spikes. */
export function advanceWaterTime(current: number, dt: number, reduceMotion: boolean): number {
  if (reduceMotion) return current;
  const step = Number.isFinite(dt) ? Math.min(Math.max(dt, 0), 0.05) : 0;
  return current + step;
}

/**
 * Scalar phase that advances with the stream. Larger values further downstream
 * (+Z) and further down the fall; time subtracts so patterns advect with flow.
 */
export function downstreamPhase(x: number, y: number, z: number, time: number): number {
  const drop = (WATER_COURT.cascade.lipY - y) / WATER_COURT.cascade.fall;
  return z * 1.35 + drop * 2.6 + x * 0.18 - time * 1.15;
}

/** Cascade sheet sample. `u` across width [0,1], `v` down the fall [0 lip → 1 land]. */
export function cascadeVertex(u: number, v: number): WaterPoint {
  const { centerX, width, lipY, fall, startZ, outward } = WATER_COURT.cascade;
  const clampedU = Math.min(1, Math.max(0, u));
  const clampedV = Math.min(1, Math.max(0, v));
  const taper = 1 - clampedV * 0.04;
  const wave = Math.sin(clampedU * Math.PI * 3) * 0.012 * clampedV;
  return {
    x: centerX + (clampedU - 0.5) * width * taper + wave,
    y: lipY - clampedV * fall,
    z: startZ + Math.sqrt(clampedV) * outward + clampedV * clampedV * 0.06,
  };
}

export function cascadeLipPoint(u = 0.5): WaterPoint {
  return cascadeVertex(u, 0);
}

export function cascadeLandingPoint(u = 0.5): WaterPoint {
  return cascadeVertex(u, 1);
}

/** True when a world XZ point sits inside a pool's rectangle (inclusive edges). */
export function inPoolBounds(
  x: number,
  z: number,
  center: readonly [number, number, number],
  sizeXZ: readonly [number, number],
  margin = 0,
): boolean {
  return Math.abs(x - center[0]) <= sizeXZ[0] / 2 + margin
    && Math.abs(z - center[2]) <= sizeXZ[1] / 2 + margin;
}

/** 0 at the pool margin, 1 toward the deepest interior — for depth tinting. */
export function poolDepthFactor(
  x: number,
  z: number,
  center: readonly [number, number, number],
  sizeXZ: readonly [number, number],
): number {
  const nx = Math.abs(x - center[0]) / (sizeXZ[0] * 0.5);
  const nz = Math.abs(z - center[2]) / (sizeXZ[1] * 0.5);
  const edge = Math.min(1, Math.max(nx, nz));
  return Math.min(1, Math.max(0, 1 - edge));
}

/**
 * Localized landing foam — algebraically identical to `impactFoamGlsl`.
 */
export function impactFoamWeight(x: number, z: number, time: number): number {
  const f = WATER_COURT.foam;
  const along = z - f.impactZ;
  const channel = 1 - Math.min(1, Math.max(0, Math.abs(x - 4) / f.channelHalf));
  const strike = Math.exp(-((along + 0.05) ** 2) / (2 * f.strikeSigma ** 2));
  const wake = along <= 0 ? 0 : Math.exp(-(along ** 2) / (2 * f.wakeSigma ** 2)) * f.wakeGain;
  const band = smoothstep(f.upstreamMin, f.upstreamMin + 0.12, along)
    * (1 - smoothstep(f.downstreamMax - 0.35, f.downstreamMax, along));
  const phase = z * 1.35 + (x - 4) * 0.18 - time * 1.15;
  const flicker = 0.72 + 0.28 * Math.sin(phase * 1.7 + Math.sin(x * 7.3 - time * 0.9));
  return Math.min(1, Math.max(0, (strike + wake) * channel * band * flicker));
}

/**
 * GLSL statements that assign `foam` from world XZ and a time expression.
 * Must stay algebraically aligned with `impactFoamWeight`.
 */
export function impactFoamGlsl(timeExpression = 'uWaterTime'): string {
  const f = WATER_COURT.foam;
  return `
      vec2 foamXZ = vWaterWorld.xz;
      float along = foamXZ.y - (${f.impactZ.toFixed(4)});
      float channel = 1.0 - clamp(abs(foamXZ.x - 4.0) / ${f.channelHalf.toFixed(4)}, 0.0, 1.0);
      float strike = exp(-((along + 0.05) * (along + 0.05)) / (2.0 * ${f.strikeSigma.toFixed(4)} * ${f.strikeSigma.toFixed(4)}));
      float wake = along <= 0.0 ? 0.0 : exp(-(along * along) / (2.0 * ${f.wakeSigma.toFixed(4)} * ${f.wakeSigma.toFixed(4)})) * ${f.wakeGain.toFixed(4)};
      float band = smoothstep(${f.upstreamMin.toFixed(4)}, ${f.upstreamMin.toFixed(4)} + 0.12, along)
                 * (1.0 - smoothstep(${f.downstreamMax.toFixed(4)} - 0.35, ${f.downstreamMax.toFixed(4)}, along));
      float phase = foamXZ.y * 1.35 + (foamXZ.x - 4.0) * 0.18 - (${timeExpression}) * 1.15;
      float flicker = 0.72 + 0.28 * sin(phase * 1.7 + sin(foamXZ.x * 7.3 - (${timeExpression}) * 0.9));
      float foam = clamp((strike + wake) * channel * band * flicker, 0.0, 1.0);
  `;
}
