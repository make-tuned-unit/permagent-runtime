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
 *
 * The props are a discriminated union, so `pulse` on a static chip is a type
 * error rather than a code review note: the lie is unwriteable, not merely
 * discouraged.
 *
 * Adoption is deliberately incremental — this lands with the World HUD pills
 * and the Brain's dead-entity chip, the two places the confusion was worst.
 */

import type { CSSProperties, ReactNode } from 'react';
import { font, radius, tabularNums } from '../../styles/tokens';
import { useTheme, type ThemeColors } from '../../styles/useTheme';
import { useFreshness } from '../../hooks/useFreshness';

export type ChipKind = 'state' | 'static' | 'filter' | 'count';
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
  gap: 6,
  padding: '2px 8px',
  borderRadius: radius.pill,
  fontFamily: font.body,
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: '0.08em',
  whiteSpace: 'nowrap',
};

export function Chip(props: ChipProps) {
  const { kind, tone = 'neutral', color, title, style, children } = props;
  const { colors, reduceMotion } = useTheme();
  const accent = color ?? toneColor(tone, colors);

  // A state chip's `asOf` is read through the app's one freshness vocabulary,
  // so "last confirmed" reads the same here as it does anywhere else.
  const asOf = kind === 'state' ? props.asOf : null;
  const freshness = useFreshness(asOf ?? null, { unknownLabel: '' });

  const live = kind === 'state';
  const pulsing = live && props.pulse === true && !reduceMotion;

  const hover = kind === 'filter' ? { cursor: 'pointer' } : null;
  const shell: CSSProperties = {
    ...SHELL,
    color: accent,
    // The distinction that matters: a live or active chip is FILLED, a fixed
    // label is an outline. Fill reads as signal; outline reads as caption.
    background: kind === 'static' ? 'transparent'
      : kind === 'filter' && !props.pressed ? 'transparent'
        : withAlpha(accent, 0.14),
    border: `1px solid ${withAlpha(accent, kind === 'static' ? 0.28 : 0.42)}`,
    ...(kind === 'count' ? { fontFamily: font.mono, ...tabularNums } : null),
    ...hover,
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

  if (kind === 'filter') {
    return (
      <button
        type="button"
        aria-pressed={props.pressed}
        onClick={props.onClick}
        title={explain}
        data-testid={props['data-testid']}
        style={shell}
      >
        {body}
      </button>
    );
  }

  return (
    <span title={explain} data-testid={props['data-testid']} style={shell}>
      {body}
    </span>
  );
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
