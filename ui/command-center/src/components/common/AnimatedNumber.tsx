/**
 * AnimatedNumber — a stat tile figure that counts instead of snapping (R8
 * item 7: "alive"). Dashboard/finance/grow tiles re-render a raw number on
 * every poll; nothing distinguished "this changed" from "this repainted",
 * and a number that silently becomes a different number reads as static data
 * rather than as a live thing the agent is tracking.
 *
 * Tabular figures (so digits don't reflow mid-count — R3), formats through
 * the same `Intl.NumberFormat` options a caller would hand `toLocaleString`,
 * and jumps straight to the target with no animation at all under reduced
 * motion (D14) or on first mount (a tile should not count up from zero the
 * first time it ever renders — only when a value it already showed changes).
 */
import { useEffect, useRef, useState } from 'react';
import { tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export interface AnimatedNumberProps {
  value: number;
  /** Passed to `Intl.NumberFormat`. */
  format?: Intl.NumberFormatOptions;
  locale?: string;
  durationMs?: number;
  className?: string;
  style?: React.CSSProperties;
  /** Text before/after the number, inside the same tabular-nums span (a unit
   *  suffix, a currency prefix) — kept in the same element so it never wraps
   *  onto its own line mid-count. */
  prefix?: string;
  suffix?: string;
}

function easeOutQuad(t: number): number {
  return 1 - (1 - t) * (1 - t);
}

export function AnimatedNumber({
  value, format, locale, durationMs = 500, className, style, prefix = '', suffix = '',
}: AnimatedNumberProps) {
  const { reduceMotion } = useTheme();
  const [display, setDisplay] = useState(value);
  // The actually-painted value, not the last-requested target — an
  // interrupted animation (a new poll landing before the current count-up
  // finishes) must continue from wherever it visually is, not snap to the
  // target it was still travelling toward.
  const displayed = useRef(value);
  const mounted = useRef(false);
  const raf = useRef<number | null>(null);

  useEffect(() => {
    const from = displayed.current;
    const to = value;

    if (!mounted.current) {
      mounted.current = true;
      displayed.current = to;
      setDisplay(to);
      return;
    }
    if (reduceMotion || from === to) {
      displayed.current = to;
      setDisplay(to);
      return;
    }

    const start = performance.now();
    const tick = (now: number) => {
      const t = Math.min(1, (now - start) / durationMs);
      const next = from + (to - from) * easeOutQuad(t);
      displayed.current = next;
      setDisplay(next);
      if (t < 1) {
        raf.current = requestAnimationFrame(tick);
      } else {
        displayed.current = to;
        setDisplay(to);
      }
    };
    raf.current = requestAnimationFrame(tick);
    return () => { if (raf.current != null) cancelAnimationFrame(raf.current); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, durationMs, reduceMotion]);

  const rounded = format?.maximumFractionDigits === undefined && format?.minimumFractionDigits === undefined
    ? Math.round(display)
    : display;
  const text = new Intl.NumberFormat(locale, format).format(rounded);

  return (
    <span className={className} style={{ ...tabularNums, ...style }}>
      {prefix}{text}{suffix}
    </span>
  );
}
