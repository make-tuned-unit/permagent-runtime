/**
 * DetailModal — the reusable detail-view shell (#503).
 *
 * Scrim + centered panel + header (title, optional badge, close) + scrollable
 * body + optional footer. Click-scrim and Escape both close, focus moves into
 * the panel on open and back to the opener on close, and Tab is contained —
 * the keyboard floor lives HERE rather than in each consumer, so every modal
 * built on this shell inherits it. Mirrors the
 * ResetConfirmModal / ConfigureProviderModal inline-modal convention (no shared
 * atom existed before this). Built generic on purpose: the GoalDetailModal is
 * its first consumer, and the CRM People modal (epic slice 2) reuses the same
 * shell for entity detail.
 *
 * R12 added `placement: 'contained'`, which is what let the last hand-rolled
 * detail shell in the app — the 108-line `PersonDetailShell` drawer — become a
 * caller of this one instead of a second implementation of it.
 */

import { useCallback, useEffect, useId, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { FiX } from 'react-icons/fi';
import { duration, ease, font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Tooltip } from './Tooltip';

interface Props {
  title: string;
  /** Optional status pill shown next to the title (e.g. goal state). */
  badge?: { label: string; color: string; bg: string } | null;
  onClose: () => void;
  /** Pinned action row at the bottom (e.g. Cancel goal). */
  footer?: ReactNode;
  /** Panel width. The default is the house size; the four sizes that already
   *  existed in the app's hand-rolled shells are 340 (a picker), 480 (a form),
   *  720 (the decision inbox) and 1000 (a document). A shell that cannot be
   *  the size its content needs is a shell people write around. */
  width?: number | string;
  /** Panel height, for a body that must fill rather than hug (a PDF frame). */
  height?: number | string;
  /** Ahead of the title in the header — a back arrow, a breadcrumb. */
  headerLeft?: ReactNode;
  /** After the title, before the close — a meta line, a Download. */
  headerRight?: ReactNode;
  /** Overrides on the body box, for a body that draws its own surface (an
   *  image viewer's dark mat) or wants no padding of its own. */
  bodyStyle?: CSSProperties;
  /**
   * Where the panel lives, and therefore whether it is a modal at all.
   *
   * `center` — the default, and what every caller before R12 gets — is the
   * modal: a scrim over the page, the panel centred on it, focus moved in on
   * open, trapped while it is up, and handed back to the opener on close.
   *
   * `contained` is the SAME panel with the modality taken out: no scrim, no
   * focus trap, square corners, filling whatever box the caller puts it in.
   * It is for a DOCK — a detail pane pinned to the window's edge, or docked
   * into the layout beside the list it details — where the surface behind it
   * stays live and reachable. A focus trap there would be a bug and not a
   * feature: it would swallow Tab on a page the user can still see and still
   * use. The dialog role and the label stay; `aria-modal` does not, because on
   * a pane you can Tab out of it would simply be false.
   */
  placement?: 'center' | 'contained';
  /** Overrides on the footer row — the twin of `bodyStyle`, for a footer whose
   *  actions must wrap rather than be squeezed onto one line (a narrow dock),
   *  or that wants its own alignment. */
  footerStyle?: CSSProperties;
  /** Stop Escape here. For a modal opened from a surface that ALSO closes on
   *  Escape — Settings, Automate — where one keypress otherwise dismisses the
   *  dialog and the page behind it together. Those surfaces listen on `window`
   *  and this listens on `document`, so stopping propagation is enough. */
  stopEscapePropagation?: boolean;
  children: ReactNode;
}

/** Everything inside the panel a keyboard can land on. Order is DOM order,
 *  which is the tab order for everything this shell contains. */
const FOCUSABLE = [
  'a[href]', 'button:not([disabled])', 'textarea:not([disabled])',
  'input:not([disabled])', 'select:not([disabled])', '[tabindex]:not([tabindex="-1"])',
].join(',');

export function DetailModal({
  title, badge, onClose, footer, children,
  width = 'min(560px, 92vw)', height, headerLeft, headerRight, bodyStyle, footerStyle,
  placement = 'center', stopEscapePropagation,
}: Props) {
  const { colors, reduceMotion } = useTheme();
  const panelRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const modal = placement === 'center';

  /**
   * Hard scroll edges (D11), for the two bars this shell pins.
   *
   * The header's rule and the footer's rule were drawn unconditionally, which
   * is decoration: on a body that fits, they separate a bar from nothing.
   * Apple's `hard` edge — the macOS default, as against iOS's `soft` fade —
   * appears exactly when content is passing under a floating bar, and this is
   * the shell every detail view in the app is built on, so it is the one place
   * worth teaching the rule.
   */
  const [edges, setEdges] = useState({ top: false, bottom: false });
  const measure = useCallback(() => {
    const el = bodyRef.current;
    if (!el) return;
    const top = el.scrollTop > 0;
    // 1px of slack: a fractional scrollHeight is normal after a zoom or a
    // subpixel layout, and must not read as "there is more below".
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight > 1;
    setEdges(prev => (prev.top === top && prev.bottom === bottom ? prev : { top, bottom }));
  }, []);
  // No dependency array on purpose: the body's content changes shape without
  // this component's props changing (a section loads, a form opens), and the
  // edges have to follow it. `measure` is a no-op when nothing moved, so this
  // settles in one extra render rather than looping.
  useEffect(measure);
  const edge = (on: boolean) => ({
    borderColor: on ? colors.border : 'transparent',
    transition: reduceMotion ? 'none' : `border-color ${duration.fast}ms ${ease.out}`,
  });

  // Focus goes in on open and comes back out on close. Without the first, a
  // keyboard user's focus is still on the page behind the scrim when the dialog
  // appears; without the second, closing drops them at the top of the document
  // rather than at the control they opened it from.
  useEffect(() => {
    // A dock is not modal: it opens beside a page the user is still working in,
    // so pulling focus into it — and pushing focus back out on close — would
    // take the caret out of whatever they were actually doing.
    if (!modal) return;
    const opener = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    // A consumer that autofocuses its own field has already made the better
    // choice — don't take it back.
    if (panel && !panel.contains(document.activeElement)) panel.focus();
    return () => { opener?.focus?.(); };
  }, [modal]);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (stopEscapePropagation) e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab' || !modal) return;

      // Tab containment. A modal that lets focus walk out to the page behind it
      // is operating a screen the user cannot see. A DOCK is the opposite case:
      // the page beside it is visible and live, so Tab has to be able to leave.
      const panel = panelRef.current;
      if (!panel) return;
      const items = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE));
      const active = document.activeElement;
      if (items.length === 0 || !panel.contains(active)) {
        e.preventDefault();
        (items[0] ?? panel).focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && active === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && active === last) { e.preventDefault(); first.focus(); }
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [onClose, stopEscapePropagation, modal]);

  const panel = (
      <div
        ref={panelRef}
        role="dialog"
        aria-modal={modal || undefined}
        aria-labelledby={titleId}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}
        style={{
          ...(modal
            // An explicit height is the cap too, or the default 86vh silently
            // shortens a viewer that asked for more.
            ? {
              width, height, maxHeight: height ?? '86vh',
              borderRadius: radius.lg,
              border: `1px solid ${colors.border}`,
            }
            // A dock fills the box the caller gave it and meets the window
            // edge, so it has no corner of its own to round and only the one
            // edge to draw.
            : { width: '100%', height: '100%', borderLeft: `1px solid ${colors.border}` }),
          background: colors.surface,
          boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
          overflow: 'hidden',
          display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '14px 18px', flexShrink: 0,
          borderBottom: '1px solid', ...edge(edges.top),
        }}>
          {headerLeft}
          <span id={titleId} style={{
            fontFamily: font.display, fontSize: textSize.body, fontWeight: 600,
            color: colors.text, flex: 1, minWidth: 0,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {title}
          </span>
          {badge && (
            <span style={{
              fontFamily: font.mono, fontSize: 10, letterSpacing: '0.04em',
              textTransform: 'uppercase', borderRadius: radius.pill, padding: '2px 9px',
              flexShrink: 0, color: badge.color, background: badge.bg,
            }}>
              {badge.label}
            </span>
          )}
          {headerRight}
          <Tooltip content="Close">
            {/* `.pa-btn` for the hover and press states every control on a Mac
                owes the pointer (D10). It had neither: the only feedback on the
                one control every modal in the app ships was the cursor. */}
            <button
              className="pa-btn"
              onClick={onClose}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': colors.fillHover,
                '--pa-btn-bg-active': colors.fillActive,
                '--pa-btn-pad': '4px',
                '--pa-btn-radius': `${radius.sm}px`,
                display: 'flex',
              } as CSSProperties}
            >
              <FiX size={16} />
            </button>
          </Tooltip>
        </div>

        {/* Body */}
        <div
          ref={bodyRef}
          onScroll={measure}
          style={{ overflow: 'auto', overscrollBehavior: 'contain', padding: '16px 18px', flex: 1, ...bodyStyle }}
        >
          {children}
        </div>

        {/* Footer */}
        {footer && (
          <div style={{
            padding: '12px 18px', flexShrink: 0,
            borderTop: '1px solid', ...edge(edges.bottom),
            display: 'flex', alignItems: 'center', gap: 10, justifyContent: 'flex-end',
            ...footerStyle,
          }}>
            {footer}
          </div>
        )}
      </div>
  );

  // A dock has no scrim, and so no click-outside: there is no "outside" to
  // click when the surface beside it is the thing being worked on.
  if (!modal) return panel;

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 120,
        // `veil` is the token for exactly this layer — the thing glass would
        // sit on — and it is theme-aware, which a flat 50% black is not: on the
        // silver theme it reads as a blackout rather than as a recede.
        background: colors.veil,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      {panel}
    </div>
  );
}
