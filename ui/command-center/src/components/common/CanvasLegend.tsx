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
 *
 * Material: floating control over canvas content → shared glass tokens (D1).
 * One glass plane per open/closed state; children use fillHover (D2/D10).
 */

import { useCallback, useId, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react';
import { concentric, font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from './Glass';
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
  // A true grid, not one flex row per entry: flex sized each row's term
  // column off that row's own content, so "Drag" and "Right-drag or arrow
  // keys" landed their descriptions at two different x positions and a
  // wrapped second line had nothing to hang under. Grid tracks are shared
  // across every row, so the marker/term/description columns line up top to
  // bottom and a wrapped description stays under its own first line.
  const hasMarkers = rows.some(row => row.marker != null);
  return (
    <dl
      style={{
        margin: 0,
        display: 'grid',
        gridTemplateColumns: hasMarkers ? 'auto auto 1fr' : 'auto 1fr',
        columnGap: space.sm,
        rowGap: space.xs,
        alignItems: 'baseline',
      }}
    >
      {rows.map(row => (
        // `display: contents` lets this wrapper carry the row's `key` without
        // introducing a box of its own — its children join the parent grid
        // directly, as if they were its siblings.
        <div key={row.term} style={{ display: 'contents' }}>
          {hasMarkers && (
            <span aria-hidden="true" style={{ lineHeight: 1.4, alignSelf: 'center' }}>
              {row.marker}
            </span>
          )}
          <dt style={{ margin: 0, color: palette.text, fontWeight: 600 }}>{row.term}</dt>
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
  const glass = useGlass('glass');
  const [open, setOpen] = useState(() => readLegendOpen(canvasId));
  const panelId = useId();

  const set = useCallback((next: boolean) => {
    setOpen(next);
    rememberLegendOpen(canvasId, next);
  }, [canvasId]);

  // Ink/accent may still come from a canvas-local palette (World scene colours);
  // the glass plane itself always comes from the shared tokens.
  const pal: LegendPalette = {
    bg: palette?.bg ?? (glass.background as string) ?? colors.surface,
    border: palette?.border ?? colors.border,
    text: palette?.text ?? colors.text,
    dim: palette?.dim ?? colors.textMuted,
    accent: palette?.accent ?? colors.cyan,
  };

  const panelPadX = space.xl;
  const panelPadY = space.lg;
  const panelRadius = radius.glass;
  const chipRadius = concentric(panelRadius, panelPadY);

  const anchor: CSSProperties = {
    position: 'absolute',
    bottom: space.xxl,
    left: space.xxl,
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
    '--pa-btn-bg-hover': colors.fillHover,
    '--pa-btn-bg-active': colors.fillActive,
    '--pa-btn-border-hover': 'transparent',
    '--pa-btn-pad': `${2}px ${space.sm}px`,
    '--pa-btn-radius': `${chipRadius > 0 ? chipRadius : radius.xs}px`,
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
            ...glass,
            // Button paints via --pa-btn-bg; feed it the glass fill so the
            // filter and translucent background travel together.
            '--pa-btn-bg': (glass.background as string) ?? colors.surface,
            '--pa-btn-border': pal.border,
            '--pa-btn-pad': `${space.xs}px ${space.lg}px`,
            '--pa-btn-radius': `${panelRadius}px`,
            boxShadow: glass.boxShadow,
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
        gap: space.md,
        padding: `${panelPadY}px ${panelPadX}px`,
        ...glass,
        // Prefer theme glass fill over a caller palette bg that may be opaque
        // (would kill the blur). Callers that need scene ink pass text/dim/accent.
        background: (glass.background as string) ?? pal.bg,
        border: `1px solid ${pal.border}`,
        borderRadius: panelRadius,
      }}
    >
      <header style={{ display: 'flex', alignItems: 'center', gap: space.lg }}>
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
    <div style={{ display: 'grid', gap: space.xxs }}>
      <span style={{
        fontFamily: font.mono,
        fontSize: textSize.micro,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: palette.dim,
        opacity: 0.75,
      }}>{label}</span>
      {children}
    </div>
  );
}
