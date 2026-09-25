import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { font, radius, space, tabularNums, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export function SectionTitle({ title, right }: { title: string; right?: string }) {
  const { colors } = useTheme();
  // 16px/600 with 14px of air below is a page heading, not a card heading. On a
  // dashboard the CONTENT is the subject and every card repeating a large title
  // is chrome competing with data. Sized down to a quiet label.
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: space.lg }}>
      <h3 style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, letterSpacing: '-0.005em', color: colors.text, margin: 0 }}>{title}</h3>
      {right && <span style={{ fontFamily: font.body, fontSize: 10.5, color: colors.textDim, marginLeft: 'auto' }}>{right}</span>}
    </div>
  );
}

/**
 * The quiet state a card shows when it has nothing to report.
 *
 * Deliberately NOT vertically centred in the card. Centring is why an empty
 * dashboard reads as broken: six cards each park one sentence in the middle of
 * 360px of nothing, and the eye has no idea where to rest. An empty card
 * should occupy the top of its box, state the fact in one line, and — where
 * there is one — offer the action that would fill it (the guidance is "tell
 * them what to do next", not "show a void").
 */
export function EmptyNote({ children, hint }: { children: React.ReactNode; hint?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ paddingTop: space.xxs }}>
      <div style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.4 }}>
        {children}
      </div>
      {hint && (
        <div style={{ fontFamily: font.body, fontSize: 10.5, color: colors.textDim, marginTop: space.xxs, lineHeight: 1.4 }}>
          {hint}
        </div>
      )}
    </div>
  );
}

/**
 * Compact statistic. The 32px number with a 6px-gapped uppercase label was
 * built for a card with room to spare; at dashboard density it is the single
 * biggest consumer of vertical space for the least information.
 */
export function StatCompact({ label, value, cyan, delta }: {
  label: string; value: string | number; cyan?: boolean; delta?: string;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{
        fontFamily: font.body, fontSize: 9.5, fontWeight: 600, letterSpacing: '0.08em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: space.xxs,
        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
      }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: space.xs }}>
        <span style={{
          fontFamily: font.display, fontSize: 19, fontWeight: 600, lineHeight: 1.15,
          letterSpacing: '-0.015em', ...tabularNums,
          color: cyan ? colors.cyan : colors.text,
        }}>{value}</span>
        {delta && <span style={{ fontSize: 10, fontWeight: 600, color: colors.success }}>{delta}</span>}
      </div>
    </div>
  );
}

export function Stat({ label, value, suffix, delta, cyan }: {
  label: string; value: string | number; suffix?: string; delta?: string; cyan?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <div>
      <div style={{ fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.10em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: space.sm }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: space.md }}>
        <div style={{ fontFamily: font.display, fontSize: textSize.display, fontWeight: 600,
          letterSpacing: '-0.02em', ...tabularNums,
          color: cyan ? colors.cyan : colors.text }}>
          {value}<span style={{ fontSize: 18, color: colors.textMuted, marginLeft: space.xxs }}>{suffix || ''}</span>
        </div>
        {delta && (
          <div style={{ fontSize: textSize.micro, fontWeight: 600, color: colors.success }}>{delta}</div>
        )}
      </div>
    </div>
  );
}

export function StatusIcon({ state }: { state: string }) {
  const { colors } = useTheme();
  const successBg = colors.success + '26'; // ~15% opacity
  const dangerBg = colors.danger + '1f'; // ~12% opacity
  const config: Record<string, { bg: string; color: string; icon: string }> = {
    completed: { bg: successBg, color: colors.success, icon: '✓' },
    paused: { bg: dangerBg, color: colors.danger, icon: '⏸' },
    awaiting_input: { bg: colors.cyanSoft, color: colors.cyan, icon: '?' },
  };
  const c = config[state] || config.completed;
  return (
    <div style={{
      width: 24, height: 24, borderRadius: '50%', background: c.bg,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      fontFamily: font.body, fontSize: textSize.caption, color: c.color, flexShrink: 0,
    }}>{c.icon}</div>
  );
}

/** Height of the fade over the last visible line when more is below. */
const SCROLL_FADE_PX = 28;

