// The Project Overview panel shell — uppercase title bar with an optional
// action slot, over a bordered card body. Shared by every Overview panel
// (Summary / Key Facts / Links / Tasks / People).

import type { ReactNode } from 'react';
import { useTheme } from '../../styles/useTheme';

export function Panel({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  const { colors, theme } = useTheme();
  // Subtle surface veil — a white wash vanishes on silver, so flip to a faint
  // graphite tint there (same approach as BrainList's theme-conditional rows).
  const veil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  return (
    <section style={{
      background: veil, border: `1px solid ${colors.border}`,
      borderRadius: 10, padding: '14px 16px',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', marginBottom: 10 }}>
        <span style={{
          fontSize: 11, fontWeight: 600, color: colors.textMuted,
          textTransform: 'uppercase', letterSpacing: '0.06em', flex: 1,
        }}>
          {title}
        </span>
        {action}
      </div>
      {children}
    </section>
  );
}
