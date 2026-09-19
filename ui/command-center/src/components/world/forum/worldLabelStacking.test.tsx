/** @vitest-environment jsdom */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
// The real Canvas renders a wrapper div and drei's <Html> portals every
// in-world label into it. Stand the wrapper in with the props ForumView gives
// it, so the test sees the same host element the labels land in.
vi.mock('@react-three/fiber', () => ({
  Canvas: ({ className }: { className?: string }) => <div className={className} data-testid="forum-canvas-host" />,
}));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Aster' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./ForumAgents', () => ({ ForumAgents: () => null }));
vi.mock('./ForumPassport', () => ({ ForumPassport: () => <div className="forum-passport">passport</div> }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { ForumView } from './ForumView';

/**
 * E2E QA, standalone WebKit night render: the in-world place tags
 * ("Automate", "Build", "Council garden", "Gallery") drew on top of the HUD.
 * "Automate" sat over the Maker hall row of the Places rail and "Build" over
 * "Architecture preview" in the "Job feed unavailable / connecting" card.
 *
 * Cause: the tags are drei <Html> overlays, portalled into the Canvas wrapper
 * div with drei's default zIndexRange [16777271, 0]. That wrapper formed no
 * stacking context, so each tag's z-index competed with the HUD lanes (3-4)
 * in `.forum-world` and won. The fix makes the canvas host an isolated
 * stacking context at z-index 0, before the HUD in DOM order.
 *
 * WHAT THIS PROVES: the canvas host carries `forum-canvas`; that class makes a
 * stacking context (isolation: isolate) with z-index 0; the HUD lanes and the
 * other HUD controls come after it and rank at or above it. By CSS stacking
 * rules that puts every <Html> label, whatever z-index it asks for, beneath
 * the rail and cards.
 *
 * WHAT IT DOES NOT PROVE: jsdom has no layout or painting, so it cannot show
 * the pixels of a tag being occluded. That needs the real WebKit render.
 */

const CSS = readFileSync(join(dirname(fileURLToPath(new URL(import.meta.url, 'file:///'))), 'forum.css'), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '');

function ruleBody(selector: string): string {
  const bodies: string[] = [];
  for (const m of CSS.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (m[1].split(',').some(s => s.trim() === selector)) bodies.push(m[2]);
  }
  return bodies.join(';');
}

function zIndexOf(selector: string): number | null {
  const m = /z-index:\s*(-?\d+)/.exec(ruleBody(selector));
  return m ? Number(m[1]) : null;
}

let root: Root;
let host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => {
  try { localStorage.clear(); } catch { /* no storage */ }
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  act(() => root.render(<ForumView />));
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

describe('in-world labels stack beneath the forum HUD', () => {
  it('gives the canvas host the class that contains the labels', () => {
    const canvasHost = host.querySelector('[data-testid="forum-canvas-host"]');
    expect(canvasHost, 'the canvas host rendered').not.toBeNull();
    expect(canvasHost!.classList.contains('forum-canvas')).toBe(true);
  });

  it('makes the canvas host an isolated stacking context at the floor', () => {
    const body = ruleBody('.forum-canvas');
    expect(body, '.forum-canvas must create its own stacking context').toMatch(/isolation:\s*isolate/);
    expect(zIndexOf('.forum-canvas')).toBe(0);
  });

  it('puts the HUD after the canvas host and at or above it', () => {
    const world = host.querySelector('.forum-world')!;
    const canvasHost = host.querySelector('.forum-canvas')!;
    for (const sel of ['.forum-lane-left', '.forum-lane-right', '.forum-desk-toggle']) {
      const el = world.querySelector(sel);
      expect(el, `${sel} rendered`).not.toBeNull();
      // Same parent, later in the tree: an equal z-index still paints on top.
      expect(el!.parentElement).toBe(canvasHost.parentElement);
      expect(canvasHost.compareDocumentPosition(el!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(zIndexOf(sel) ?? 0, `${sel} ranks below the canvas`).toBeGreaterThanOrEqual(0);
    }
  });
});
