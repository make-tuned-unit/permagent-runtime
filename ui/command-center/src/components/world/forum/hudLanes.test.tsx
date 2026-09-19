/** @vitest-environment jsdom */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
vi.mock('@react-three/fiber', () => ({ Canvas: () => null }));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Aster' }));
vi.mock('./ForumConversation', () => ({ ForumConversation: () => null }));
vi.mock('./forumQuery', () => ({ askFromForum: vi.fn() }));
vi.mock('./ForumCapabilities', () => ({ ForumCapabilities: () => null }));
vi.mock('./ForumAgents', () => ({ ForumAgents: () => null }));
vi.mock('./ForumPassport', () => ({ ForumPassport: () => <div className="forum-passport">passport</div> }));
vi.mock('../agents/goalActivity', () => ({ useActiveGoals: () => ({ goals: [], loaded: false }) }));
import { ForumView } from './ForumView';

/**
 * E2E QA at 1512x945, night and day: "The Places rail, open by default,
 * overlaps other panels with text drawn over text." It hid the title card's
 * "Living forum · Night / follows system appearance", drew "Debating theatre"
 * over the COMMONS place card and "Harbor" over "Job feed unavailable /
 * connecting", and the Sovereign Passport covered part of "Fly the skiff".
 *
 * The cause was not one bad number. Every card in a corner was its own
 * `position: absolute` box with an offset tuned by hand against the others
 * (rail top:96px/bottom:96px over a place card at bottom:125px; passport at
 * bottom:132px over the skiff at bottom:118px), so anything added later — the
 * rail — landed on top of what was already there. The fix puts each corner in
 * one flex column, where two items cannot share space.
 *
 * WHAT THIS PROVES: that the rail and the left-edge cards are siblings in one
 * lane in top-to-bottom order, that the passport, the skiff and Walk share the
 * right lane, and that no stylesheet rule pins any of those items back out of
 * flow with `position: absolute` or a corner offset.
 *
 * WHAT IT DOES NOT PROVE: jsdom has no layout engine, so it cannot measure that
 * the boxes do not intersect at 1512x945. That is the flex column's job in a
 * real browser; this holds the structure that makes it true, which is the part
 * a later "just pin it at bottom: Npx" edit would silently undo.
 */

// Under jsdom `import.meta.url` is not a file: URL, so resolve from the module
// path vitest reports rather than handing readFileSync a URL.
const CSS = readFileSync(join(dirname(fileURLToPath(new URL(import.meta.url, 'file:///'))), 'forum.css'), 'utf8');

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

function laneOf(selector: string): Element | null {
  return host.querySelector(selector)?.parentElement ?? null;
}

describe('the forum HUD lays each corner out as one column', () => {
  it('puts the open place rail in flow between the title card and the bottom cards', () => {
    const rail = host.querySelector('.forum-rail');
    expect(rail?.getAttribute('data-open'), 'the rail is open by default').toBe('open');
    const lane = laneOf('.forum-rail');
    expect(lane?.classList.contains('forum-lane-left')).toBe(true);
    for (const card of ['.forum-brand', '.forum-location', '.forum-live', '.forum-controls']) {
      expect(laneOf(card), `${card} left the rail's lane`).toBe(lane);
    }
    const order = [...lane!.children].map(c => c.className.split(' ')[0]);
    expect(order).toEqual(['forum-brand', 'forum-rail', 'forum-location', 'forum-live', 'forum-controls']);
  });

  it('stacks the passport above the skiff and Walk in one right-hand lane', () => {
    // Henry is selected at the desk by default, so the passport is showing:
    // the case QA saw covering "Fly the skiff".
    const lane = laneOf('.forum-fly');
    expect(lane?.classList.contains('forum-lane-right')).toBe(true);
    expect(laneOf('.forum-passport-slot')).toBe(lane);
    expect(laneOf('.forum-play')).toBe(lane);
    const order = [...lane!.children].map(c => c.className.split(' ')[0]);
    expect(order).toEqual(['forum-passport-slot', 'forum-fly', 'forum-play']);
  });

  it('keeps the lanes as flex columns', () => {
    const block = ruleBodies('.forum-lane').join(';');
    expect(block).toMatch(/flex-direction:\s*column/);
    expect(block).toMatch(/position:\s*absolute/);
    // A flex item's default min-height is its content, fourteen places tall:
    // the rail would push the bottom cards out of the frame instead of
    // scrolling. It needs an explicit, small floor.
    expect(ruleBodies('.forum-rail').join(';')).toMatch(/min-height:\s*\d+px/);
    // ...and a short rail must clip its handle, not hang it over the place card.
    expect(ruleBodies('.forum-rail-handle').join(';')).toMatch(/max-height:\s*100%/);
  });

  it('never pins a lane item back out of flow', () => {
    const members = ['forum-brand', 'forum-rail', 'forum-location', 'forum-live', 'forum-controls',
      'forum-passport-slot', 'forum-agora-return', 'forum-fly', 'forum-play'];
    const offenders: string[] = [];
    for (const { selector, body } of rules()) {
      if (!members.includes(subjectClass(selector) ?? '')) continue;
      if (/position:\s*(absolute|fixed)/.test(body) || /(?:^|[;{\s])(top|left|right|bottom)\s*:/.test(body)) {
        offenders.push(`${selector} { ${body.trim().replace(/\s+/g, ' ')} }`);
      }
    }
    expect(offenders).toEqual([]);
  });
});

/** Innermost `selector { body }` pairs, one per comma-separated selector. */
function rules(): { selector: string; body: string }[] {
  const out: { selector: string; body: string }[] = [];
  const noComments = CSS.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const m of noComments.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    for (const selector of m[1].split(',')) out.push({ selector: selector.trim(), body: m[2] });
  }
  return out;
}

function ruleBodies(selector: string): string[] {
  return rules().filter(r => r.selector === selector).map(r => r.body);
}

/** The class the rule actually styles: `.forum-world .forum-live` → forum-live. */
function subjectClass(selector: string): string | null {
  const last = selector.split(/[\s>+~]+/).filter(Boolean).pop() ?? '';
  const m = /^\.([\w-]+)(?:\[[^\]]*\])*$/.exec(last);
  return m ? m[1] : null;
}
