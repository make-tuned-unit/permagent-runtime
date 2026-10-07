import { describe, expect, it } from 'vitest';
import {
  WATER_COURT,
  advanceWaterTime,
  cascadeLandingPoint,
  cascadeLipPoint,
  cascadeVertex,
  downstreamPhase,
  impactFoamGlsl,
  impactFoamWeight,
  inPoolBounds,
  poolDepthFactor,
  smoothstep,
} from './habitatWaterFlow';
import { createHabitatWaterMaterial } from './habitatWaterMaterial';

describe('habitat water flow contract', () => {
  it('advances time with a resume-safe delta and freezes under reduced motion', () => {
    expect(advanceWaterTime(1.5, 0.016, false)).toBeCloseTo(1.516, 5);
    expect(advanceWaterTime(1.5, 2, false)).toBeCloseTo(1.55, 5);
    expect(advanceWaterTime(1.5, -1, false)).toBe(1.5);
    expect(advanceWaterTime(1.5, 0.016, true)).toBe(1.5);
    expect(advanceWaterTime(1.5, Number.NaN, false)).toBe(1.5);
  });

  it('keeps a single downstream phase direction from basin through fall to pool', () => {
    const t = 2.5;
    const upper = downstreamPhase(4, WATER_COURT.upper.center[1], WATER_COURT.upper.center[2], t);
    const lip = downstreamPhase(4, WATER_COURT.cascade.lipY, WATER_COURT.cascade.startZ, t);
    const midFall = downstreamPhase(4, 0.7, -6.2, t);
    const landing = downstreamPhase(4, WATER_COURT.cascade.landY, -6.0, t);
    const rill = downstreamPhase(4, WATER_COURT.rill.center[1], WATER_COURT.rill.center[2], t);
    const lower = downstreamPhase(4, WATER_COURT.lower.center[1], WATER_COURT.lower.center[2], t);
    expect(lip).toBeGreaterThan(upper);
    expect(midFall).toBeGreaterThan(lip);
    expect(landing).toBeGreaterThan(midFall);
    expect(rill).toBeGreaterThan(landing);
    expect(lower).toBeGreaterThan(rill);
    expect(downstreamPhase(4, 0.7, -6.2, t + 1)).toBeLessThan(midFall);
  });

  it('meets the measured lip and receiving rill without floating past the court', () => {
    const lip = cascadeLipPoint(0.5);
    expect(lip.y).toBeCloseTo(WATER_COURT.cascade.lipY, 5);
    expect(lip.z).toBeCloseTo(WATER_COURT.cascade.startZ, 5);
    const upperSouth = WATER_COURT.upper.center[2] + WATER_COURT.upper.sizeXZ[1] / 2;
    expect(Math.abs(lip.z - upperSouth)).toBeLessThan(0.03);

    const land = cascadeLandingPoint(0.5);
    expect(land.y).toBeCloseTo(WATER_COURT.cascade.landY, 5);
    expect(inPoolBounds(land.x, land.z, WATER_COURT.rill.center, WATER_COURT.rill.sizeXZ, 0.08)).toBe(true);

    for (let u = 0; u <= 1; u += 0.25) {
      for (let v = 0; v <= 1; v += 0.25) {
        const p = cascadeVertex(u, v);
        expect(Math.abs(p.x - 4)).toBeLessThanOrEqual(WATER_COURT.cascade.width / 2 + 0.02);
        expect(p.y).toBeLessThanOrEqual(WATER_COURT.cascade.lipY + 1e-6);
        expect(p.y).toBeGreaterThanOrEqual(WATER_COURT.cascade.landY - 1e-6);
      }
    }
  });

  it('matches the GLSL foam algebra at upstream, strike, wake and out-of-band samples', () => {
    const f = WATER_COURT.foam;
    const t = 1.25;
    // Manual evaluation of the same expressions the GLSL emits.
    const ref = (x: number, z: number) => {
      const along = z - f.impactZ;
      const channel = 1 - Math.min(1, Math.max(0, Math.abs(x - 4) / f.channelHalf));
      const strike = Math.exp(-((along + 0.05) ** 2) / (2 * f.strikeSigma ** 2));
      const wake = along <= 0 ? 0 : Math.exp(-(along ** 2) / (2 * f.wakeSigma ** 2)) * f.wakeGain;
      const band = smoothstep(f.upstreamMin, f.upstreamMin + 0.12, along)
        * (1 - smoothstep(f.downstreamMax - 0.35, f.downstreamMax, along));
      const phase = z * 1.35 + (x - 4) * 0.18 - t * 1.15;
      const flicker = 0.72 + 0.28 * Math.sin(phase * 1.7 + Math.sin(x * 7.3 - t * 0.9));
      return Math.min(1, Math.max(0, (strike + wake) * channel * band * flicker));
    };

    const samples: [number, number][] = [
      [4, WATER_COURT.upper.center[2]],
      [4, f.impactZ - 0.4],
      [4, f.impactZ - 0.2],
      [4, f.impactZ],
      [4, f.impactZ + 0.9],
      [4, f.impactZ + 2.6],
      [4, WATER_COURT.lower.center[2]],
      [6.5, f.impactZ],
    ];
    for (const [x, z] of samples) {
      expect(impactFoamWeight(x, z, t)).toBeCloseTo(ref(x, z), 10);
    }

    expect(impactFoamWeight(4, WATER_COURT.upper.center[2], t)).toBe(0);
    expect(impactFoamWeight(4, f.impactZ - 0.4, t)).toBe(0);
    const strike = impactFoamWeight(4, f.impactZ, t);
    const wake = impactFoamWeight(4, f.impactZ + 0.9, t);
    const farFade = impactFoamWeight(4, f.impactZ + 2.6, t);
    expect(strike).toBeGreaterThan(0.45);
    expect(wake).toBeGreaterThan(0.05);
    expect(wake).toBeLessThan(strike);
    expect(farFade).toBeLessThan(wake);
    expect(impactFoamWeight(4, WATER_COURT.lower.center[2], t)).toBe(0);
    expect(impactFoamWeight(6.5, f.impactZ, t)).toBe(0);

    // Source check: material injects shared GLSL and syncs clearcoatNormal (not shader execution).
    const material = createHabitatWaterMaterial({ value: 0 });
    expect(material.roughness).toBeCloseTo(0.4, 5);
    expect(material.clearcoat).toBeLessThanOrEqual(0.12);
    expect(material.clearcoatRoughness).toBeGreaterThanOrEqual(0.55);
    expect(material.envMapIntensity).toBeLessThanOrEqual(0.55);
    expect(material.specularIntensity).toBeLessThanOrEqual(0.55);
    const shader = {
      uniforms: {} as Record<string, { value: unknown }>,
      vertexShader: '#include <worldpos_vertex>',
      fragmentShader: '#include <normal_fragment_maps>\n#include <clearcoat_normal_fragment_maps>\n#include <color_fragment>',
    };
    material.onBeforeCompile(shader as Parameters<typeof material.onBeforeCompile>[0], undefined as never);
    expect(shader.fragmentShader).toContain('waveXZ');
    expect(shader.fragmentShader).toContain('depthXZ');
    expect(shader.fragmentShader).toContain('clearcoatNormal = normalize');
    expect(shader.fragmentShader.match(/\bvec2 p\b/g) ?? []).toHaveLength(0);
    expect(shader.fragmentShader).toContain(impactFoamGlsl('depthT').trim().slice(0, 48));
    expect(shader.fragmentShader).not.toContain('vWaterView');
    material.dispose();
    expect(poolDepthFactor(4, -8.05, WATER_COURT.upper.center, WATER_COURT.upper.sizeXZ)).toBeGreaterThan(0.85);
  });

  it('covers the hollow inner wall span with a slight wall overlap', () => {
    const bed = WATER_COURT.beds.upper;
    const innerWest = 2.48;
    const innerEast = 5.52;
    const innerNorth = -9.32;
    const innerSouth = -6.68;
    expect(bed.size[0]).toBeCloseTo(3.06, 5);
    expect(bed.size[2]).toBeCloseTo(2.66, 5);
    expect(bed.center[0] - bed.size[0] / 2).toBeLessThanOrEqual(innerWest);
    expect(bed.center[0] + bed.size[0] / 2).toBeGreaterThanOrEqual(innerEast);
    expect(bed.center[2] - bed.size[2] / 2).toBeLessThanOrEqual(innerNorth);
    expect(bed.center[2] + bed.size[2] / 2).toBeGreaterThanOrEqual(innerSouth);
    expect(bed.topY).toBeLessThan(WATER_COURT.upper.center[1] - 0.2);
  });
});
