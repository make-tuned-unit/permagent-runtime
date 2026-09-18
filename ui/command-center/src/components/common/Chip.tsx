/**
 * `Chip` — the app's small labels, and the place its liveness claims are kept
 * honest.
 *
 * Chips are the most duplicated atom in the codebase (category badges, tags,
 * status pills, filter toggles, counts), but the reason to build the primitive
 * is not the duplication. It is that a chip is where the app claims something
 * is happening. In the World, a fixed capability label — "LOCAL", a fact about
 * where the Reader runs, true whether or not anything is running — rendered
 * pixel-identically to "SWEEPING", which is a claim that a sweep is in flight
 * right now. Nothing on screen separated the two, so neither could be trusted.
 *
 * Hence `kind`, which is required and has no default:
 *
 *   state  — bound to a live feed. May carry an `asOf` and may pulse.
 *   static — a fixed label. Never animates, never carries a timestamp, and is
 *            drawn as an outline rather than a fill so it reads as a caption
 *            and not as a signal.
 *   filter — a toggle. Renders as a button and reports `aria-pressed`.
 *   count  — a figure. Tabular, so it doesn't reflow as it changes.
 *   link   — goes somewhere. A button, but never `aria-pressed`: a chip that
 *            navigates has no on/off state to report, and saying it does is
 *            the same class of lie as a static label that pulses.
 *
 * The props are a discriminated union, so `pulse` on a static chip is a type
 * error rather than a code review note: the lie is unwriteable, not merely
 * discouraged.
 *
 * Adoption is deliberately incremental — this lands with the World HUD pills
 * and the Brain's dead-entity chip, the two places the confusion was worst.
 *
 * ## Two things a chip in a long list needs
 *
 * `quiet` is the density rule made into a prop. The default treatment is
 * calibrated for a chip that appears once or twice on a surface; the same
 * treatment down a column of fifteen rows reads as fifteen alerts, and the
 * one row that genuinely differs is lost inside them. A quiet chip drops its
 * fill, thins its hairline and dims its tone, so a repeated label reads as a
 * rhythm and the emphasis budget is spent on the rare state instead. It comes
 * back up to full on hover, so nothing is lost — only postponed.
 *
 * The interactive kinds also take the app's shared `.pa-btn` interaction
 * rules. An inline style cannot express `:hover` or `:active` at all, so a
 * clickable chip was pressable with no acknowledgement — the same defect the
 * Button primitive exists to fix, and a chip action is a button that happens
 * to sit inline (U3 §1.3). Their colours are therefore handed to the shared
 * rules as custom properties rather than set inline, which is what lets hover
 * actually land.
 */

import type { CSSProperties, ReactNode } from 'react';
import { font, radius, space, tabularNums } from '../../styles/tokens';
import { useTheme, type ThemeColors } from '../../styles/useTheme';
import { useFreshness } from '../../hooks/useFreshness';
import { Tooltip } from './Tooltip';

export type ChipKind = 'state' | 'static' | 'filter' | 'count' | 'link';
export type ChipTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'stale';

interface ChipBase {
  children: ReactNode;
  tone?: ChipTone;
  /**
   * An out-of-system colour, for the one family that legitimately has one: the
   * World's agent trims, which are scene identity rather than semantics. Every
   * other caller should reach for `tone`.
   */
  color?: string;
  /**
   * Draw this as a caption rather than a mark: no fill, a thinner hairline,
   * the tone dimmed. For a label that is true of most rows in a list, where
   * the default treatment repeated fifteen times reads as fifteen alerts. The
   * rare row keeps the default and gets the emphasis.
   */
  quiet?: boolean;
  /** Overrides the kind's own explanation. */
  title?: string;
  style?: CSSProperties;
  'data-testid'?: string;
}

export type ChipProps = ChipBase & (
  | {
    kind: 'state';
    /** When this reading was last confirmed true. Shown on hover — a live
     *  claim should be able to say how recently it was live. */
    asOf?: number | string | Date | null;
    /** Only for a state that is genuinely in flight right now. A pulse is a
     *  claim; decoration that pulses forever is the thing this file exists to
     *  prevent. Ignored under reduce-motion. */
    pulse?: boolean;
    pressed?: never; onClick?: never;
  }
  | { kind: 'static'; asOf?: never; pulse?: never; pressed?: never; onClick?: never }
  | { kind: 'filter'; pressed: boolean; onClick: () => void; asOf?: never; pulse?: never }
  | { kind: 'count'; asOf?: never; pulse?: never; pressed?: never; onClick?: never }
  | {
    kind: 'link';
    onClick: () => void;
    /** When the destination is a disclosure on this same page rather than
     *  another surface, the chip says so the way any disclosure control does.
     *  Still never `aria-pressed`: an expanded row is not an on state. */
    expanded?: boolean;
    /** Id of the region `expanded` refers to. */
    controls?: string;
    asOf?: never; pulse?: never; pressed?: never;
  }
);

function toneColor(tone: ChipTone, colors: ThemeColors): string {
  switch (tone) {
    case 'accent': return colors.cyan;
    case 'success': return colors.success;
    case 'warning': return colors.warning;
    case 'danger': return colors.danger;
    case 'stale': return colors.stale;
    default: return colors.textMuted;
  }
}

