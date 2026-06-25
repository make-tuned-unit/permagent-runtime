/**
 * EntityDetailModal — generic, reusable detail-modal shell.
 *
 * A purely presentational scrim + panel: header (title + optional subtitle +
 * close), a scrollable body (caller-supplied sections), and a footer action row.
 * It holds NO entity-specific logic — the caller supplies the title, the body
 * content, and the footer buttons. The goal-detail modal (#503) and the CRM
 * People-modal (CRM epic slice 2) both fill this same shell with different
 * content, so the chrome stays consistent and is debugged once.
 *
 * Pattern mirrors the DecisionInbox overlay (scrim click closes, inner click
 * stops propagation) and the AddCardPicker mount convention.
 */

import type { ReactNode } from 'react';
import { useEffect } from 'react';
import { FiX } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

/** A labelled section in the modal body. `mono` renders the value monospaced. */
export function DetailSection({ label, children }: { label: string; children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{
        fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase',
        letterSpacing: '0.06em', color: colors.textDim, marginBottom: 6,
      }}>
        {label}
      </div>
      <div style={{ fontSize: 13, color: colors.text, fontFamily: font.body, lineHeight: 1.5 }}>
        {children}
      </div>
    </div>
  );
}

interface Props {
  /** Modal title (entity headline). */
  title: ReactNode;
  /** Optional one-line subtitle under the title (e.g. type + id chip). */
  subtitle?: ReactNode;
  /** Scrollable body — caller-supplied sections (e.g. DetailSection rows). */
  children: ReactNode;
  /** Footer action row (caller-supplied buttons). Omit for a view-only modal. */
  footer?: ReactNode;
  onClose: () => void;
  /** Max panel width; defaults to a comfortable detail size. */
  width?: string;
}

export function EntityDetailModal({ title, subtitle, children, footer, onClose, width }: Props) {
  const { colors } = useTheme();

  // Close on Escape — matches the rest of the app's modal/panel convention.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 110,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: width ?? 'min(560px, 92vw)', maxHeight: '86vh',
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
          display: 'flex', alignItems: 'flex-start', gap: 10,
          padding: '16px 18px', borderBottom: `1px solid ${colors.border}`,
        }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{
              fontFamily: font.display, fontSize: 16, fontWeight: 600, color: colors.text,
              wordBreak: 'break-word',
            }}>
              {title}
            </div>
            {subtitle && (
              <div style={{ marginTop: 4, fontSize: 11, color: colors.textDim, fontFamily: font.mono }}>
                {subtitle}
              </div>
            )}
          </div>
          <button
            onClick={onClose}
            title="Close"
            style={{
              background: 'none', border: 'none', color: colors.textMuted,
              cursor: 'pointer', padding: 4, display: 'flex', flexShrink: 0,
            }}
          >
            <FiX size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ overflow: 'auto', padding: '16px 18px', flex: 1 }}>
          {children}
        </div>

        {/* Footer actions */}
        {footer && (
          <div style={{
            padding: '12px 18px', borderTop: `1px solid ${colors.border}`,
            display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap',
          }}>
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}
