// Forum Antechamber status contract — WORLD_VIEW_BIBLE.md §6.
// FROZEN after Phase 0. The Antechamber's status surface reads ONLY from this module.
// Mesh is offline/dormant today; when Mesh ships, real state plugs in here and the
// room updates without touching area code. No daemon code, no endpoints behind this.

export type MeshStatus =
  | { state: 'offline' }
  | { state: 'connecting' }
  | { state: 'connected'; peerCount: number };

const OFFLINE: MeshStatus = { state: 'offline' };

export function useMeshStatus(): MeshStatus {
  return OFFLINE;
}
