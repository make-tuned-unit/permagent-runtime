/**
 * StateBlock — the Projects tab's empty-vs-error affordance.
 *
 * Lived inside `ProjectsView` until `ProjectOverview`'s Tasks panel needed the
 * same shape for the same two endpoints. It says the two states apart out
 * loud: an `empty` tone names the action that would fill the surface, an
 * `error` tone names what failed and hands back a retry. A fetch failure that
 * renders as an empty list is the defect this exists to make unwriteable.
 */

import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function StateBlock({ tone, title, detail, onRetry, compact }: {
  tone: 'empty' | 'error';
  title: string;
  detail?: string;
  onRetry?: () => void;
  compact?: boolean;
}) {
  const { colors } = useTheme();
  const isError = tone === 'error';
  const accent = isError ? colors.danger : colors.textMuted;
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6,
      textAlign: 'center', padding: compact ? '20px 16px' : '40px 24px',
      color: colors.textMuted, fontFamily: font.body,
    }}>
      <div style={{ fontSize: 13, fontWeight: 600, color: accent }}>{title}</div>
      {detail && (
        <div style={{ fontSize: 11, color: colors.textDim, maxWidth: 320, lineHeight: 1.5 }}>
          {detail}
        </div>
      )}
      {onRetry && (
        <button
          onClick={onRetry}
          style={{
            marginTop: 6, padding: '5px 14px', borderRadius: 6, cursor: 'pointer',
            background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
            color: colors.cyan, fontSize: 11, fontWeight: 600, fontFamily: font.body,
          }}
        >
          Try again
        </button>
      )}
    </div>
  );
}
