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
 */

import { useEffect, useId, useRef } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { FiX } from 'react-icons/fi';
import { font, radius, textSize } from '../../styles/tokens';
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
  width = 'min(560px, 92vw)', height, headerLeft, headerRight, bodyStyle, stopEscapePropagation,
}: Props) {
  const { colors } = useTheme();
  const panelRef = useRef<HTMLDivElement>(null);
  const titleId = useId();

  // Focus goes in on open and comes back out on close. Without the first, a
  // keyboard user's focus is still on the page behind the scrim when the dialog
  // appears; without the second, closing drops them at the top of the document
  // rather than at the control they opened it from.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    // A consumer that autofocuses its own field has already made the better
    // choice — don't take it back.
    if (panel && !panel.contains(document.activeElement)) panel.focus();
    return () => { opener?.focus?.(); };
  }, []);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (stopEscapePropagation) e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab') return;

      // Tab containment. A modal that lets focus walk out to the page behind it
      // is operating a screen the user cannot see.
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
  }, [onClose, stopEscapePropagation]);

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 120,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}
        style={{
          // An explicit height is the cap too, or the default 86vh silently
          // shortens a viewer that asked for more.
          width, height, maxHeight: height ?? '86vh',
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
          overflow: 'hidden',
          display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '14px 18px',
          borderBottom: `1px solid ${colors.border}`,
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
            <button
              onClick={onClose}
              style={{
                background: 'none', border: 'none', color: colors.textMuted,
                cursor: 'pointer', padding: 4, display: 'flex',
              }}
            >
              <FiX size={16} />
            </button>
          </Tooltip>
        </div>

        {/* Body */}
        <div style={{ overflow: 'auto', padding: '16px 18px', flex: 1, ...bodyStyle }}>
          {children}
        </div>

        {/* Footer */}
        {footer && (
          <div style={{
            padding: '12px 18px', borderTop: `1px solid ${colors.border}`,
            display: 'flex', alignItems: 'center', gap: 10, justifyContent: 'flex-end',
          }}>
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}
