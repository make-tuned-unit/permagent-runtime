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
import type { CSSProperties, ReactNode } from 'react';
import { font, glassSeamShadow, space, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from './Glass';

/**
 * The opaque half of D11's hard scroll edge, as a standalone style fragment.
 *
 * Apple: under pinned chrome the boundary on macOS is "a linear, nearly
 * opaque boundary between pinned controls and scrolling content" — an opaque
 * fill behind the header plus the header's own border, not a see-through bar
 * with a hairline floating over whatever scrolls beneath it. `position:
 * relative` + `zIndex: 1` keep that fill on top of the scrolling content it
 * is meant to hide.
 *
 * `ViewHeader`'s own `surface` prop applies this to its root div. Exported
 * separately for pinned headers that are not built from `<ViewHeader>` at
 * all (the chat dock's own header, `ChatDock.tsx`) — same mechanic, so
 * "views stop doing it themselves" isn't only true for `ViewHeader` callers.
 * The fill color itself is never chosen here: it depends on what sits behind
 * the header on that screen (`colors.bg`, `colors.surface`, a gradient
 * stand-in, …), so callers always pass their own.
 */
/** The bar's reserved height and vertical padding. Exported so a sibling pane's
 *  header (the chat dock's) can take the same numbers: two headers side by
 *  side at different heights read as two apps, not one (ruling 2026-09-21). */
export const VIEW_HEADER_MIN_HEIGHT = 64;
export const VIEW_HEADER_PADDING_Y = space.xxl;

export function hardScrollEdgeSurface(background: string): CSSProperties {
  return { background, position: 'relative', zIndex: 1 };
}

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
  /** Opaque fill that turns this header into D11's hard scroll edge — see
   *  `hardScrollEdgeSurface`. Omit it and the header wears the `glass`
   *  material (the toolbar is the floating control layer, which is where
   *  glass belongs). Pass the view's own fill (`colors.bg`, `colors.surface`,
   *  …) when content can scroll behind it and must be cut off hard. */
  surface?: string;
}

export function ViewHeader({ title, subtitle, leading, afterTitle, actions, surface }: ViewHeaderProps) {
  const { colors, theme } = useTheme();
  // "There is no liquid glass anywhere": the one bar every view wears was
  // either bare (no `surface`) or an opaque fill, so the toolbar — the surface
  // Apple names first for glass — never wore the glass material. With no
  // `surface` it now takes the `glass` token's translucent fill and its rim
  // (specular top edge, hairline, ambient shadow), and `useGlass` flattens it
  // under Reduce Transparency.
  //
  // The token's `backdrop-filter` is deliberately NOT applied. This bar is a
  // fixed sibling ABOVE the scroll container (see below), and every view's
  // wrapper behind it is an opaque gradient, so no content ever passes under
  // it: a blur here would sample a static fill and blur it into itself — the
  // inert, every-frame compositing pass Glass.tsx exists to stamp out, paid on
  // all six tabs for zero visible pixels. The filter belongs back on this bar
  // only on the day a view lets its content scroll underneath it.
  //
  // `surface` still wins: D11's hard edge is a deliberate opaque cut.
  const glass = useGlass('glass');
  // The SEAMED rim, hairline on the bottom only: the token's four-sided ring
  // put a line under the titlebar band and a second one beside the rail's own
  // seam, doubling it in the header band (see `glassSeamShadow`).
  const material: CSSProperties = {
    background: glass.background,
    boxShadow: glassSeamShadow(theme, 'glass', 'bottom'),
  };
  const chrome: CSSProperties = surface
    ? { borderBottom: `1px solid ${colors.border}`, ...hardScrollEdgeSurface(surface) }
    // The rim's hairline is the bottom edge here, so no `borderBottom` beside
    // it; `relative` + `zIndex: 1` let the ambient shadow fall on the content
    // below instead of being painted over by it. (That z-index caps menus that
    // drop out of the bar at level 1. Checked against what sits below them:
    // Home's cards are each their own stacking context at 0 — every card
    // carries a `transform` — so the edit-mode menu still clears the remove
    // buttons; Grow's opaque header has had the same z-index all along.)
    : { ...material, position: 'relative', zIndex: 1 };

  return (
    <div
      data-testid="view-header"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: space.xl,
        // A docked chat pane can leave a top-level view only a few hundred
        // pixels wide. Let title/actions form a second row instead of
        // allowing controls to paint outside the workspace.
        flexWrap: 'wrap',
        padding: `${VIEW_HEADER_PADDING_Y}px ${space.huge}px`,
        // The bar is a fixed sibling ABOVE the scroll container, never inside
        // it — a header that scrolls away takes the view's identity with it.
        flexShrink: 0,
        fontFamily: font.body,
        color: colors.text,
        // Reserve a stable height whether or not a subtitle is present, so
        // switching tabs doesn't shift the content below by a few pixels.
        minHeight: VIEW_HEADER_MIN_HEIGHT,
        boxSizing: 'border-box',
        ...chrome,
      }}
    >
      {leading}

      {/* minWidth:0 is what lets the title actually truncate inside a flex row.
          Not flex:1 when there's an afterTitle control — the title takes only
          its own width so the control sits immediately beside it rather than
          across a gap. */}
      <div
        data-testid="view-title-block"
        style={{ minWidth: 0, flex: afterTitle ? '0 1 auto' : '1 1 220px' }}
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
          <div style={{ ...type.micro, color: colors.textMuted, marginTop: space.xxs }}>{subtitle}</div>
        )}
      </div>

      {afterTitle && <div style={{ flexShrink: 0 }}>{afterTitle}</div>}

      {/* Spacer so afterTitle hugs the title and actions stay at the far edge. */}
      {afterTitle && <div style={{ flex: 1 }} />}

      {actions && (
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: space.md, flex: '0 1 auto', minWidth: 0, maxWidth: '100%', flexWrap: 'wrap' }}>
          {actions}
        </div>
      )}
    </div>
  );
}
