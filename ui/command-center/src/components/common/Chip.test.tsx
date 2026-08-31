/** @vitest-environment jsdom
 *
 * `Chip` exists to make one specific lie unwriteable.
 *
 * The World's HUD pills are the app's densest cluster of status labels, and
 * until now a permanently-fixed capability label ("LOCAL", which is a fact
 * about where the Reader runs) was pixel-identical to a live one ("SWEEPING",
 * which is a claim that something is happening right now). A reader has no way
 * to tell which pills are watching something and which are just words.
 *
 * So `kind` is required, and the kinds must not look alike. These tests hold
 * that line: a static chip carries no liveness cue and cannot be made to
 * animate, a state chip does carry one, and the two never render the same.
 */

import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { Chip } from './Chip';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

function render(node: React.ReactElement): HTMLElement {
  act(() => root.render(node));
  return container.firstElementChild as HTMLElement;
}

const dot = (el: HTMLElement) => el.querySelector('[data-testid="chip-dot"]');

describe('Chip — the liveness distinction', () => {
  it('gives a live status a liveness cue and a fixed label none', () => {
    const live = render(<Chip kind="state" tone="success">SWEEPING</Chip>);
    const liveDot = dot(live)!;
    const liveBg = live.style.background;
    expect(liveDot).toBeTruthy();

    const fixed = render(<Chip kind="static" tone="success">LOCAL</Chip>);
    expect(dot(fixed)).toBeNull();
    // Not only the dot: a fixed label is not filled like a live one, so the
    // two are distinguishable at a glance in a row of pills.
    expect(fixed.style.background).not.toBe(liveBg);
  });

  it('says in words which kind it is, for anyone not reading the shape', () => {
    expect(render(<Chip kind="static">LOCAL</Chip>).title)
      .toContain('not a live status');
    expect(render(<Chip kind="static" title="Runs on this machine">LOCAL</Chip>).title)
      .toBe('Runs on this machine');
  });

  it('animates only a live chip that asked to, and never under reduce motion', () => {
    expect(render(<Chip kind="state" pulse>SWEEPING</Chip>).querySelector('.status-pulse'))
      .toBeTruthy();
    // Not by default: a pulse is a claim that something is happening NOW.
    expect(render(<Chip kind="state">ON WATCH</Chip>).querySelector('.status-pulse'))
      .toBeNull();

    localStorage.setItem('permagent-reduce-motion', 'true');
    expect(render(<Chip kind="state" pulse>SWEEPING</Chip>).querySelector('.status-pulse'))
      .toBeNull();
  });

  it('dates a live chip that knows when it was last confirmed', () => {
    const el = render(
      <Chip kind="state" asOf={Date.now() - 2 * 60_000}>ON WATCH</Chip>,
    );
    expect(el.title).toContain('2m ago');
  });
});

describe('Chip — the other kinds', () => {
  it('makes a link a destination, never a toggle', () => {
    const onClick = vi.fn();
    const el = render(<Chip kind="link" onClick={onClick}>Acme Deal</Chip>);
    expect(el.tagName).toBe('BUTTON');
    // A chip that navigates has no on/off state, and claiming one is the same
    // class of lie as a fixed label that pulses.
    expect(el.getAttribute('aria-pressed')).toBeNull();
    expect(dot(el)).toBeNull();
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('makes a filter a real toggle', () => {
    const el = render(<Chip kind="filter" pressed onClick={() => {}}>people</Chip>);
    expect(el.tagName).toBe('BUTTON');
    expect(el.getAttribute('aria-pressed')).toBe('true');
    expect(dot(el)).toBeNull();
  });

  it('sets counts in figures that do not reflow as they change', () => {
    const el = render(<Chip kind="count">128</Chip>);
    expect(el.style.fontVariantNumeric).toBe('tabular-nums');
    expect(dot(el)).toBeNull();
  });

  it('is not a button unless it does something', () => {
    expect(render(<Chip kind="state">ON WATCH</Chip>).tagName).toBe('SPAN');
    expect(render(<Chip kind="static">LOCAL</Chip>).tagName).toBe('SPAN');
    expect(render(<Chip kind="count">12</Chip>).tagName).toBe('SPAN');
  });
});
