/**
 * The pane title. Graduated from `components/settings/atoms.tsx` (#1177,
 * #1185): `history/HistoryView` needed the same title treatment as Settings
 * and had to reach across directories to get it — a primitive two screens
 * share belongs in `components/common`, not in one of the two screens.
 *
 * `type.title` rather than a hand-typed 24px: the ramp's `title` is 20/26/600
 * at -0.01em, which is the macOS large-title proportion for a dense window.
 *
 * Left-aligned, and the subtitle sits at a readable measure — Tahoe's
 * typography is *"bolder and left-aligned"*, and centered body copy is now
 * the un-Apple choice (WWDC25/356).
 */

import { font, space, textSize, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function H1({ children, sub }: { children: React.ReactNode; sub?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ marginBottom: space.huge }}>
      <div style={{ ...type.title, fontFamily: font.display, color: colors.text }}>{children}</div>
      {sub && (
        <div style={{
          fontSize: textSize.small, color: colors.textMuted,
          marginTop: space.sm, maxWidth: 620, lineHeight: 1.45,
        }}>{sub}</div>
      )}
    </div>
  );
}
