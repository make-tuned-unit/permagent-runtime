import { describe, expect, it } from 'vitest';
import { dispatchBody } from './dispatchBody';

describe('dispatchBody', () => {
  it('unwraps a reply that IS a fenced block — the shape a drafted dispatch arrives in', () => {
    expect(dispatchBody('```\nSend this to the vendor.\nThanks,\nJesse\n```'))
      .toBe('Send this to the vendor.\nThanks,\nJesse');
  });

  it('drops the info string too, so a language/kind tag never reaches the clipboard', () => {
    expect(dispatchBody('```markdown\n# Brief\n\nBuild the thing.\n```'))
      .toBe('# Brief\n\nBuild the thing.');
  });

  it('ignores leading and trailing blank space around the fence', () => {
    expect(dispatchBody('\n\n```text\npayload\n```\n\n')).toBe('payload');
  });

  it('keeps prose that surrounds the fence — half a reply is worse than three backticks', () => {
    const reply = "Here's the prompt I'd send:\n\n```\nDo the thing.\n```\n\nWant me to adjust the tone?";
    expect(dispatchBody(reply)).toBe(reply);
  });

  it('keeps everything when there are two fenced blocks', () => {
    const reply = '```\nfirst\n```\n\nand also\n\n```\nsecond\n```';
    expect(dispatchBody(reply)).toBe(reply);
  });

  it('leaves an ordinary prose reply alone apart from trimming', () => {
    expect(dispatchBody('  Sure — I emailed them.  ')).toBe('Sure — I emailed them.');
  });

  it('does not mangle a fenced block whose body contains inline backticks', () => {
    expect(dispatchBody('```\nrun `npm test` first\n```')).toBe('run `npm test` first');
  });

  it('handles an empty body without throwing', () => {
    expect(dispatchBody('')).toBe('');
  });
});
