import { describe, expect, it } from 'vitest';
import { handsFreeStatusLabel } from './voiceStatus';

describe('handsFreeStatusLabel', () => {
  it('names the required wake phrase instead of claiming to listen for bare speech', () => {
    expect(handsFreeStatusLabel('ready', 'Hey Henry')).toBe('say “Hey Henry”');
  });

  it('keeps active turn states explicit', () => {
    expect(handsFreeStatusLabel('recording', 'Hey Henry')).toBe('● listening');
    expect(handsFreeStatusLabel('processing', 'Hey Henry')).toBe('thinking…');
  });

  it('uses the ordinary hands-free label when the wake gate is not armed', () => {
    expect(handsFreeStatusLabel('ready')).toBe('◉ hands-free');
  });
});
