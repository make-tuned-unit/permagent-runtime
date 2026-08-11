/**
 * ViewHeader — the one header every top-level view wears.
 *
 * Before this, the four main tabs each invented their own: Projects at 16px
 * with -0.01em tracking, Build at 14px, Automate at 20px with no tracking, and
 * Home with no title at all. They also disagreed structurally — Projects and
 * Build put a fixed bar above the scroll area, while Automate's header lived
 * INSIDE the scrolling content and slid away as you scrolled.
 *
 * Sizes come from the `type` ramp in styles/tokens, which a design audit added
 * under the heading "no arbitrary sizes" and which had zero callers until now.
 * A view's name is its page title, so it takes `type.title`; the supporting
 * line takes `type.micro`. Nothing here hardcodes a font size — retune the ramp
 * and every view follows.
 */
import type { ReactNode } from 'react';
import { font, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export interface ViewHeaderProps {
  /** The view's name — always the tab you are on, so the header answers
   *  "where am I". Live subjects (Build's active task) belong in `subtitle`;
   *  a title that swaps out makes the one view that can't tell you that.
   *  Truncates rather than wrapping the bar to two rows. */
  title: ReactNode;
  /** Supporting line: a count, a status, a hint. Omit when there is nothing
   *  honest to say — an empty line is worse than none. */
  subtitle?: ReactNode;
  /** Fixed-size element before the text (e.g. Build's Mobius). */
  leading?: ReactNode;
  /** A control that sits immediately to the RIGHT of the title, on the same
   *  baseline — for a per-view switcher that belongs with the title rather than
   *  pinned to the far edge (Build's project picker). Kept left of the flex
   *  spacer so it stays next to the name instead of drifting right. */
  afterTitle?: ReactNode;
  /** Controls pinned to the right edge (search, buttons, indicators). */
  actions?: ReactNode;
}

export function ViewHeader({ title, subtitle, leading, afterTitle, actions }: ViewHeaderProps) {
  const { colors } = useTheme();

  return (
    <div
      data-testid="view-header"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '16px 24px',
        borderBottom: `1px solid ${colors.border}`,
        // The bar is a fixed sibling ABOVE the scroll container, never inside
        // it — a header that scrolls away takes the view's identity with it.
        flexShrink: 0,
        fontFamily: font.body,
        color: colors.text,
        // Reserve a stable height whether or not a subtitle is present, so
        // switching tabs doesn't shift the content below by a few pixels.
        minHeight: 64,
        boxSizing: 'border-box',
      }}
    >
      {leading}

      {/* minWidth:0 is what lets the title actually truncate inside a flex row.
          Not flex:1 when there's an afterTitle control — the title takes only
          its own width so the control sits immediately beside it rather than
          across a gap. */}
      <div
        data-testid="view-title-block"
        style={{ minWidth: 0, flex: afterTitle ? '0 1 auto' : 1 }}
      >
        <div
          data-testid="view-title"
          style={{
            ...type.title,
            fontFamily: font.display,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {title}
        </div>
        {subtitle !== undefined && subtitle !== null && (
          <div style={{ ...type.micro, color: colors.textMuted, marginTop: 2 }}>{subtitle}</div>
        )}
      </div>

      {afterTitle && <div style={{ flexShrink: 0 }}>{afterTitle}</div>}

      {/* Spacer so afterTitle hugs the title and actions stay at the far edge. */}
      {afterTitle && <div style={{ flex: 1 }} />}

      {actions && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexShrink: 0 }}>
          {actions}
        </div>
      )}
    </div>
  );
}
