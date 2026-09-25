import { describe, expect, it } from 'vitest';
import { shouldRender } from './brainRenderGate';

const base = { active: true, width: 800, height: 600, documentHidden: false };

describe('shouldRender', () => {
  it('renders when active, sized, and visible', () => {
    expect(shouldRender(base)).toBe(true);
  });

  it('pauses when the mode is not active (e.g. List is shown)', () => {
    expect(shouldRender({ ...base, active: false })).toBe(false);
  });

  it('pauses on a zero-width or zero-height canvas', () => {
    expect(shouldRender({ ...base, width: 0 })).toBe(false);
    expect(shouldRender({ ...base, height: 0 })).toBe(false);
    expect(shouldRender({ ...base, width: 0, height: 0 })).toBe(false);
  });

  it('pauses on a backgrounded tab', () => {
    expect(shouldRender({ ...base, documentHidden: true })).toBe(false);
  });

  it('a negative or NaN size never renders (defensive — clientWidth is never negative in practice)', () => {
    expect(shouldRender({ ...base, width: -1 })).toBe(false);
    expect(shouldRender({ ...base, width: NaN })).toBe(false);
  });
});
