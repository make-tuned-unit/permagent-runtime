/**
 * A small schematic rendering of a `DashboardLayoutData` — the "live mini-
 * preview rendered from the real layout at small scale" the LANE brief asks
 * the Layouts gallery and the preview overlay to share (rather than each
 * inventing its own thumbnail). Draws the actual 12-column grid at the
 * layout's actual proportions (rows scaled to fit `height`), one block per
 * card. Not the real card components — those need live data and full width
 * to mean anything at this scale — but the real geometry, so two templates
 * that arrange cards differently are visibly different at a glance (R6:
 * recognizable, not a generic rectangle). Full names live in the outer
 * `aria-label` and each block's `Tooltip` — never a raw `title=` — since a
 * block this small has no room to set a new, off-ramp font size just to fit
 * a label (the type ramp's per-directory budget is frozen).
 */
import { radius, space } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { GRID_COLS, type DashboardLayoutData } from './useLayout';
import { useCardRegistry } from './cards/useCardRegistry';
import { Tooltip } from '../common/Tooltip';

export function MiniLayoutPreview({ layout, height = 120 }: { layout: DashboardLayoutData; height?: number }) {
  const { colors } = useTheme();
  const { registry } = useCardRegistry();
  const rows = layout.cards.reduce((m, c) => Math.max(m, c.position.y + c.size.h), 1);
  return (
    <div
      role="img"
      aria-label={`Layout preview: ${layout.cards.map(c => registry[c.type]?.name ?? c.type).join(', ')}`}
      style={{
        display: 'grid',
        gridTemplateColumns: `repeat(${GRID_COLS}, 1fr)`,
        gridTemplateRows: `repeat(${rows}, 1fr)`,
        gap: space.xxs,
        width: '100%',
        height,
        padding: space.sm,
        borderRadius: radius.md,
        background: colors.bg,
        border: `1px solid ${colors.border}`,
        boxSizing: 'border-box',
      }}
    >
      {layout.cards.map(c => (
        <Tooltip key={c.id} content={registry[c.type]?.name ?? c.type}>
          <div
            style={{
              gridColumn: `${c.position.x + 1} / span ${c.size.w}`,
              gridRow: `${c.position.y + 1} / span ${c.size.h}`,
              background: colors.cyanSoft,
              border: `1px solid ${colors.borderHi}`,
              borderRadius: radius.xs,
              minWidth: 0,
              minHeight: 0,
            }}
          />
        </Tooltip>
      ))}
    </div>
  );
}
