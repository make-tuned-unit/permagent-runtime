/**
 * @vitest-environment jsdom
 *
 * Tooltip primitive — placement flip, reduce-motion, a11y wiring, and the
 * owned-dir fitness gate that keeps raw `title=` from growing back.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

import {
  Tooltip,
  TooltipBubble,
  TOOLTIP_COLD_DELAY_MS,
  _resetTooltipWarmForTests,
  _markTooltipHiddenForTests,
} from './Tooltip';
import { placeViewportTooltip } from './tooltipPlacement';
import { setReduceMotion, setNativeReduceTransparency, setTheme } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  _resetTooltipWarmForTests();
  setReduceMotion(false);
  setNativeReduceTransparency(false);
  setTheme('dark');
  vi.useFakeTimers();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  // Portalled tips land on document.body.
  document.querySelectorAll('[role="tooltip"]').forEach(el => el.remove());
  vi.useRealTimers();
  _resetTooltipWarmForTests();
  setReduceMotion(false);
  setNativeReduceTransparency(false);
});

function tip(): HTMLElement | null {
  return document.querySelector('[role="tooltip"]');
}

describe('placeViewportTooltip', () => {
  const viewport = { width: 800, height: 600 };

  it('flips top → bottom when the trigger sits against the top edge', () => {
    const nearTop = { x: 400, y: 4, width: 40, height: 20 };
    const p = placeViewportTooltip(nearTop, 'top', 8, viewport);
    expect(p.side).toBe('bottom');
    expect(p.top).toBeGreaterThan(nearTop.y);
  });

  it('flips right → left when the trigger sits against the right edge', () => {
    const nearRight = { x: 760, y: 300, width: 40, height: 20 };
    const p = placeViewportTooltip(nearRight, 'right', 8, viewport);
    expect(p.side).toBe('left');
  });

  it('keeps the preferred side when there is room', () => {
    const mid = { x: 400, y: 300, width: 40, height: 20 };
    expect(placeViewportTooltip(mid, 'top', 8, viewport).side).toBe('top');
    expect(placeViewportTooltip(mid, 'right', 8, viewport).side).toBe('right');
  });
});

// ── Measured placement: the box the bubble actually occupies ──────────
//
// jsdom has no layout engine, so nothing here measures a real bubble. What it
// DOES do is give `placeViewportTooltip` a bubble size and check the box that
// falls out of `left`/`top`/`transform` — which is the arithmetic that was
// wrong. Two things must hold for every placement: the box is inside the
// window, and it does not sit on the trigger it labels.

/** Resolve a placement + bubble size into the rect the bubble will occupy. */
function bubbleBox(p: ReturnType<typeof placeViewportTooltip>, tip: { width: number; height: number }) {
  const { left, top, side } = p;
  switch (side) {
    case 'top': return { l: left - tip.width / 2, r: left + tip.width / 2, t: top - tip.height, b: top };
    case 'bottom': return { l: left - tip.width / 2, r: left + tip.width / 2, t: top, b: top + tip.height };
    case 'left': return { l: left - tip.width, r: left, t: top - tip.height / 2, b: top + tip.height / 2 };
    case 'right': return { l: left, r: left + tip.width, t: top - tip.height / 2, b: top + tip.height / 2 };
  }
}