/**
 * Rows a card body counts for its "+N more" pill. Mark each list row with
 * `data-card-row`; a body with no marked rows says "More" instead of a count.
 */
export const CARD_ROW_ATTR = 'data-card-row';

/**
 * How many marked rows end below the visible bottom of `el`. A row cut in half
 * counts: it is exactly the one the reader cannot finish.
 */
export function rowsBelowFold(el: HTMLElement): number {
  const bottom = el.getBoundingClientRect().bottom;
  let n = 0;
  el.querySelectorAll(`[${CARD_ROW_ATTR}]`).forEach(row => {
    if (row.getBoundingClientRect().bottom > bottom + 0.5) n++;
  });
  return n;
}

/**
 * The one scrolling body every Home card's list sits in.
 *
 * Each card used to roll its own `flex: 1; overflow: auto` div. The grid sets
 * a card's height, not its content, so the last row landed wherever the tile
 * ended — To-dos showed "Refresh pricing and venture landing pages" sliced
 * through the middle of its glyphs, with nothing saying there was more below
 * (QA walk, 1456px). Fixed here once instead of per card: while content
 * continues below, the bottom edge fades out (so a cut line reads as "keeps
 * going", not as broken) and a "+N more" pill counts the rows still hidden and
 * scrolls to them. At the end of the list both go away.
 */
export function CardScrollBody({ children, style, testId, className }: {
  children: ReactNode;
  style?: CSSProperties;
  testId?: string;
  className?: string;
}) {
  const { colors } = useTheme();
  const ref = useRef<HTMLDivElement>(null);
  // null = everything fits; 0 = overflowing with no marked rows; n = n rows.
  const [hidden, setHidden] = useState<number | null>(null);

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const more = el.scrollHeight - el.clientHeight - el.scrollTop > 1;
    const next = more ? rowsBelowFold(el) : null;
    setHidden(prev => (prev === next ? prev : next));
  }, []);

  // Every render: rows arrive, collapse and reorder without a size change.
  useLayoutEffect(measure);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => ro.disconnect();
  }, [measure]);

  const fade = hidden !== null
    ? `linear-gradient(to bottom, #000 calc(100% - ${SCROLL_FADE_PX}px), transparent)`
    : undefined;

  return (
    <div style={{ position: 'relative', flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <div
        ref={ref}
        data-testid={testId}
        className={className}
        data-overflowing={hidden !== null ? 'true' : undefined}
        onScroll={measure}
        style={{
          flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden',
          maskImage: fade, WebkitMaskImage: fade,
          ...style,
        }}
      >
        {children}
      </div>
      {hidden !== null && (
        <Button
          colors={colors}
          variant="bare"
          type="button"
          flashSuccess={false}
          data-testid={testId ? `${testId}-more` : undefined}
          onClick={() => {
            const el = ref.current;
            if (!el) return;
            const top = el.scrollTop + Math.max(el.clientHeight - SCROLL_FADE_PX, SCROLL_FADE_PX);
            if (typeof el.scrollTo === 'function') el.scrollTo({ top, behavior: 'smooth' });
            else el.scrollTop = top;
          }}
          style={{
            '--pa-btn-bg': colors.surfaceHi,
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-border': colors.border,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-pad': `${space.xxs}px ${space.md}px`,
            '--pa-btn-radius': `${radius.md}px`,
            '--pa-btn-weight': 600,
            // Bottom-RIGHT on the fade, not centred over content: centred, the
            // pill sat on top of whichever row's text happened to be under the
            // middle column — on a wide card that is real text, not empty fade
            // (dagK coordinator, 2560px pass). The fade covers the full width,
            // so a corner affordance still reads as "there's more" without
            // masking anything a reader is mid-line on.
            position: 'absolute', bottom: space.xxs, right: space.xxs,
            fontFamily: font.body, fontSize: textSize.micro, whiteSpace: 'nowrap',
            boxShadow: colors.elevationRaised,
          } as CSSProperties}
        >
          {hidden > 0 ? `+${hidden} more ↓` : 'More ↓'}
        </Button>
      )}
    </div>
  );
}
