/**
 * Sticky section jump bar for the Automate page.
 *
 * The page is one long scroll — health, agents at work, recent activity,
 * automations, learned, installed — and the only way to Learned was to scroll
 * past everything above it. This pins a row of section buttons to the top of
 * the scroller; each one scrolls its section into view, and the one whose
 * section you are reading is marked current.
 *
 * Sections are found by `data-section="<id>"` inside `scrollRoot`, so the page
 * keeps rendering them where it always did.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, RefObject } from 'react';
import { font, getReduceMotion, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { Button } from '../common/Button';

export interface SectionNavItem {
  id: string;
  label: string;
  count?: number;
}

/** Height of the bar plus breathing room: a section whose top has passed this
 *  line is the one being read. Also the sections' `scroll-margin-top`, and
 *  the top edge of the IntersectionObserver's reading band below. */
export const SECTION_NAV_OFFSET = 56;

/**
 * Pure: which nav item is "current" given which sections are presently
 * inside the reading band (see the observer's `rootMargin` below), in NO
 * particular order.
 *
 * QA 2026-09-25: the nav lit "Automations" at scroll top. Root cause was
 * TWO bugs at once, both fixed by making this its own pure function instead
 * of scroll-math folded into a `useState` initializer:
 *   1. `useState(() => items[0]?.id)` only runs ONCE, at first mount — on
 *      that very first render, before any data had loaded, "Automations"
 *      (rendered while `jobsLoading`) genuinely WAS `items[0]`. Once
 *      "Running Now" and "Recent Activity" appeared above it, nothing ever
 *      told the nav its own `items[0]` had changed.
 *   2. Nothing ran the calculation again on mount/resize — only on a
 *      `scroll` event, so a page that loads already at the top (no scroll
 *      ever fires) kept whatever the initializer guessed forever.
 * This function has no such staleness: called with an empty/no-match
 * `intersecting` set (nothing has entered the band — true at scroll top,
 * before the page has scrolled at all) it always resolves to the FIRST item
 * in `items`, in `items`' own order, never a stale leftover.
 */
export function pickActiveSection(items: SectionNavItem[], intersecting: ReadonlySet<string> | readonly string[]): string | null {
  if (items.length === 0) return null;
  const inBand = intersecting instanceof Set ? intersecting : new Set(intersecting);
  return items.find(i => inBand.has(i.id))?.id ?? items[0].id;
}

export function SectionNav({ items, scrollRoot }: { items: SectionNavItem[]; scrollRoot: RefObject<HTMLElement | null> }) {
  const { colors } = useTheme();
  const material = useGlass('glass');
  const [active, setActive] = useState<string | null>(() => pickActiveSection(items, []));
  const intersectingRef = useRef<Set<string>>(new Set());

  const findSection = useCallback((id: string) =>
    scrollRoot.current?.querySelector<HTMLElement>(`[data-section="${id}"]`) ?? null, [scrollRoot]);

  // Whenever the section LIST changes (data loads, a section appears or
  // disappears), recompute immediately against whatever is currently known
  // to be in the band — not on the next scroll, which may never come.
  useEffect(() => { setActive(pickActiveSection(items, intersectingRef.current)); }, [items]);

  // A real scrollspy, not scroll-position arithmetic: each section is
  // observed against a thin horizontal band starting `SECTION_NAV_OFFSET`px
  // below the scroller's own top (clearing the sticky bar itself) and
  // ending most of the way down the viewport, so "current" changes as a
  // section's top crosses that line — the same behaviour a real reader's
  // eye tracks. `root: scrollRoot.current` scopes it to the page's own
  // scroller, not the window (this page's scroller is a nested div).
  useEffect(() => {
    const root = scrollRoot.current;
    if (!root || typeof IntersectionObserver === 'undefined') return;
    const elements = items
      .map(item => ({ id: item.id, el: findSection(item.id) }))
      .filter((x): x is { id: string; el: HTMLElement } => !!x.el);
    if (elements.length === 0) return;

    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const match = elements.find(x => x.el === entry.target);
        if (!match) continue;
        if (entry.isIntersecting) intersectingRef.current.add(match.id);
        else intersectingRef.current.delete(match.id);
      }
      setActive(pickActiveSection(items, intersectingRef.current));
    }, { root, rootMargin: `-${SECTION_NAV_OFFSET}px 0px -65% 0px`, threshold: 0 });

    for (const { el } of elements) observer.observe(el);
    return () => observer.disconnect();
  }, [scrollRoot, items, findSection]);

  const jump = (id: string) => {
    const el = findSection(id);
    // jsdom has no scrollIntoView; a real webview always does.
    el?.scrollIntoView?.({ block: 'start', behavior: getReduceMotion() ? 'auto' : 'smooth' });
    setActive(id);
  };

  if (items.length < 2) return null;
  return (
    <nav
      aria-label="Automate sections"
      data-testid="automate-section-nav"
      style={{
        position: 'sticky', top: 0, zIndex: 2,
        display: 'flex', flexWrap: 'wrap', gap: space.xs,
        padding: `${space.sm}px ${space.md}px`, marginBottom: space.xxl,
        borderRadius: radius.md, border: `1px solid ${colors.border}`,
        ...material,
        // Opaque, after the glass spread: the bar floats over list rows as they
        // scroll, and a translucent fill let their text collide with the tabs
        // (QA 2026-10-07 Q21). Chrome over scrolling content must be solid.
        background: colors.surface,
        WebkitBackdropFilter: 'none',
        backdropFilter: 'none',
      }}
    >
      {items.map(item => {
        const on = item.id === active;
        return (
          <Button
            key={item.id}
            colors={colors}
            variant={on ? 'ghostOn' : 'ghost'}
            aria-current={on ? 'true' : undefined}
            onClick={() => jump(item.id)}
            flashSuccess={false}
            minPendingMs={0}
            style={{
              '--pa-btn-bg': on ? colors.cyanSoft : 'transparent',
              '--pa-btn-fg': on ? colors.text : colors.textMuted,
              '--pa-btn-border': 'transparent',
              '--pa-btn-bg-hover': on ? colors.cyanSoft : colors.fillHover,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-pad': `${space.xs}px ${space.lg}px`,
              '--pa-btn-radius': `${radius.sm}px`,
              '--pa-btn-weight': on ? 600 : 500,
              fontFamily: font.body,
              fontSize: textSize.caption,
              gap: space.sm,
            } as CSSProperties}
          >
            {item.label}
            {!!item.count && (
              <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, fontVariantNumeric: 'tabular-nums' }}>{item.count}</span>
            )}
          </Button>
        );
      })}
    </nav>
  );
}
