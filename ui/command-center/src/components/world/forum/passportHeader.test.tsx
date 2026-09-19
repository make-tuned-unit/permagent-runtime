/** @vitest-environment jsdom */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

// The card polls chitin.id through the shared store. Stand the store in with
// each status the header can show, so no network is touched.
const store = {
  data: null as null | Record<string, unknown>,
  loading: false,
  connectivity: 'ok' as 'ok' | 'offline',
  lastSuccessfulFetch: null as Date | null,
  startPolling: () => {},
  stopPolling: () => {},
};
vi.mock('../../../stores/identityStore', () => ({
  useIdentityStore: (pick: (s: typeof store) => unknown) => pick(store),
}));
vi.mock('../shared/useOrchestratorName', () => ({ useOrchestratorName: () => 'Hank' }));
import { ForumPassport } from './ForumPassport';

/**
 * E2E QA, WebKit night render of World / Solar Forum: the Sovereign Passport
 * header collided. "SOVEREIGN PASSPORT" and "NOT YET READ" filled one line
 * edge to edge, with "AGENT" squeezed between and under them.
 *
 * Cause: the header was one flex row that could not wrap, and the status was
 * `white-space: nowrap`, so whenever the two phrases did not fit, the eyebrow
 * was the only thing that could break. That row also sets the card's width
 * (the right lane shrinks to fit, up to the 300px slot), so the two phrases
 * always touched, and "LAST KNOWN · OFFLINE" is wider than the slot. ("AGENT"
 * itself is the sovereign's in-world nameplate, a drei <Html> label; the
 * stacking half of that is covered in worldLabelStacking.test.tsx.)
 *
 * WHAT THIS PROVES: for every status the card can show, the header holds
 * exactly two phrases; the row may wrap; neither phrase can break inside
 * itself; and the status keeps to the right on a shared line while dropping
 * whole to its own line when there is no room. Under CSS flex rules that means
 * the two phrases can never share or overlap the same space.
 *
 * WHAT IT DOES NOT PROVE: jsdom has no layout engine, so it measures no text
 * widths and cannot show where the wrap falls. That needs the real WebKit
 * render.
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

let root: Root;
let host: HTMLDivElement;
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
beforeEach(() => {
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

const STATES: Array<[string, () => void]> = [
  ['not yet read', () => { store.lastSuccessfulFetch = null; store.loading = false; }],
  ['reading', () => { store.lastSuccessfulFetch = null; store.loading = true; }],
  ['offline', () => { store.lastSuccessfulFetch = new Date(); store.connectivity = 'offline'; }],
  ['verified', () => { store.lastSuccessfulFetch = new Date(); store.connectivity = 'ok'; store.data = { soulValid: true }; }],
];

describe('the passport header never runs its two phrases together', () => {
  it.each(STATES)('renders exactly the eyebrow and the status (%s)', (_label, set) => {
    set();
    act(() => root.render(<ForumPassport />));
    const head = host.querySelector('.forum-passport-head')!;
    expect(head, 'header rendered').not.toBeNull();
    const kids = Array.from(head.children);
    expect(kids.map(k => k.className)).toEqual(['eyebrow', 'forum-passport-status']);
    expect(kids[0].textContent).toBe('SOVEREIGN PASSPORT');
    expect(kids[1].textContent!.length).toBeGreaterThan(0);
  });

  it('lets the header row wrap between the two phrases', () => {
    const body = ruleBody('.forum-passport-head');
    expect(body).toMatch(/display:\s*flex/);
    expect(body, 'the row must be allowed to wrap').toMatch(/flex-wrap:\s*wrap/);
    expect(body, 'a wrapped status needs space above it').toMatch(/row-gap:\s*\d/);
  });

  it('keeps each phrase whole, so a break falls between them and not inside one', () => {
    expect(ruleBody('.forum-passport-head > .eyebrow')).toMatch(/white-space:\s*nowrap/);
    expect(ruleBody('.forum-passport-status')).toMatch(/white-space:\s*nowrap/);
  });

  it('keeps the status on the right whether it shares the line or has its own', () => {
    // `margin-left: auto` right-aligns it beside the eyebrow and, once it has
    // wrapped, still right-aligns it on its own line; space-between would
    // throw a lone wrapped item to the left edge under the eyebrow.
    expect(ruleBody('.forum-passport-status')).toMatch(/margin-left:\s*auto/);
  });
});