describe('placeViewportTooltip (measured)', () => {
  const viewport = { width: 1280, height: 800 };
  /** A label that wrapped to two lines — the case the 48px guess got wrong. */
  const twoLine = { width: 210, height: 60 };

  function expectInside(box: ReturnType<typeof bubbleBox>) {
    expect(box.l).toBeGreaterThanOrEqual(0);
    expect(box.t).toBeGreaterThanOrEqual(0);
    expect(box.r).toBeLessThanOrEqual(viewport.width);
    expect(box.b).toBeLessThanOrEqual(viewport.height);
  }

  function expectClearOf(box: ReturnType<typeof bubbleBox>, anchor: { x: number; y: number; width: number; height: number }) {
    const overlaps = box.l < anchor.x + anchor.width && box.r > anchor.x
      && box.t < anchor.y + anchor.height && box.b > anchor.y;
    expect(overlaps).toBe(false);
  }

  // The reported bug: the chat dock's header buttons sit ~50px down and flush
  // right. Unmeasured, 50 >= FLIP_ROOM_PX(48) said "top fits", so a two-line
  // label was drawn from y = 50 - 8 - 60 = -18 — up through the header and
  // against the icon row — while its centre-only clamp let it run off the
  // right edge as well.
  it('keeps a wrapped label off the chat header and inside the window', () => {
    const detachButton = { x: 1180, y: 50, width: 28, height: 28 };
    const p = placeViewportTooltip(detachButton, 'top', 8, viewport, twoLine);
    const box = bubbleBox(p, twoLine);
    expectInside(box);
    expectClearOf(box, detachButton);
  });

  it('clamps by the bubble edge, not its midpoint, against the right edge', () => {
    const flushRight = { x: 1240, y: 400, width: 28, height: 28 };
    const p = placeViewportTooltip(flushRight, 'top', 8, viewport, twoLine);
    // Centre-clamping put `left` at 1272 and half a 210px bubble past 1280.
    expect(p.left + twoLine.width / 2).toBeLessThanOrEqual(viewport.width - 8);
    expectInside(bubbleBox(p, twoLine));
  });

  it('does not flip a one-line label down onto the content it covers', () => {
    // 46px of clearance: too little for the 48px guess, ample for a 28px
    // bubble plus its 8px gap and 8px margin. Flipping here dropped the label
    // onto the transcript below the header.
    const oneLine = { width: 180, height: 28 };
    const anchor = { x: 400, y: 46, width: 28, height: 28 };
    const p = placeViewportTooltip(anchor, 'top', 8, viewport, oneLine);
    expect(p.side).toBe('top');
    expectInside(bubbleBox(p, oneLine));
    expectClearOf(bubbleBox(p, oneLine), anchor);
  });

  it('holds the invariants for every side and both edges', () => {
    const sides = ['top', 'bottom', 'left', 'right'] as const;
    const anchors = [
      { x: 0, y: 0, width: 28, height: 28 },
      { x: 1252, y: 0, width: 28, height: 28 },
      { x: 0, y: 772, width: 28, height: 28 },
      { x: 1252, y: 772, width: 28, height: 28 },
      { x: 640, y: 400, width: 28, height: 28 },
    ];
    for (const side of sides) {
      for (const anchor of anchors) {
        const p = placeViewportTooltip(anchor, side, 8, viewport, twoLine);
        const box = bubbleBox(p, twoLine);
        expectInside(box);
        expectClearOf(box, anchor);
      }
    }
  });
});

