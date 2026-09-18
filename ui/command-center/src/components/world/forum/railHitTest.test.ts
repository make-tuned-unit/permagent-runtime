import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * Jesse: "the places menu in world doesnt open."
 *
 * It could not be opened at all, and the cause was hit testing rather than
 * state. The rail is a flex row — a 32px handle, then the list — and the
 * closed list is slid `translateX(-102%)` out of view. A transform moves what
 * you see AND what you can click, so the list's right edge came to rest
 * exactly over the handle, while `opacity: 0` left it fully clickable. An
 * invisible panel sat on the button and ate every press: the thing you were
 * clicking to open the menu was the closed menu.
 *
 * jsdom has no layout, so it cannot reproduce the overlap. What it can do is
 * hold the rule that fixes it, which is the part that would be silently lost.
 */
const CSS = readFileSync(new URL('./forum.css', import.meta.url), 'utf8');

function closedListBlock(): string {
  const start = CSS.indexOf(".forum-rail[data-open='closed'] .forum-rail-list");
  expect(start, 'the closed-rail rule went missing').toBeGreaterThan(-1);
  return CSS.slice(start, CSS.indexOf('}', start));
}

describe('the closed place rail cannot swallow its own handle', () => {
  it('drops pointer events on the list while it is closed', () => {
    expect(closedListBlock()).toMatch(/pointer-events:\s*none/);
  });

  it('still slides and fades, so the fix did not cost the animation', () => {
    const block = closedListBlock();
    expect(block).toMatch(/transform:\s*translateX\(-102%\)/);
    expect(block).toMatch(/opacity:\s*0/);
  });

  it('keeps the handle itself clickable', () => {
    // The rail container is pointer-events: none so the world can be dragged
    // through it; the handle has to opt back in or nothing works.
    const start = CSS.indexOf('.forum-rail-handle {');
    const block = CSS.slice(start, CSS.indexOf('}', start));
    expect(block).toMatch(/pointer-events:\s*auto/);
  });
});
