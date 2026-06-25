/**
 * DetailModal — the reusable detail-view shell (#503).
 *
 * Scrim + centered panel + header (title, optional badge, close) + scrollable
 * body + optional footer. Click-scrim and Escape both close. Mirrors the
 * ResetConfirmModal / ConfigureProviderModal inline-modal convention (no shared
 * atom existed before this). Built generic on purpose: the GoalDetailModal is
 * its first consumer, and the CRM People modal (epic slice 2) reuses the same
 * shell for entity detail.
 */

import { useEffect } from 'react';
import type { ReactNode } from 'react';
import { FiX } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  title: string;
  /** Optional status pill shown next to the title (e.g. goal state). */
  badge?: { label: string; color: string; bg: string } | null;
  onClose: () => void;
  /** Pinned action row at the bottom (e.g. Cancel goal). */
  footer?: ReactNode;
  children: ReactNode;
}

export function DetailModal({ title, badge, onClose, footer, children }: Props) {
  const { colors } = useTheme();

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [onClose]);

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
        onClick={e => e.stopPropagation()}
        style={{
          width: 'min(560px, 92vw)', maxHeight: '86vh',
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
          <span style={{
            fontFamily: font.display, fontSize: 14, fontWeight: 600,
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
          <button
            onClick={onClose}
            title="Close"
            style={{
              background: 'none', border: 'none', color: colors.textMuted,
              cursor: 'pointer', padding: 4, display: 'flex',
            }}
          >
            <FiX size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ overflow: 'auto', padding: '16px 18px', flex: 1 }}>
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