describe('Tooltip', () => {
  it('wires aria-describedby to a role=tooltip bubble on focus', () => {
    act(() => {
      root.render(
        <Tooltip content="Save the draft">
          <button type="button">Save</button>
        </Tooltip>,
      );
    });
    const btn = container.querySelector('button')!;
    act(() => { btn.focus(); });
    const bubble = tip();
    expect(bubble).toBeTruthy();
    expect(bubble!.textContent).toBe('Save the draft');
    expect(btn.getAttribute('aria-describedby')).toBe(bubble!.id);
    expect(btn.getAttribute('title')).toBeNull();
  });

  it('skips the spring under reduce-motion', () => {
    setReduceMotion(true);
    act(() => {
      root.render(
        <TooltipBubble
          id="t1"
          left={10}
          top={10}
          transform="translate(-50%, -100%)"
          fromTransform="translate(-50%, -100%) translateY(4px)"
        >
          Quiet
        </TooltipBubble>,
      );
    });
    const bubble = tip()!;
    expect(bubble.style.transition).toBe('');
    expect(bubble.style.opacity).toBe('1');
  });

  it('opens after the cold delay on hover, instantly when warm', () => {
    act(() => {
      root.render(
        <Tooltip content="Collapse">
          <button type="button">«</button>
        </Tooltip>,
      );
    });
    const btn = container.querySelector('button')!;
    // React synthesises mouseEnter from mouseover (bubbling).
    act(() => {
      btn.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));
    });
    expect(tip()).toBeNull();
    act(() => { vi.advanceTimersByTime(TOOLTIP_COLD_DELAY_MS); });
    expect(tip()?.textContent).toBe('Collapse');

    act(() => {
      btn.dispatchEvent(new MouseEvent('mouseout', { bubbles: true }));
    });
    expect(tip()).toBeNull();
    _markTooltipHiddenForTests(Date.now());

    act(() => {
      btn.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }));
    });
    expect(tip()?.textContent).toBe('Collapse');
  });

  // The math above is only worth having if the bubble's real box reaches it.
  // jsdom reports 0×0 for everything, so we stand in for the layout engine and
  // then assert on the placement the component committed.
  it('places from the bubble it measured, not from the 48px guess', () => {
    setReduceMotion(true); // resting transform, no spring frame to wait on
    const real = HTMLElement.prototype.getBoundingClientRect;
    const rect = (x: number, y: number, width: number, height: number) => ({
      x, y, width, height, top: y, left: x, right: x + width, bottom: y + height,
      toJSON: () => ({}),
    } as DOMRect);
    // Trigger: a dock-header icon button, ~50px down and flush right.
    // Bubble: "Open chat in its own window" wrapped to two lines.
    HTMLElement.prototype.getBoundingClientRect = function () {
      return this.getAttribute('role') === 'tooltip'
        ? rect(0, 0, 210, 60)
        : rect(980, 50, 28, 28);
    };
    try {
      act(() => {
        root.render(
          <Tooltip content="Open chat in its own window">
            <button type="button">detach</button>
          </Tooltip>,
        );
      });
      act(() => { container.querySelector('button')!.focus(); });
      const bubble = tip()!;
      // 50px of clearance does not hold a 60px bubble: it must go below the
      // button, not up through the header.
      expect(bubble.style.transform).toBe('translate(-50%, 0)');
      expect(parseFloat(bubble.style.top)).toBeGreaterThanOrEqual(50 + 28);
      // ...and the clamp is on the bubble's edge, so its right side stays in.
      expect(parseFloat(bubble.style.left) + 210 / 2).toBeLessThanOrEqual(window.innerWidth - 8);
    } finally {
      HTMLElement.prototype.getBoundingClientRect = real;
    }
  });

  it('dismisses on Escape', () => {
    act(() => {
      root.render(
        <Tooltip content="Close">
          <button type="button">x</button>
        </Tooltip>,
      );
    });
    const btn = container.querySelector('button')!;
    act(() => { btn.focus(); });
    expect(tip()).toBeTruthy();
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(tip()).toBeNull();
  });

  it('collapses glass to opaque under reduce-transparency', () => {
    setNativeReduceTransparency(true);
    act(() => {
      root.render(
        <TooltipBubble id="t2" left={0} top={0} transform="none">
          Opaque
        </TooltipBubble>,
      );
    });
    const bubble = tip()!;
    // Never set → undefined in jsdom (see Glass.test.tsx); must not be a blur.
    expect(bubble.style.backdropFilter).toBeFalsy();
    expect(bubble.style.background).toBeTruthy();
  });
});

// ── Fitness: components/ must not reintroduce native title= tooltips ──

/** Component props named `title` that are headings / labels / tip strings
 *  owned by the child — not OS hover tooltips on the host. */
const TITLE_PROP_COMPONENTS = new Set([
  'ViewHeader',
  'DetailModal',
  'FormModal',
  'ConfirmDialog',
  'Chip', // Chip's `title` prop is the tip string; Tooltip owns the chrome.
  'Section', // Settings' grouped-inset-list header — a heading, not a tooltip.
  'WorkSection', // Agents panel's section header — same shape as Section.
  'StateBlock', // Empty/error block's title is its headline, not a tooltip.
  'SectionTitle', // Dashboard card section heading.
  'Panel', // Projects panel chrome heading.
  'HudShell', // World HUD window title.
  'LinksPanel', // Projects resources panel heading.
  'DisclaimerDialog', // Finance opt-in dialog heading.
  'MeetingsSection', // Person detail meeting-title form field.
  'ToggleChip', // Build pane chip tip string (wraps Tooltip internally).
  'IconButton', // Document viewer glyph tip string (wraps Tooltip internally).
  'GrowthProjectRow', // Growth card row tip string (wraps Tooltip internally).
  'LinkButton', // Person profile link tip string (wraps Tooltip internally).
]);

/**
 * Allowlist for a11y-required titles that are NOT native hover tooltips:
 *  - `<iframe title="…">` — accessible name for the frame
 *  - `<svg><title>…</title></svg>` — accessible name for the graphic
 *  - `<option title="…">` — OS-drawn select popup; cannot host a portal tip
 * Listed explicitly so a future `title=` cannot hide behind a vague exception.
 */
