// W2 prop library — anchor registration + placement helpers.
// Bridges prop-local layouts to the frozen shared/anchors.ts registry (§6).

import { useEffect } from 'react';
import type { AgentAnchor } from '../shared/anchors';
import { registerAnchors, unregisterAnchors } from '../shared/anchors';

/**
 * Register this prop's anchors while mounted; unregister (and drop claims)
 * on unmount so lazily-unloaded areas never advertise stale seats.
 * `anchors` must be referentially stable (useMemo) — re-registering on every
 * render would churn the registry.
 */
export function useRegisterAnchors(anchors: AgentAnchor[]): void {
  useEffect(() => {
    registerAnchors(anchors);
    return () => unregisterAnchors(anchors.map((a) => a.id));
  }, [anchors]);
}

/**
 * Transform a prop-local anchor position into world space for a prop placed
 * at `origin` with Y rotation `rotY`, and compose the facing angle.
 */
export function placeAnchor(
  local: [number, number, number],
  localFacing: number,
  origin: [number, number, number],
  rotY: number
): { position: [number, number, number]; facing: number } {
  const cos = Math.cos(rotY);
  const sin = Math.sin(rotY);
  const [lx, ly, lz] = local;
  return {
    position: [origin[0] + lx * cos + lz * sin, origin[1] + ly, origin[2] - lx * sin + lz * cos],
    facing: localFacing + rotY,
  };
}
