// Agora population — the DATA seam for the Forum crowd (#306 prep).
//
// SCALING MODEL (read this before touching the crowd):
// The Agora is where sovereign, chiti-identified agents gather — and there may
// eventually be THOUSANDS present at once. The crowd is therefore rendered as a
// COUNT, never as a collection of mounted components. One THREE.Points renders
// the whole population in a single draw call whether it is 0, 100, or 10,000
// (see GlyphField.tsx). This module is the honest, data-driven source of that
// count and (when available) the per-occupant identities.
//
// HONESTY LAW (WORLD_VIEW_BIBLE.md §4): population reads ONLY from the frozen
// shared/meshStatus.ts contract. Offline today ⇒ count 0 ⇒ empty Agora,
// "awaits the mesh". Never fabricate peers.
//
// IDENTITY: the frozen meshStatus contract exposes a peerCount (a number) but no
// per-peer list yet. So today the crowd renders the REAL count with procedural
// per-slot variety (a stable hash of the slot index → hue/height), which is NOT
// a claim of identity — it is visual texture over a truthful headcount. When the
// Mesh contract grows a real occupant list, populate `occupants` here and the
// crowd colours itself from real chiti-IDs with no renderer change. The
// AgoraOccupant.isSelf seam distinguishes "you + your own sovereign agent"
// (rendered full-detail at the dais) from the instanced crowd.

import { useMeshStatus } from '../../shared/meshStatus';

/** Hard cap on rendered crowd instances. Beyond this the Forum shows density /
 *  "and N more" rather than trying to place 10k anything. */
export const AGORA_VISIBLE_CAP = 512;

export interface AgoraOccupant {
  /** Chiti-ID (sovereign identity). */
  id: string;
  displayName: string;
  /** True for the viewer's own sovereign agent (rendered full-detail, not crowd). */
  isSelf: boolean;
}

export interface AgoraPopulation {
  /** Real total peer count from meshStatus (0 while offline). */
  total: number;
  /** How many crowd instances to actually render = min(total, cap). */
  visible: number;
  /** total - visible; surfaced as "and N more" density, never mounted. */
  overflow: number;
  /** Real per-occupant identities when the contract provides them (empty today). */
  occupants: AgoraOccupant[];
  online: boolean;
}

/** Stable hash → [0,1) for procedural per-slot variety (no identity claim). */
export function slotHash(n: number): number {
  const s = Math.sin(n * 12.9898 + 78.233) * 43758.5453;
  return s - Math.floor(s);
}

/** Hue [0,1) derived from a chiti-ID string — a peer's colour when a real list
 *  exists. Deterministic so a given sovereign always reads the same colour. */
export function chitiHue(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return (h % 3600) / 3600;
}

/**
 * The live Agora population, derived honestly from the frozen meshStatus.
 * Offline ⇒ empty. Connected ⇒ the real peerCount, capped for rendering.
 */
export function useAgoraPopulation(): AgoraPopulation {
  const mesh = useMeshStatus();
  if (mesh.state !== 'connected') {
    return { total: 0, visible: 0, overflow: 0, occupants: [], online: false };
  }
  const total = Math.max(0, mesh.peerCount);
  const visible = Math.min(total, AGORA_VISIBLE_CAP);
  return {
    total,
    visible,
    overflow: total - visible,
    occupants: [], // no per-peer list in the contract yet (documented gap)
    online: true,
  };
}
