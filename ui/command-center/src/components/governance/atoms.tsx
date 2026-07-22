/**
 * Small presentational primitives shared across the Governance panels, styled
 * from the design tokens so every panel reads as one surface.
 */

import type { ReactNode } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function Card({ children }: { children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      padding: '18px 20px',
    }}>
      {children}
    </div>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      fontFamily: font.body, fontSize: 11, fontWeight: 600,
      letterSpacing: '0.10em', textTransform: 'uppercase', color: colors.textDim,
    }}>
      {children}
    </div>
  );
}

/** A labeled row: primary text + optional sub-line on the left, a value node on
 *  the right. */
export function StatRow({ left, sub, right }: { left: ReactNode; sub?: ReactNode; right: ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 12,
      padding: '10px 12px', borderRadius: radius.md,
      background: colors.bgDeeper, border: `1px solid ${colors.border}`,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {left}
        </div>
        {sub != null && (
          <div style={{ fontSize: 11, color: colors.textMuted, fontFamily: font.mono, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {sub}
          </div>
        )}
      </div>
      <div style={{ flexShrink: 0 }}>{right}</div>
    </div>
  );
}
