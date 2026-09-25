/**
 * A small schematic rendering of a `DashboardLayoutData` — the "live mini-
 * preview rendered from the real layout at small scale" the LANE brief asks
 * the Layouts gallery and the preview overlay to share (rather than each
 * inventing its own thumbnail). Draws the actual 12-column grid at the
 * layout's actual proportions (rows scaled to fit `height`), one block per
 * card, LABELLED with its card name (coordinator, Phase-1 presentation pass:
 * "a layout must be legible at a glance", not just a grid of anonymous
 * rectangles a hover reveals one at a time).
 *
 * The label is drawn at the type ramp's smallest real role (`textSize.micro`,
 * 11px — never a new off-ramp size, which is frozen) and visually shrunk with
 * a CSS `transform: scale()`, the same technique a favicon or a map legend
 * uses to fit a real word into a small box: the source size stays a token,
 * only its rendered size changes. Each block also keeps a `Tooltip` (never a
 * raw `title=`) for the full name when a block is too narrow to show it.
 */
import { textSize, radius, space, font } from '../../styles/tokens';
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
      {layout.cards.map(c => {
        const name = registry[c.type]?.name ?? c.type;
        return (
          <Tooltip key={c.id} content={name}>
            <div
              style={{
                gridColumn: `${c.position.x + 1} / span ${c.size.w}`,
                gridRow: `${c.position.y + 1} / span ${c.size.h}`,
                background: colors.cyanSoft,
                border: `1px solid ${colors.borderHi}`,
                borderRadius: radius.xs,
                minWidth: 0,
                minHeight: 0,
                overflow: 'hidden',
                display: 'flex', alignItems: 'flex-start', justifyContent: 'flex-start',
                padding: space.xxs,
              }}
            >
              <span style={{
                display: 'inline-block',
                fontFamily: font.body, fontSize: textSize.micro, fontWeight: 600,
                color: colors.cyan, lineHeight: 1,
                whiteSpace: 'nowrap',
                transform: 'scale(0.55)', transformOrigin: 'top left',
              }}>
                {name}
              </span>
            </div>
          </Tooltip>
        );
      })}
    </div>
  );
}
