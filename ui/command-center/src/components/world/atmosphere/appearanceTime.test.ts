import { describe, expect, it } from 'vitest';
import { daylightAmount, sampleAppearanceTime } from './timeOfDay';

describe('World follows existing resolved system/app appearance', () => {
  it('keeps light appearance sunlit even after sunset', () => {
    expect(sampleAppearanceTime(23, true).phase).toBe('day');
  });
  it('keeps dark appearance moonlit even at noon', () => {
    expect(sampleAppearanceTime(12, false).phase).toBe('night');
  });
  it('retains local dawn and dusk when they agree with appearance', () => {
    expect(sampleAppearanceTime(7, true).phase).toBe('dawn');
    expect(sampleAppearanceTime(19, false).phase).toBe('dusk');
  });
  it('uses the same lighting and lantern state for all existing consumers', () => {
    const day = sampleAppearanceTime(12, true);
    const night = sampleAppearanceTime(12, false);
    expect(day.keyIntensity).toBeGreaterThan(night.keyIntensity);
    expect(night.lanternGlow).toBeGreaterThan(day.lanternGlow);
    expect(night.starOpacity).toBeGreaterThan(day.starOpacity);
    expect(daylightAmount(day)).toBe(1);
    expect(daylightAmount(night)).toBe(0);
  });
});
