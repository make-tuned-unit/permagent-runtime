/**
 * The three async-state blocks every Grow panel shares: a skeleton, a loading
 * line and an error with a retry.
 *
 * Split out of GrowView.tsx (R9) so a panel can import the state it needs
 * without importing the view that used to own them.
 */

import { ease, space, textSize } from '../../styles/tokens';
import type { ThemeColors } from '../../styles/tokens';
import { Button } from '../common/Button';
import { growAccent } from './growStyles';
import { CARD_R } from './growGeometry';

// ── Shared async-state blocks ────────────────────────────────────────────────

/**
 * Placeholder cards that occupy roughly the space the real ones will.
 *
 * A one-line "Loading…" where a stack of cards is about to appear collapses
 * the column and then springs it back open — the jolt reads as a flash even
 * when the fetch is fast. Holding the shape costs nothing and the arrival
 * becomes a fill rather than a jump.
 */
export function SkeletonCards({ colors, count = 2, height = 76 }: { colors: ThemeColors; count?: number; height?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }} aria-hidden>
      <style>{'@keyframes pa-skeleton { 0%,100% { opacity: 0.5; } 50% { opacity: 0.85; } }'}</style>
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          className="pa-skeleton"
          style={{
            height, borderRadius: CARD_R,
            background: colors.bgDeeper, border: `1px solid ${colors.border}`,
            // The one animated thing on this screen, and it is a placeholder —
            // never a glass surface (anti-slop #9). Spring curve, and the
            // app-wide prefers-reduced-motion guard in index.css stops it.
            animation: `pa-skeleton 1.6s ${ease.smooth} ${i * 0.12}s infinite`,
          }}
        />
      ))}
    </div>
  );
}

export function LoadingState({ colors, label, inline }: { colors: ThemeColors; label: string; inline?: boolean }) {
  const body = (
    <div style={{ fontSize: textSize.caption, color: colors.textDim }}>{label}</div>
  );
  if (inline) {
    return (
      <div style={{ border: `1px dashed ${colors.border}`, borderRadius: CARD_R, padding: space.huge, textAlign: 'center' }}>{body}</div>
    );
  }
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{body}</div>
  );
}

export function ErrorState({ colors, message, onRetry, inline }: { colors: ThemeColors; message: string; onRetry: () => void; inline?: boolean }) {
  const body = (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: textSize.small, color: colors.text, marginBottom: space.xs }}>{message}</div>
      <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.xl }}>Something went wrong reaching the server.</div>
      <Button
        colors={colors}
        onClick={onRetry}
        style={{ ...growAccent(colors, `${space.sm}px ${space.xxl}px`), fontSize: textSize.caption }}
      >Retry</Button>
    </div>
  );
  if (inline) {
    return (
      <div style={{ border: `1px solid ${colors.border}`, borderRadius: CARD_R, padding: space.huge }}>{body}</div>
    );
  }
  return (
    <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>{body}</div>
  );
}
