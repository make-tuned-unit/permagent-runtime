/** @vitest-environment jsdom */
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { ForumPlaceRail } from './ForumPlaceRail';
import { DISTRICTS } from './districts';
import { getThemedColors } from '../../../styles/tokens';

/**
 * The user: "a screen-reader user would lose the entire interface."
 *
 * The reported symptom — `entire contents of window 1` collapsing to 0 — turned
 * out to be the measuring instrument, not this app: on macOS 26.2 AppleScript's
 * `entire contents` returns an empty list for every process on the machine,
 * including a seven-item menu bar. The app's AX tree is healthy.
 *
 * The audit that followed did find one real defect of the kind that was being
 * looked for. The closed place rail is still rendered — `railHitTest.test.ts`
 * pins the `translateX(-102%)` / `opacity: 0` that hide it, and neither removes
 * a node from the accessibility tree — so it was held out of that tree by
 * `aria-hidden` alone. But the documented way to close the rail is Escape
 * *while focus is inside it*, and WebKit refuses to honour `aria-hidden` on a
 * subtree that retains focus. The closed rail would come back to a screen
 * reader while staying invisible on screen, with the reading cursor on an
 * off-screen control.
 *
 * WHAT THIS TEST PROVES: that the closed rail carries `inert` as well as
 * `aria-hidden`, and that closing it moves focus back to the handle.
 *
 * WHAT IT DOES NOT PROVE: jsdom implements neither `inert` nor an AX bridge, so
 * it cannot show that focus is actually blocked, that the subtree really leaves
 * the AX tree, or that WebKit stops complaining. It holds the attribute
 * contract that makes the real browser do those things — the part that would
 * otherwise be deleted as redundant, since nothing visible changes without it.
 */

const PLACES = DISTRICTS.map(d => ({ id: d.id, name: d.name, subtitle: d.subtitle }));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function renderRail(open: boolean) {
  if (!host) {
    host = document.createElement('div');
    // Attached to the document, or nothing in here can hold focus.
    document.body.appendChild(host);
    root = createRoot(host);
  }
  act(() => root!.render(
    <ForumPlaceRail
      places={PLACES}
      activeId={PLACES[0].id}
      open={open}
      colors={getThemedColors()}
      onSelect={() => {}}
      onToggle={() => {}}
      onHold={() => {}}
    />,
  ));
  return host.querySelector('#forum-place-rail') as HTMLElement;
}

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
});

describe('the closed place rail leaves the accessibility tree', () => {
  it('pairs aria-hidden with inert when closed', () => {
    const list = renderRail(false);
    expect(list.getAttribute('aria-hidden')).toBe('true');
    // The load-bearing half. `aria-hidden` is advisory and WebKit drops it the
    // moment focus is inside; `inert` is what the browser enforces.
    expect(list.getAttribute('inert')).toBe('');
  });

  it('carries neither while open, so the places stay readable', () => {
    const list = renderRail(true);
    expect(list.getAttribute('aria-hidden')).not.toBe('true');
    expect(list.hasAttribute('inert')).toBe(false);
  });

  it('keeps every place out of the tab order while closed', () => {
    const closed = renderRail(false);
    const tabIndexes = [...closed.querySelectorAll('.forum-rail-place')]
      .map(b => b.getAttribute('tabindex'));
    expect(tabIndexes.length).toBe(PLACES.length);
    expect(new Set(tabIndexes)).toEqual(new Set(['-1']));

    const open = renderRail(true);
    const openTabIndexes = [...open.querySelectorAll('.forum-rail-place')]
      .map(b => b.getAttribute('tabindex'));
    expect(new Set(openTabIndexes)).toEqual(new Set(['0']));
  });

  it('returns focus to the handle when it closes under the keyboard', () => {
    const list = renderRail(true);
    const place = list.querySelector('.forum-rail-place') as HTMLButtonElement;
    act(() => place.focus());
    expect(document.activeElement).toBe(place);

    // Escape-to-close lands here. Without the handback, `inert` blurs the
    // focused place and the keyboard user is dropped on <body> — able to see
    // the world, with no idea where their cursor went.
    renderRail(false);
    expect(document.activeElement).toBe(host!.querySelector('.forum-rail-handle'));
  });

  it('does not steal focus when it closes while focus is elsewhere', () => {
    renderRail(true);
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();

    renderRail(false);
    expect(document.activeElement).toBe(outside);
    outside.remove();
  });
});
