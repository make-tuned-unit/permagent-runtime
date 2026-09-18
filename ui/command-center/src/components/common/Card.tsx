/**
 * Card primitives — graduated from `components/settings/atoms.tsx` (#1177,
 * #1185): `history/SpendPanel` and `settings/SettingsView` both pull these
 * for the same data-dense card layout, and `settings/atoms` is not a home
 * either of those directories should have to reach into for a primitive.
 * `components/common` is the one place a cross-directory atom lives.
 *
 * Migrated from the Governance surface when its panels were folded into
 * Settings — shared by the Spend / Sovereignty / Models panes so their
 * data-dense views read as one surface.
 */

import { font, radius, space, textSize, type, concentric } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

/** Outer padding of a `Card`. Named because `StatRow` derives its own corner
 *  from it: `r_inner = r_outer - padding` (WWDC25/356). */
const CARD_PAD = space.xxl;

export function Card({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      borderRadius: radius.lg,
      background: colors.surface,
      border: `1px solid ${colors.border}`,
      padding: CARD_PAD,
    }}>
      {children}
    </div>
  );
}

export function SectionLabel({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return <div style={{ ...type.label, fontFamily: font.body, color: colors.textDim }}>{children}</div>;
}

/** A labeled row: primary text + optional sub-line on the left, a value node on
 *  the right. Its corner is concentric with the `Card` it sits in. */
export function StatRow({ left, sub, right }: { left: React.ReactNode; sub?: React.ReactNode; right: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: space.xl,
      padding: `${space.lg}px ${space.xl}px`,
      borderRadius: concentric(radius.lg, CARD_PAD),
      background: colors.fillSubtle,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: textSize.small, fontWeight: 600, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {left}
        </div>
        {sub != null && (
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, fontFamily: font.mono, marginTop: space.xxs, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {sub}
          </div>
        )}
      </div>
      <div style={{ flexShrink: 0 }}>{right}</div>
    </div>
  );
}
