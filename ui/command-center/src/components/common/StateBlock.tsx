/**
 * StateBlock — the app's one empty/error/loading affordance for a data
 * surface.
 *
 * Lived inside `ProjectsView` until `ProjectOverview`'s Tasks panel needed the
 * same shape for the same two endpoints. It says the states apart out loud:
 * an `empty` tone names the action that would fill the surface, an `error`
 * tone names what failed and hands back a retry, and `loading` is a plain
 * "not ready yet" line. A fetch failure that renders as an empty list is the
 * defect this exists to make unwriteable.
 *
 * `loading` was added when Automate's per-section loading text (a bare
 * `<div>`, one of three near-identical hand-rolled copies of this shape)
 * converged onto this component instead of forking a second one (R6/R7,
 * UX_STANDARD.md — "one concept, one place").
 */

import { type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export function StateBlock({ tone, title, detail, onRetry, compact }: {
  tone: 'empty' | 'error' | 'loading';
  title: string;
  detail?: string;
  onRetry?: () => void;
  compact?: boolean;
}) {
  const { colors } = useTheme();
  const isError = tone === 'error';
  const accent = isError ? colors.danger : tone === 'loading' ? colors.textDim : colors.textMuted;
  return (
    <div style={{
      display: 'flex', flexDirection: 'column', alignItems: 'center', gap: space.sm,
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
            marginTop: space.sm,
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
