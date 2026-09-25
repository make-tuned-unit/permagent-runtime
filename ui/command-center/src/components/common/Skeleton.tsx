/**
 * Skeleton — a loading placeholder in the final layout shape (R2), not a
 * spinner in the void. Pure CSS shimmer (`.pa-skeleton`, index.css), so it
 * costs nothing to mount many of at once (a list of ten skeleton rows) and
 * collapses to a static block under reduced motion via the same app-wide
 * guard every other animation here honours.
 *
 * `SkeletonText` is the common case — one or more lines, the last one
 * shorter so a paragraph-shaped skeleton doesn't look like a single bar
 * repeated. `Skeleton` is the primitive for anything else (an avatar circle,
 * a stat tile, a thumbnail) — give it the exact box the real content fills.
 */
import type { CSSProperties } from 'react';
import { radius, space } from '../../styles/tokens';

export function Skeleton({ width = '100%', height = 14, radius: r = radius.sm, style }: {
  width?: number | string;
  height?: number | string;
  radius?: number;
  style?: CSSProperties;
}) {
  return (
    <div
      className="pa-skeleton"
      aria-hidden="true"
      style={{
        width, height,
        '--pa-skel-radius': `${r}px`,
        ...style,
      } as CSSProperties}
    />
  );
}

export function SkeletonText({ lines = 3, lastLineWidth = '60%', gap = space.sm }: {
  lines?: number;
  lastLineWidth?: number | string;
  gap?: number;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap }} aria-hidden="true">
      {Array.from({ length: lines }, (_, i) => (
        <Skeleton key={i} width={i === lines - 1 ? lastLineWidth : '100%'} />
      ))}
    </div>
  );
}
