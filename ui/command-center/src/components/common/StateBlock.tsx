/**
 * StateBlock — the Projects tab's empty-vs-error affordance.
 *
 * Lived inside `ProjectsView` until `ProjectOverview`'s Tasks panel needed the
 * same shape for the same two endpoints. It says the two states apart out
 * loud: an `empty` tone names the action that would fill the surface, an
 * `error` tone names what failed and hands back a retry. A fetch failure that
 * renders as an empty list is the defect this exists to make unwriteable.
 */

import { type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

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
      <div style={{ fontSize: textSize.small, fontWeight: 600, color: accent }}>{title}</div>
      {detail && (
        <div style={{ fontSize: textSize.micro, color: colors.textMuted, maxWidth: 320, lineHeight: 1.5 }}>
          {detail}
        </div>
      )}
      {onRetry && (
        <Button
          colors={colors}
          variant="ghostOn"
          onClick={onRetry}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-border': colors.borderHi,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-pad': '5px 14px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': 600,
            marginTop: 6,
            fontSize: textSize.micro,
            fontFamily: font.body,
          } as CSSProperties}
        >
          Try again
        </Button>
      )}
    </div>
  );
}
