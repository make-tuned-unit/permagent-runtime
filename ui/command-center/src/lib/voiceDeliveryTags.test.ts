import { describe, expect, it } from 'vitest';
import { stripVoiceDeliveryTags } from './voiceDeliveryTags';

describe('stripVoiceDeliveryTags', () => {
  it('drops the tags a voice reply is saved with', () => {
    expect(stripVoiceDeliveryTags("[pause] Let me add that for you.\n\n[pause] There's a Confirm card."))
      .toBe("Let me add that for you.\n\nThere's a Confirm card.");
    expect(stripVoiceDeliveryTags('[Excited] We shipped it!')).toBe('We shipped it!');
  });
  it('keeps links and ordinary bracketed words', () => {
    expect(stripVoiceDeliveryTags('Read [the tide table](https://x.example) now.'))
      .toBe('Read [the tide table](https://x.example) now.');
    expect(stripVoiceDeliveryTags('A [HIGH] finding and [pause](https://x.example).'))
      .toBe('A [HIGH] finding and [pause](https://x.example).');
  });
});
