import { describe, expect, it, vi } from 'vitest';
vi.mock('./BrainScene', () => ({ BrainScene: () => null }));
import { brainSliderRight } from './BrainView';
import { CHAT_LAUNCHER_MARGIN } from '../chat/ChatLauncher';

// QA 2026-09-19, real window: the floating "Chat with Henry" pill covered the
// right end of the Brain time slider, which ran to 16px from the edge.
describe('brainSliderRight', () => {
  it('stops short of the chat pill when it is showing', () => {
    const right = brainSliderRight(false, { width: 150, height: 44 });
    expect(right).toBeGreaterThanOrEqual(150 + CHAT_LAUNCHER_MARGIN);
  });
  it('uses the plain gutter while the pill is hidden', () => {
    expect(brainSliderRight(false, null)).toBe(16);
  });
  it('still clears the side panel when a node is selected', () => {
    expect(brainSliderRight(true, { width: 150, height: 44 })).toBe(376);
  });
});
