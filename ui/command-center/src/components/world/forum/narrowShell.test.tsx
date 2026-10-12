/** @vitest-environment jsdom */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { FORUM_NARROW_SHELL_PX } from './ForumView';

const CSS = readFileSync(join(dirname(fileURLToPath(new URL(import.meta.url, 'file:///'))), 'forum.css'), 'utf8');

/**
 * QA 2026-10-12 (N43/N47): Agent desk + chat dock left 318 px (1280) / 62 px
 * (1024) of world and the title card ran under the desk toggle. jsdom has no
 * layout, so this pins the structure that fixes it: the desk stacks under the
 * world when the shell is narrow, and the HUD reflows on the world's own width.
 */
describe('narrow forum shell', () => {
  it('stacks an open desk beneath the world instead of beside it', () => {
    expect(FORUM_NARROW_SHELL_PX).toBeGreaterThanOrEqual(700);
    expect(CSS).toMatch(/\.forum-shell\[data-narrow='true'\]\[data-desk='open'\]\s*\{[^}]*grid-template-rows/);
  });
  it('makes the world a size container and reflows the HUD below ~520px of it', () => {
    expect(CSS).toMatch(/\.forum-world\s*\{[^}]*container:forum-stage \/ inline-size/);
    const q = /@container forum-stage \(max-width:520px\)\s*\{([\s\S]*?)\n\}/.exec(CSS);
    expect(q?.[1]).toMatch(/\.forum-brand \.eyebrow\s*\{\s*display:none/);
    expect(q?.[1]).toMatch(/\.habitat-view-tools\s*\{[^}]*flex-direction:column/);
  });
});
