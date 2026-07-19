// The world keeps the user's hours — a REAL signal (the local clock). These
// pin the pure curve: continuous across the whole cycle (incl. the midnight
// wrap), midday == the bible §1 baseline, and monotone day/night contrast so
// lanterns/stars/nebula read correctly.

import { describe, expect, it } from 'vitest';
import { sampleTimeOfDay, phaseOf, mixHex } from './timeOfDay';

describe('mixHex', () => {
  it('returns the endpoints and the midpoint', () => {
    expect(mixHex('#000000', '#FFFFFF', 0)).toBe('#000000');
    expect(mixHex('#000000', '#FFFFFF', 1)).toBe('#FFFFFF');
    expect(mixHex('#000000', '#FFFFFF', 0.5)).toBe('#808080');
  });
});

describe('phaseOf', () => {
  it('bands the day, wrapping negatives/overflows', () => {
    expect(phaseOf(3)).toBe('night');
    expect(phaseOf(7)).toBe('dawn');
    expect(phaseOf(13)).toBe('day');
    expect(phaseOf(19)).toBe('dusk');
    expect(phaseOf(23)).toBe('night');
    expect(phaseOf(-1)).toBe('night'); // 23:00
    expect(phaseOf(25)).toBe('night'); // 01:00
    expect(phaseOf(31)).toBe('dawn'); // 07:00
  });
});

describe('sampleTimeOfDay', () => {
  it('midday is exactly the bible §1 baseline (#FFF0D4 @ 1.6, fill .25, amb .08)', () => {
    const noon = sampleTimeOfDay(12);
    expect(noon.keyColor).toBe('#FFF0D4');
    expect(noon.keyIntensity).toBeCloseTo(1.6, 5);
    expect(noon.fillIntensity).toBeCloseTo(0.25, 5);
    expect(noon.ambientIntensity).toBeCloseTo(0.08, 5);
    expect(noon.phase).toBe('day');
  });

  it('night is dimmer + cooler than day, with lanterns/stars/nebula up', () => {
    const night = sampleTimeOfDay(1);
    const day = sampleTimeOfDay(13);
    expect(night.keyIntensity).toBeLessThan(day.keyIntensity);
    expect(night.ambientIntensity).toBeLessThan(day.ambientIntensity);
    expect(night.lanternGlow).toBeGreaterThan(day.lanternGlow);
    expect(night.starOpacity).toBeGreaterThan(day.starOpacity);
    expect(night.nebulaOpacity).toBeGreaterThan(day.nebulaOpacity);
    expect(night.fireflies).toBeGreaterThan(day.fireflies);
  });

  it('fireflies + full starfield are gone at midday (day gates them off)', () => {
    const noon = sampleTimeOfDay(12);
    expect(noon.fireflies).toBe(0);
    expect(noon.starOpacity).toBeLessThan(0.35);
  });

  it('wraps mod 24 — 0h and 24h and 48h all sample identically', () => {
    const a = sampleTimeOfDay(0);
    const b = sampleTimeOfDay(24);
    const c = sampleTimeOfDay(48);
    expect(b).toEqual(a);
    expect(c).toEqual(a);
    // and negative hours wrap too (−2h == 22:00)
    expect(sampleTimeOfDay(-2).keyIntensity).toBeCloseTo(sampleTimeOfDay(22).keyIntensity, 5);
  });

  it('is continuous across the midnight wrap (no seam at 24→0)', () => {
    const before = sampleTimeOfDay(23.98);
    const after = sampleTimeOfDay(0.02);
    // Values a few minutes apart across midnight must be near-identical.
    expect(Math.abs(after.keyIntensity - before.keyIntensity)).toBeLessThan(0.05);
    expect(Math.abs(after.ambientIntensity - before.ambientIntensity)).toBeLessThan(0.02);
  });

  it('every sampled scalar stays within sane [0,2]-ish bounds across the cycle', () => {
    for (let h = 0; h < 24; h += 0.25) {
      const s = sampleTimeOfDay(h);
      expect(s.keyIntensity).toBeGreaterThan(0);
      expect(s.keyIntensity).toBeLessThanOrEqual(2);
      expect(s.starOpacity).toBeGreaterThanOrEqual(0);
      expect(s.starOpacity).toBeLessThanOrEqual(1);
      expect(s.fireflies).toBeGreaterThanOrEqual(0);
      expect(s.fireflies).toBeLessThanOrEqual(1);
      expect(s.keyColor).toMatch(/^#[0-9A-F]{6}$/);
    }
  });
});
