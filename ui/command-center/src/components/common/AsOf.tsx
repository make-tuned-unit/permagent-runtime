/**
 * `<AsOf>` — how old something is, said the same way everywhere.
 *
 * The vocabulary lives in `useFreshness`; this is the one place it is drawn.
 * Two surfaces had their own version of this line and their own colour for it,
 * which meant a user learning what "stale" looks like on Home learned nothing
 * that transferred to the Brain.
 *
 * Deliberately quiet: it inherits its font and size from wherever it is
 * dropped, and touches only colour — a timestamp is a caption, not a badge.
 * While the reading is fresh it is plain text. Once it is stale it takes the
 * `stale` role and can carry a dot, so the difference survives for anyone who
 * doesn't separate those two ambers.
 */

import { useTheme } from '../../styles/useTheme';
import { useFreshness, type UseFreshnessOptions } from '../../hooks/useFreshness';
import { radius } from '../../styles/tokens';

export interface AsOfProps extends UseFreshnessOptions {
  /** When the thing was last true. Epoch millis, a Date, or a wire timestamp. */
  asOf: number | string | Date | null | undefined;
  /** Leads the reading — "Updated", "Last run", "Synced". Dropped when there
   *  is no date at all, because there is then no "updated" to date. */
  prefix?: string;
  /** Trails it after a middot — "reconnecting", "not polling". */
  suffix?: string;
  /** Show the non-verbal staleness cue. Only ever drawn when stale. */
  dot?: boolean;
  'data-testid'?: string;
}

export function AsOf({ asOf, prefix, suffix, dot, 'data-testid': testId, ...options }: AsOfProps) {
  const { colors } = useTheme();
  const freshness = useFreshness(asOf, options);
  const known = freshness.tone !== 'unknown';

  const color = freshness.tone === 'live' ? undefined
    : freshness.tone === 'stale' ? colors.stale
      : colors.textMuted;

  return (
    <span
      data-testid={testId}
      title={freshness.exact ?? undefined}
      style={{ color, display: 'inline-flex', alignItems: 'center', gap: 6 }}
    >
      {dot && freshness.stale && (
        <span
          data-testid="as-of-dot"
          aria-hidden="true"
          style={{
            width: 6, height: 6, borderRadius: radius.pill, flexShrink: 0,
            background: freshness.tone === 'stale' ? colors.stale : colors.textMuted,
            display: 'inline-block',
          }}
        />
      )}
      <span>
        {known && prefix ? `${prefix} ` : ''}
        {freshness.label}
        {suffix ? ` · ${suffix}` : ''}
      </span>
    </span>
  );
}
