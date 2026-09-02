import { type CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

/**
 * Non-blocking chip for a failed workspace persistence (layout resize or
 * active-workspace switch). The store keeps the optimistic local value, so
 * without this the UI would silently lie that the arrangement was saved —
 * the Dashboard SaveIndicator "Save failed" pattern, plus the Retry path the
 * dashboard's auto-dismissing label doesn't need (a lost layout save has no
 * later poll to heal it). Renders nothing while saves are healthy.
 */
export function WorkspaceSaveErrorChip() {
  const { colors } = useTheme();
  const failure = useCommandCenter(s => s.workspaceSaveFailure);
  const retry = useCommandCenter(s => s.retryWorkspaceSave);
  const dismiss = useCommandCenter(s => s.dismissWorkspaceSaveFailure);

  if (!failure) return null;

  const label = failure.kind === 'layout'
    ? "Couldn't save workspace layout"
    : "Couldn't save active workspace";

  return (
    <div
      style={{
        position: 'fixed', bottom: 18, left: '50%', transform: 'translateX(-50%)',
        zIndex: 95, display: 'flex', alignItems: 'center', gap: 10,
        padding: '6px 12px', borderRadius: radius.md,
        background: colors.surface, border: `1px solid ${colors.border}`,
        boxShadow: colors.cardShadow, fontFamily: font.body, fontSize: textSize.micro,
      }}
      title={failure.message}
    >
      <span style={{ color: colors.danger, fontWeight: 500 }}>{label}</span>
      {/* The retry promise is handed to the button so the round trip is
          visible. No tick: `retryWorkspaceSave` swallows its own failure and
          resolves either way, so a tick could confirm a save that did not
          happen — and a save that DID happen clears the failure, which
          unmounts this chip. */}
      <Button
        colors={colors}
        variant="bare"
        onClick={() => retry()}
        flashSuccess={false}
        style={{
          '--pa-btn-fg': colors.cyan,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
          '--pa-btn-weight': 600,
          fontFamily: font.body,
          fontSize: textSize.micro,
        } as CSSProperties}
      >
        Retry
      </Button>
      <Button
        colors={colors}
        variant="bare"
        onClick={dismiss}
        title="Dismiss"
        style={{
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '0',
          fontFamily: font.body,
          fontSize: textSize.micro,
        } as CSSProperties}
      >
        Dismiss
      </Button>
    </div>
  );
}
