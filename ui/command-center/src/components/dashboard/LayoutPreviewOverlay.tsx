/**
 * Current-vs-proposed layout comparison (P9, HOME_TEMPLATES_RESEARCH.md —
 * "the agent must propose, never silently apply, a layout change"). Opened
 * either from the Layouts gallery ("Try") or from the agent's
 * `preview_dashboard_layout` tool call (`dashboard_layout_preview` event).
 * Apply persists (snapshotting the layout it replaces, server-side); Keep
 * current discards the preview with no state change; Try another goes back
 * to the gallery.
 */
import { useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import { Button } from '../common/Button';
import { MiniLayoutPreview } from './MiniLayoutPreview';
import type { DashboardLayoutData } from './useLayout';

interface Props {
  current: DashboardLayoutData;
  proposed: DashboardLayoutData;
  /** One-sentence justification (R5 / HAX G11 — "explain why"). */
  reason: string;
  /** Real cards this template names that Phase 2 hasn't built yet. */
  comingSoon?: string[];
  onApply: () => Promise<void> | void;
  onKeepCurrent: () => void;
  onTryAnother?: () => void;
}

export function LayoutPreviewOverlay({ current, proposed, reason, comingSoon = [], onApply, onKeepCurrent, onTryAnother }: Props) {
  const { colors } = useTheme();
  const [applying, setApplying] = useState(false);

  const apply = async () => {
    setApplying(true);
    try { await onApply(); } finally { setApplying(false); }
  };

  return (
    <DetailModal
      title="Preview this layout"
      onClose={onKeepCurrent}
      width="min(760px, 94vw)"
      footer={
        <div style={{ display: 'flex', gap: space.md, justifyContent: 'flex-end', width: '100%' }}>
          <Button
            colors={colors}
            type="button"
            onClick={onKeepCurrent}
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-border': colors.border,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-pad': '6px 14px',
              '--pa-btn-radius': `${radius.md}px`,
            } as CSSProperties}
          >
            Keep current
          </Button>
          {onTryAnother && (
            <Button
              colors={colors}
              type="button"
              onClick={onTryAnother}
              style={{
                '--pa-btn-bg': 'transparent',
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-border': colors.border,
                '--pa-btn-bg-hover': colors.surfaceHi,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-pad': '6px 14px',
                '--pa-btn-radius': `${radius.md}px`,
              } as CSSProperties}
            >
              Try another
            </Button>
          )}
          <Button
            colors={colors}
            type="button"
            disabled={applying}
            onClick={apply}
            style={{
              '--pa-btn-bg': colors.cyanSoft,
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-border': colors.cyan,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-fg-hover': colors.cyan,
              '--pa-btn-pad': '6px 16px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 600,
            } as CSSProperties}
          >
            {applying ? 'Applying…' : 'Apply'}
          </Button>
        </div>
      }
    >
      <div style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.text, marginBottom: space.lg, lineHeight: 1.5 }}>
        {reason}
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: space.lg }}>
        <div>
          <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase', color: colors.textDim, marginBottom: space.sm }}>
            Current
          </div>
          <MiniLayoutPreview layout={current} height={220} />
        </div>
        <div>
          <div style={{ fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase', color: colors.cyan, marginBottom: space.sm }}>
            Proposed
          </div>
          <MiniLayoutPreview layout={proposed} height={220} />
        </div>
      </div>
      {comingSoon.length > 0 && (
        <div style={{ marginTop: space.lg, fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          Not built yet, so left out of this preview: {comingSoon.join(', ')}.
        </div>
      )}
    </DetailModal>
  );
}
