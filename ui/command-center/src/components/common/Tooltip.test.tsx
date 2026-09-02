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

// ── Fitness: owned dirs must not reintroduce native title= tooltips ──

const OWNED_DIRS = [
  'awareness',
  'inspection',
  'voice',
  'sidebar',
  'common',
  'settings',
  'history',
  'inbox',
];

/** Component props named `title` that are headings / labels, not OS tooltips. */
const TITLE_PROP_COMPONENTS = new Set([
  'ViewHeader',
  'DetailModal',
  'FormModal',
  'ConfirmDialog',
  'Chip', // Chip's `title` prop is the tip string; Tooltip owns the chrome.
  'Section', // Settings' grouped-inset-list header — a heading, not a tooltip.
  'WorkSection', // Agents panel's section header — same shape as Section.
  'StateBlock', // Empty/error block's title is its headline, not a tooltip.
]);

/**
 * Allowlist for a11y-required titles that are NOT native hover tooltips:
 *  - `<iframe title="…">` — accessible name for the frame
 *  - `<svg><title>…</title></svg>` — accessible name for the graphic
 * Listed explicitly so a future `title=` cannot hide behind a vague exception.
 */
const A11Y_TITLE_ALLOWLIST = [
  { kind: 'iframe-attr' as const, pattern: /<iframe\b[^>]*\btitle=/ },
  { kind: 'svg-title-el' as const, pattern: /<svg[\s\S]*?<title[\s>]/ },
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
    if (a.kind === 'svg-title-el' && /<title[\s>]/.test(line) && /<svg/.test(fileText)) return true;
  }
  return false;
}

describe('owned dirs: zero native title= tooltips', () => {
  it('lists the a11y allowlist so it stays intentional', () => {
    expect(A11Y_TITLE_ALLOWLIST.map(a => a.kind)).toEqual([
      'iframe-attr',
      'svg-title-el',
    ]);
  });

  it('has no raw title= left in owned component dirs (or ChatApp)', () => {
    const files: string[] = [];
    for (const d of OWNED_DIRS) {
      files.push(...walkTsx(join(SRC_ROOT, d)));
    }
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
        // Destructured defaults: `title = 'Key'` — not a JSX attribute.
        if (/^\s*title\s*=/.test(line) && !/<\w/.test(line)) {
          prev.push(line);
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
