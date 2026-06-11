// Agent anchor registry — the W2 → W3 interface. WORLD_VIEW_BIBLE.md §6.
// API FROZEN after Phase 0. W2 registers anchors with its props; W3 consumes them
// for seated/engaged working postures and locomotion targets.

export type AreaId = 'hall' | 'build' | 'brain' | 'lab' | 'automate' | 'antechamber';
export type AnchorKind = 'seat' | 'stand' | 'lean';

export interface AgentAnchor {
  /** e.g. 'build.bench2.seatL' */
  id: string;
  areaId: AreaId;
  kind: AnchorKind;
  /** World-space position. Seat anchors assume seated agent height ~1.7u. */
  position: [number, number, number];
  /** Y rotation the agent assumes when engaged. */
  facing: number;
}

const registry = new Map<string, AgentAnchor>();
const claims = new Map<string, string>(); // anchorId -> agentId

/** W2 only: register anchors when a prop mounts. Re-registering the same id replaces it. */
export function registerAnchors(anchors: AgentAnchor[]): void {
  for (const a of anchors) registry.set(a.id, a);
}

/** W2 only: remove anchors when a prop unmounts (lazy-unloaded area). */
export function unregisterAnchors(ids: string[]): void {
  for (const id of ids) {
    registry.delete(id);
    claims.delete(id);
  }
}

export function getAnchors(areaId?: AreaId): AgentAnchor[] {
  const all = [...registry.values()];
  return areaId ? all.filter((a) => a.areaId === areaId) : all;
}

export function getAnchor(id: string): AgentAnchor | undefined {
  return registry.get(id);
}

/** Simple in-memory lock so two agents never share a seat. */
export function claimAnchor(id: string, agentId: string): boolean {
  if (!registry.has(id)) return false;
  const holder = claims.get(id);
  if (holder && holder !== agentId) return false;
  claims.set(id, agentId);
  return true;
}

export function releaseAnchor(id: string): void {
  claims.delete(id);
}

export function releaseAgentAnchors(agentId: string): void {
  for (const [id, holder] of claims) if (holder === agentId) claims.delete(id);
}
