/**
 * Pure formatting + mapping helpers for the data-dense Settings panes (Spend,
 * Sovereignty, Models — migrated from the retired Governance surface). Kept
 * separate from the components so they are unit-testable without a DOM.
 */

import type { ThemeColors } from '../../styles/tokens';

/** Format a USD amount. Sub-cent spend still reads as a real number (4dp) so a
 *  $0.003 call never rounds to "$0.00" and looks free. */
export function formatUsd(v: number): string {
  if (!Number.isFinite(v)) return '$0.00';
  if (v > 0 && v < 0.01) return `$${v.toFixed(4)}`;
  return `$${v.toFixed(2)}`;
}

/** Compact token count: 1.2k, 3.4M. */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0';
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** The four budget bands map to a semantic color. `ok` is muted (nothing to
 *  flag); soft warns; gate/hard escalate to danger. */
export function bandColor(band: string, colors: ThemeColors): string {
  switch (band) {
    case 'soft':
      return colors.warning;
    case 'gate':
      return colors.danger;
    case 'hard':
      return colors.danger;
    default:
      return colors.textDim;
  }
}

/** Human sentence for a band, for a11y labels / tooltips. */
export function bandLabel(band: string): string {
  switch (band) {
    case 'soft':
      return 'over the soft warning threshold';
    case 'gate':
      return 'at the gate — pauses for approval';
    case 'hard':
      return 'at the hard stop';
    default:
      return 'within budget';
  }
}

/** Relative "x ago" for an RFC-3339 / SQLite timestamp; empty string when
 *  unparseable so callers can omit rather than render "NaN". */
export function timeAgo(iso: string): string {
  if (!iso) return '';
  // SQLite CURRENT_TIMESTAMP is "YYYY-MM-DD HH:MM:SS" (space, UTC, no zone);
  // normalize so Date parses it as UTC rather than local.
  const norm = iso.includes('T') ? iso : `${iso.replace(' ', 'T')}Z`;
  const t = Date.parse(norm);
  if (Number.isNaN(t)) return '';
  const secs = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}