const A11Y_TITLE_ALLOWLIST = [
  { kind: 'iframe-attr' as const, pattern: /<iframe\b[^>]*\btitle=/ },
  { kind: 'svg-title-el' as const, pattern: /<svg[\s\S]*?<title[\s>]/ },
  { kind: 'option-attr' as const, pattern: /<option\b[^>]*\btitle=/ },
];

const SRC_ROOT = join(process.cwd(), 'src/components');
const CC_SRC = join(process.cwd(), 'src');

function walkTsx(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) walkTsx(p, out);
    else if (name.endsWith('.tsx') && !name.endsWith('.test.tsx')) out.push(p);
  }
  return out;
}

function isComponentTitleProp(line: string, prevLines: string[]): boolean {
  // `<ViewHeader title=` / `<DetailModal title=` on same or recent line.
  const window = [line, ...prevLines.slice(-3)].join(' ');
  for (const name of TITLE_PROP_COMPONENTS) {
    if (new RegExp(`<${name}\\b[^>]*\\btitle=`).test(window)) return true;
    if (new RegExp(`<${name}\\b`).test(window) && /^\s*title=/.test(line)) return true;
  }
  // `title={title}` handed to DetailModal inside FormModal / ConfirmDialog.
  if (/title=\{title\}/.test(line) && /DetailModal|ConfirmDialog/.test(window)) return true;
  return false;
}

function isAllowlistedA11y(line: string, fileText: string): boolean {
  for (const a of A11Y_TITLE_ALLOWLIST) {
    if (a.kind === 'iframe-attr' && a.pattern.test(line)) return true;
    if (a.kind === 'option-attr' && a.pattern.test(line)) return true;
    if (a.kind === 'svg-title-el' && /<title[\s>]/.test(line) && /<svg/.test(fileText)) return true;
  }
  return false;
}

describe('components/: zero native title= tooltips', () => {
  it('lists the a11y allowlist so it stays intentional', () => {
    expect(A11Y_TITLE_ALLOWLIST.map(a => a.kind)).toEqual([
      'iframe-attr',
      'svg-title-el',
      'option-attr',
    ]);
  });

  it('has no raw title= left in components/ (or ChatApp)', () => {
    const files: string[] = walkTsx(SRC_ROOT);
    files.push(join(CC_SRC, 'ChatApp.tsx'));

    const offenders: string[] = [];
    for (const file of files) {
      const text = readFileSync(file, 'utf8');
      const lines = text.split('\n');
      const prev: string[] = [];
      for (let i = 0; i < lines.length; i += 1) {
        const line = lines[i];
        const trimmed = line.trim();
        // Comments / block-comment remnants mentioning title=
        if (trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*')) {
          prev.push(line);
          if (prev.length > 4) prev.shift();
          continue;
        }
        if (!/\btitle\s*=/.test(line)) {
          prev.push(line);
          if (prev.length > 4) prev.shift();
          continue;
        }
        // Non-JSX: assignments, comparisons, defaults, document.title.
        if (
          /^\s*title\s*=/.test(line) && !/<\w/.test(line)
          || /\b(const|let|var)\s+title\s*=/.test(line)
          || /\btitle\s*=/.test(line) && /function\s+\w+\s*\(|\(/.test(line) && !/<\w/.test(line)
          || /\.title\s*[=!]/.test(line)
          || /document\.title\s*=/.test(line)
          || /typeof\s+.*\.title/.test(line)
        ) {
          prev.push(line);
          if (prev.length > 4) prev.shift();
          continue;
        }
        // Interface / type fields: `title?: string` / `title: string`
        if (/title\??\s*:/.test(line) && !/title\s*=/.test(line)) {
          prev.push(line);
          continue;
        }
        if (isComponentTitleProp(line, prev) || isAllowlistedA11y(line, text)) {
          prev.push(line);
          continue;
        }
        offenders.push(`${relative(CC_SRC, file)}:${i + 1}: ${line.trim()}`);
        prev.push(line);
        if (prev.length > 4) prev.shift();
      }
    }
    expect(offenders).toEqual([]);
  });
});