/** Chips are the one shape the ruling reserves the full pill for. */
const SHELL: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: space.sm,
  padding: `${space.xxs}px ${space.md}px`,
  borderRadius: radius.pill,
  fontFamily: font.body,
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: '0.08em',
  whiteSpace: 'nowrap',
};

export function Chip(props: ChipProps) {
  const { kind, tone = 'neutral', color, quiet, title, style, children } = props;
  const { colors, reduceMotion } = useTheme();
  const accent = color ?? toneColor(tone, colors);

  // A state chip's `asOf` is read through the app's one freshness vocabulary,
  // so "last confirmed" reads the same here as it does anywhere else.
  const asOf = kind === 'state' ? props.asOf : null;
  const freshness = useFreshness(asOf ?? null, { unknownLabel: '' });

  const live = kind === 'state';
  const pulsing = live && props.pulse === true && !reduceMotion;

  const interactive = kind === 'filter' || kind === 'link';

  // The distinction that matters: a live or active chip is FILLED, a fixed
  // label is an outline. Fill reads as signal; outline reads as caption. A
  // quiet chip is an outline whatever its kind — it is a caption by request.
  const filled = !quiet
    && kind !== 'static'
    && !(kind === 'filter' && !props.pressed);
  const background = filled ? withAlpha(accent, 0.14) : 'transparent';
  const borderColor = withAlpha(accent, quiet ? 0.22 : kind === 'static' ? 0.28 : 0.42);
  const foreground = quiet ? withAlpha(accent, 0.78) : accent;

  const shell: CSSProperties = {
    ...SHELL,
    ...(quiet ? { fontWeight: 600 } : null),
    ...(kind === 'count' ? { fontFamily: font.mono, ...tabularNums } : null),
    // An interactive chip hands its colours to `.pa-btn` instead of setting
    // them inline, because an inline value would win over the hover rule and
    // leave the press unacknowledged — the defect, not the styling.
    ...(interactive
      ? {
        '--pa-btn-bg': background,
        '--pa-btn-bg-hover': withAlpha(accent, filled ? 0.22 : 0.12),
        '--pa-btn-border': borderColor,
        '--pa-btn-border-hover': withAlpha(accent, quiet ? 0.42 : 0.6),
        '--pa-btn-fg': foreground,
        // A quiet chip comes back up to full when you reach for it.
        '--pa-btn-fg-hover': accent,
        '--pa-btn-pad': SHELL.padding,
        '--pa-btn-radius': `${radius.pill}px`,
        '--pa-btn-weight': quiet ? 600 : 700,
      } as CSSProperties
      : { color: foreground, background, border: `1px solid ${borderColor}` }),
    ...style,
  };

  const explain = title ?? defaultTitle(kind, freshness.label);

  const body = (
    <>
      {live && (
        // The liveness cue: present only when something is actually watched.
        <span
          data-testid="chip-dot"
          aria-hidden="true"
          className={pulsing ? 'status-pulse' : undefined}
          style={{
            width: 5, height: 5, borderRadius: radius.pill,
            background: accent, flexShrink: 0, display: 'inline-block',
          }}
        />
      )}
      {children}
    </>
  );

  if (props.kind === 'filter' || props.kind === 'link') {
    const button = (
      <button
        type="button"
        // The app's one set of interaction rules: hover, the 80ms pressed
        // give, and the focus ring every control shares.
        className="pa-btn"
        // Only a filter has an on/off state to report. A link has a
        // destination, which `aria-pressed` would misdescribe.
        aria-pressed={props.kind === 'filter' ? props.pressed : undefined}
        aria-expanded={props.kind === 'link' ? props.expanded : undefined}
        aria-controls={props.kind === 'link' ? props.controls : undefined}
        onClick={props.onClick}
        data-testid={props['data-testid']}
        style={shell}
      >
        {body}
      </button>
    );
    return explain ? <Tooltip content={explain}>{button}</Tooltip> : button;
  }

  const span = (
    <span
      data-testid={props['data-testid']}
      style={shell}
      // Static / count chips are not buttons; when they carry an explanation
      // the tip must still be reachable from the keyboard.
      tabIndex={explain ? 0 : undefined}
    >
      {body}
    </span>
  );
  return explain ? <Tooltip content={explain}>{span}</Tooltip> : span;
}

/** What the chip says about itself when the caller hasn't. Only `static` gets
 *  a standing explanation, because it is the kind a reader can misread. */
function defaultTitle(kind: ChipKind, asOfLabel: string): string | undefined {
  if (kind === 'static') return 'A fixed label — not a live status';
  if (kind === 'state' && asOfLabel) return `Last confirmed ${asOfLabel}`;
  return undefined;
}

function withAlpha(color: string, alpha: number): string {
  if (color.startsWith('#') && (color.length === 7 || color.length === 4)) {
    const hex = color.length === 4
      ? color.slice(1).split('').map(c => c + c).join('')
      : color.slice(1);
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }
  // Already an rgba()/named colour — layer the alpha via color-mix rather than
  // guessing at its channels.
  return `color-mix(in srgb, ${color} ${Math.round(alpha * 100)}%, transparent)`;
}
