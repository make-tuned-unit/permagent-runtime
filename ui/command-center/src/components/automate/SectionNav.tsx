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
import { useCallback, useEffect, useState } from 'react';
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
 *  line is the one being read. Also the sections' `scroll-margin-top`. */
export const SECTION_NAV_OFFSET = 56;

export function SectionNav({ items, scrollRoot }: { items: SectionNavItem[]; scrollRoot: RefObject<HTMLElement | null> }) {
  const { colors } = useTheme();
  const material = useGlass('glass');
  const [active, setActive] = useState<string | null>(items[0]?.id ?? null);

  const findSection = useCallback((id: string) =>
    scrollRoot.current?.querySelector<HTMLElement>(`[data-section="${id}"]`) ?? null, [scrollRoot]);

  // Track the section in view from the scroller's own scroll events.
  useEffect(() => {
    const root = scrollRoot.current;
    if (!root) return;
    const onScroll = () => {
      const rootTop = root.getBoundingClientRect().top;
      let current = items[0]?.id ?? null;
      for (const item of items) {
        const el = findSection(item.id);
        if (el && el.getBoundingClientRect().top - rootTop <= SECTION_NAV_OFFSET) current = item.id;
      }
      // At the very bottom the last short section can never reach the line.
      if (root.scrollHeight > root.clientHeight && root.scrollTop + root.clientHeight >= root.scrollHeight - 2) {
        current = items[items.length - 1]?.id ?? current;
      }
      setActive(current);
    };
    root.addEventListener('scroll', onScroll, { passive: true });
    return () => root.removeEventListener('scroll', onScroll);
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
