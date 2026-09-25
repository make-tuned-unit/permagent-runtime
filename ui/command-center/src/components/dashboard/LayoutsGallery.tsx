/**
 * Customize → Layouts: the 5 templates browsable without chat (LANE brief
 * correction to the research doc — "Templates are also directly browsable
 * without chat"), each with a live mini-preview rendered from its real
 * layout, plus "Custom (yours)" — the user's current hand-arranged board.
 * "Try" opens the same current-vs-proposed overlay the agent's
 * `preview_dashboard_layout` tool triggers (R7: one concept, one place).
 */
import { useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import { Button } from '../common/Button';
import { MiniLayoutPreview } from './MiniLayoutPreview';
import { LayoutPreviewOverlay } from './LayoutPreviewOverlay';
import { useDashboardTemplates, realizedLayout, comingSoonCards, type DashboardTemplate } from './templates';
import type { DashboardLayoutData } from './useLayout';

interface Props {
  currentLayout: DashboardLayoutData;
  onApply: (layout: DashboardLayoutData) => Promise<void> | void;
  onClose: () => void;
}

export function LayoutsGallery({ currentLayout, onApply, onClose }: Props) {
  const { colors } = useTheme();
  const templates = useDashboardTemplates();
  const [previewing, setPreviewing] = useState<DashboardTemplate | null>(null);

  if (previewing) {
    return (
      <LayoutPreviewOverlay
        current={currentLayout}
        proposed={realizedLayout(previewing)}
        reason={`${previewing.name} — ${previewing.question}`}
        comingSoon={comingSoonCards(previewing)}
        onApply={async () => { await onApply(realizedLayout(previewing)); onClose(); }}
        onKeepCurrent={() => setPreviewing(null)}
        onTryAnother={() => setPreviewing(null)}
      />
    );
  }

  return (
    <DetailModal title="Layouts" onClose={onClose} width="min(880px, 96vw)">
      <div style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, marginBottom: space.lg, lineHeight: 1.5 }}>
        Five arrangements of your Home dashboard, each built for a different moment. Try one to
        compare it against what you have now — nothing changes until you apply it.
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: space.lg }}>
        {templates.map(t => (
          <TemplateTile key={t.id} template={t} onTry={() => setPreviewing(t)} />
        ))}
        <div style={{
          borderRadius: radius.lg, border: `1px solid ${colors.border}`, padding: space.lg,
          display: 'flex', flexDirection: 'column', gap: space.md,
        }}>
          <div>
            <div style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
              Custom (yours)
            </div>
            <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
              Your current hand-arranged layout
            </div>
          </div>
          <MiniLayoutPreview layout={currentLayout} />
        </div>
      </div>
    </DetailModal>
  );
}

function TemplateTile({ template, onTry }: { template: DashboardTemplate; onTry: () => void }) {
  const { colors } = useTheme();
  const soon = comingSoonCards(template);
  return (
    <div style={{
      borderRadius: radius.lg, border: `1px solid ${colors.border}`, padding: space.lg,
      display: 'flex', flexDirection: 'column', gap: space.md,
    }}>
      <div>
        <div style={{ fontFamily: font.display, fontSize: textSize.small, fontWeight: 600, color: colors.text }}>
          {template.name}
        </div>
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginTop: space.xxs }}>
          {template.question}
        </div>
      </div>
      <MiniLayoutPreview layout={realizedLayout(template)} />
      {soon.length > 0 && (
        <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim }}>
          Coming soon: {soon.join(', ')}
        </div>
      )}
      <Button
        colors={colors}
        type="button"
        onClick={onTry}
        style={{
          '--pa-btn-bg': colors.cyanSoft,
          '--pa-btn-fg': colors.cyan,
          '--pa-btn-border': colors.cyan,
          '--pa-btn-bg-hover': colors.cyanSoft,
          '--pa-btn-fg-hover': colors.cyan,
          '--pa-btn-pad': '6px 14px',
          '--pa-btn-radius': `${radius.md}px`,
          '--pa-btn-weight': 600,
          alignSelf: 'flex-start',
        } as CSSProperties}
      >
        Try
      </Button>
    </div>
  );
}
