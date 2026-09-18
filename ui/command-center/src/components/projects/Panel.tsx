// The Project Overview panel shell — uppercase title bar with an optional
// action slot, over a bordered card body. Shared by every Overview panel
// (Summary / Key Facts / Links / Tasks / People).

import type { ReactNode } from 'react';
import { useTheme } from '../../styles/useTheme';
import { radius, space, textSize } from '../../styles/tokens';

export function Panel({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  const { colors } = useTheme();
  // Subtle surface veil — a white wash vanishes on silver, so flip to a faint
  // graphite tint there — which is exactly what `fillSubtle` now is.
  const veil = colors.fillSubtle;
  // `fillSubtle` IS this idiom, tokenised (#1162). The hand-written pair it
  // replaces predates the token and was a shade off on both themes; the token
  // carries the THEME's own ink, so one name reads as a lift on the void and
  // as a shade on the pearl without a conditional here.
  return (
    <section style={{
      background: veil, border: `1px solid ${colors.border}`,
      borderRadius: radius.lg, padding: `14px ${space.xxl}px`,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', marginBottom: space.lg }}>
        <span style={{
          fontSize: textSize.micro, fontWeight: 600, color: colors.textMuted,
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
