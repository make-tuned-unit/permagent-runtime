/**
 * `CanvasLegend` — the key the app's three canvases share.
 *
 * Every DOM surface in the app carries its own explanation: a button says what
 * it does, a chip says whether it is live, a column header names its column. A
 * 3D canvas has none of that. Nothing on the World, the Brain's graph or the
 * People graph said that dragging turns the scene, that a dimmed face means
 * nobody has spoken to that person in a month, or which of the glowing things
 * is a real signal rather than set dressing. Each canvas had answered that
 * differently, which is to say none of them had answered it.
 *
 * So: one key, three canvases, two halves.
 *
 *   Gestures — only the ones that actually exist. A key that lists a control
 *   the canvas does not have is worse than no key, because it is now the
 *   user's fault when it does not work. Every row here was read off the
 *   handler that implements it.
 *
 *   Vocabulary — what the picture means, and specifically which parts of it
 *   are claims. This is the Chip doctrine (`Chip.tsx`) applied to a scene:
 *   a chip must not pulse unless something is happening, and by the same rule
 *   a canvas must say which of its motion is bound to a feed and which is
 *   decoration. The World's rain really is the Brain ingesting a memory; the
 *   Brain graph's travelling lights really are constant. Both facts belong on
 *   screen, not in a source comment.
 *
 * The behaviour is "teach once, then be quiet" (`legendMemory.ts`): open on a
 * first visit, dismissed for good on request, per canvas. It is a caption, not
 * a dialog — it never traps focus, never covers the scene's controls, and can
 * be dismissed from the keyboard by anyone who has tabbed into it.
 */

import { useCallback, useId, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from './Button';
import { readLegendOpen, rememberLegendOpen } from './legendMemory';

export interface LegendRow {
  /** An optional swatch or glyph — for a vocabulary the scene draws rather than names. */
  marker?: ReactNode;
  term: string;
  meaning: string;
}

/** Canvases that paint from their own palette (the World) hand it over here. */
export interface LegendPalette {
  bg: string;
  border: string;
  text: string;
  dim: string;
  accent: string;
}

export interface CanvasLegendProps {
  /** Storage identity. Stable per canvas — it is what "already taught" means. */
  canvasId: string;
  title?: string;
  gestures: LegendRow[];
  vocabulary: LegendRow[];
  gesturesLabel?: string;
  vocabularyLabel?: string;
  palette?: Partial<LegendPalette>;
  /** Position/size overrides. Defaults to the bottom-left of the canvas. */
  style?: CSSProperties;
}

function Rows({ rows, palette }: { rows: LegendRow[]; palette: LegendPalette }) {
  return (
    <dl style={{ margin: 0, display: 'grid', gap: 4 }}>
      {rows.map(row => (
        <div key={row.term} style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
          {row.marker != null && (
            <span aria-hidden="true" style={{ flexShrink: 0, lineHeight: 1.4 }}>{row.marker}</span>
          )}
          <dt style={{ color: palette.text, fontWeight: 600, flexShrink: 0 }}>{row.term}</dt>
          <dd style={{ margin: 0, color: palette.dim, lineHeight: 1.45 }}>{row.meaning}</dd>
        </div>
      ))}
    </dl>
  );
}

export function CanvasLegend({
  canvasId,
  title = 'Key',
  gestures,
  vocabulary,
  gesturesLabel = 'Getting around',
  vocabularyLabel = 'What you’re looking at',
  palette,
  style,
}: CanvasLegendProps) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(() => readLegendOpen(canvasId));
  const panelId = useId();

  const set = useCallback((next: boolean) => {
    setOpen(next);
    rememberLegendOpen(canvasId, next);
  }, [canvasId]);

  const pal: LegendPalette = {
    bg: palette?.bg ?? colors.surface,
    border: palette?.border ?? colors.border,
    text: palette?.text ?? colors.text,
    dim: palette?.dim ?? colors.textMuted,
    accent: palette?.accent ?? colors.cyan,
  };

  const anchor: CSSProperties = {
    position: 'absolute',
    bottom: 16,
    left: 16,
    zIndex: 12,
    fontFamily: font.body,
    fontSize: textSize.micro,
    ...style,
  };

  const controlVars = {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': pal.dim,
    '--pa-btn-fg-hover': pal.accent,
    '--pa-btn-border': 'transparent',
    '--pa-btn-bg-hover': `${pal.accent}1f`,
    '--pa-btn-border-hover': 'transparent',
    '--pa-btn-pad': '2px 6px',
    '--pa-btn-radius': `${radius.xs}px`,
    fontFamily: font.mono,
    fontSize: textSize.micro,
    letterSpacing: '0.08em',
  } as CSSProperties;

  if (!open) {
    return (
      <div style={anchor}>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          flashSuccess={false}
          data-testid="canvas-legend-open"
          aria-expanded={false}
          aria-controls={panelId}
          onClick={() => set(true)}
          style={{
            ...controlVars,
            '--pa-btn-bg': pal.bg,
            '--pa-btn-border': pal.border,
            '--pa-btn-pad': '4px 10px',
            backdropFilter: 'blur(8px)',
          } as CSSProperties}
        >
          {title}
        </Button>
      </div>
    );
  }

  // Escape closes it for anyone who reached it with the keyboard. Deliberately
  // NOT a window listener: Escape already means "leave walking mode" in the
  // World and "come back from the Agora", and a caption must not compete for a
  // key that navigates.
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      set(false);
    }
  };

  return (
    <section
      id={panelId}
      data-testid="canvas-legend"
      aria-label={`${title} — what this view shows and how to move around it`}
      onKeyDown={onKeyDown}
      style={{
        ...anchor,
        maxWidth: 330,
        display: 'grid',
        gap: 8,
        padding: '10px 12px',
        background: pal.bg,
        border: `1px solid ${pal.border}`,
        borderRadius: radius.md,
        backdropFilter: 'blur(10px)',
      }}
    >
      <header style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{
          fontFamily: font.mono,
          fontSize: textSize.micro,
          letterSpacing: '0.14em',
          textTransform: 'uppercase',
          color: pal.dim,
        }}>{title}</span>
        <span style={{ flex: 1 }} />
        <Button
          colors={colors}
          variant="bare"
          type="button"
          flashSuccess={false}
          data-testid="canvas-legend-dismiss"
          aria-label={`Dismiss the ${title.toLowerCase()}`}
          onClick={() => set(false)}
          style={controlVars}
        >
          ✕
        </Button>
      </header>

      <Section label={gesturesLabel} palette={pal}><Rows rows={gestures} palette={pal} /></Section>
      <Section label={vocabularyLabel} palette={pal}><Rows rows={vocabulary} palette={pal} /></Section>
    </section>
  );
}

function Section({ label, palette, children }: { label: string; palette: LegendPalette; children: ReactNode }) {
  return (
    <div style={{ display: 'grid', gap: 3 }}>
      <span style={{
        fontFamily: font.mono,
        fontSize: 9,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: palette.dim,
        opacity: 0.75,
      }}>{label}</span>
      {children}
    </div>
  );
}
