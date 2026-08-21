/**
 * Layout for the People 3D graph — an ego network.
 *
 * You sit at the origin. Project centroids sit around you. People in one
 * project gather at that centroid; a person in several sits BETWEEN those
 * centroids. Shared membership also pulls the projects themselves together:
 * those clusters sit side by side on the ring, so the person stays organized
 * in the graph instead of jumping to whichever project was associated last
 * (or collapsing onto you when two projects land opposite each other).
 *
 * Edges:
 *   - ego: every person connects to you. That is the network as it exists
 *     today — your contacts, radiating out.
 *   - project: two people share an edge when they share a project. These are
 *     the first glimpse of connections that do not run through you; they stay
 *     dimmer than ego edges so the hub reads first.
 *
 * Unassigned people form a cluster below you rather than vanishing, and they
 * still get an ego edge (they are in your directory).
 *
 * Pure: no three.js, no React. The renderer just draws what this returns.
 */

import type { DirectoryPerson } from '../projects/types';

export const UNASSIGNED_CLUSTER_ID = '__unassigned__';
export const EGO_NODE_ID = '__you__';

export interface GraphCluster {
  id: string;
  name: string;
  x: number;
  y: number;
  z: number;
}

export interface GraphNode {
  id: string;
  name: string;
  company: string | null;
  role: string | null;
  photoUrl: string | null;
  lastContactAt: string | null;
  projectIds: string[];
  kind: 'person' | 'you';
  x: number;
  y: number;
  z: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  /** Shared project id, UNASSIGNED_CLUSTER_ID, or EGO_NODE_ID. */
  via: string;
  kind: 'ego' | 'project';
}

export interface PeopleGraphLayout {
  clusters: GraphCluster[];
  nodes: GraphNode[];
  edges: GraphEdge[];
}

const CLUSTER_RADIUS = 6;
const JITTER = 1.35;
/** Neighbors on the ring stay beside each other, never 180° apart. */
const MAX_NEIGHBOR_ANGLE = Math.PI / 3;
const YOU: GraphNode = {
  id: EGO_NODE_ID,
  name: 'You',
  company: null,
  role: null,
  photoUrl: null,
  lastContactAt: null,
  projectIds: [],
  kind: 'you',
  x: 0,
  y: 0.35,
  z: 0,
};

function hash01(s: string, salt: string): number {
  let h = 2166136261;
  const src = `${salt}:${s}`;
  for (let i = 0; i < src.length; i++) {
    h = Math.imul(h ^ src.charCodeAt(i), 16777619);
  }
  return ((h >>> 0) % 10000) / 10000;
}

function jitter(id: string, axis: 'x' | 'y' | 'z'): number {
  return (hash01(id, axis) - 0.5) * 2 * JITTER;
}

function uniquePreserve(ids: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

function pairKey(a: string, b: string): string {
  return a < b ? `${a}\0${b}` : `${b}\0${a}`;
}

function circularDistance(i: number, j: number, n: number): number {
  const d = Math.abs(i - j);
  return Math.min(d, n - d);
}

/** Shared-person counts between every pair of projects. */
export function projectAffinity(people: DirectoryPerson[]): Map<string, number> {
  const affinity = new Map<string, number>();
  for (const person of people) {
    const ids = uniquePreserve(person.projects.map(p => p.project_id));
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const key = pairKey(ids[i], ids[j]);
        affinity.set(key, (affinity.get(key) ?? 0) + 1);
      }
    }
  }
  return affinity;
}

function arrangementCost(order: string[], affinity: Map<string, number>): number {
  const n = order.length;
  if (n < 2) return 0;
  const index = new Map(order.map((id, i) => [id, i]));
  let cost = 0;
  for (const [key, weight] of affinity) {
    if (weight <= 0) continue;
    const sep = key.indexOf('\0');
    const a = key.slice(0, sep);
    const b = key.slice(sep + 1);
    const i = index.get(a);
    const j = index.get(b);
    if (i === undefined || j === undefined) continue;
    cost += weight * circularDistance(i, j, n);
  }
  return cost;
}

function reverseArc(order: string[], from: number, to: number): string[] {
  const next = order.slice();
  let i = from;
  let j = to;
  while (i < j) {
    const tmp = next[i];
    next[i] = next[j];
    next[j] = tmp;
    i += 1;
    j -= 1;
  }
  return next;
}

function improveCircular(order: string[], affinity: Map<string, number>): string[] {
  const n = order.length;
  if (n < 4) return order;
  let current = order;
  let currentCost = arrangementCost(current, affinity);
  let improved = true;
  let guard = 0;
  while (improved && guard < 40) {
    improved = false;
    guard += 1;
    for (let i = 0; i < n - 1; i++) {
      for (let j = i + 2; j < n; j++) {
        if (i === 0 && j === n - 1) continue;
        const next = reverseArc(current, i, j);
        const cost = arrangementCost(next, affinity);
        if (cost < currentCost) {
          current = next;
          currentCost = cost;
          improved = true;
        }
      }
    }
  }
  return current;
}

function sortKey(id: string, names: Map<string, string>): string {
  return `${names.get(id) ?? id}\0${id}`;
}

function compareProjects(a: string, b: string, names: Map<string, string>): number {
  const ka = sortKey(a, names);
  const kb = sortKey(b, names);
  if (ka < kb) return -1;
  if (ka > kb) return 1;
  return 0;
}

function componentOf(
  start: string,
  remaining: Set<string>,
  affinity: Map<string, number>,
): string[] {
  const comp: string[] = [];
  const queue = [start];
  remaining.delete(start);
  while (queue.length > 0) {
    const id = queue.shift()!;
    comp.push(id);
    for (const other of [...remaining]) {
      if ((affinity.get(pairKey(id, other)) ?? 0) > 0) {
        remaining.delete(other);
        queue.push(other);
      }
    }
  }
  return comp;
}

function strongestNeighbor(id: string, placed: string[], affinity: Map<string, number>): string {
  let best = placed[0];
  let bestWeight = -1;
  for (const other of placed) {
    const w = affinity.get(pairKey(id, other)) ?? 0;
    if (w > bestWeight || (w === bestWeight && other < best)) {
      bestWeight = w;
      best = other;
    }
  }
  return best;
}

function pathCostAt(order: string[], index: number, node: string, affinity: Map<string, number>): number {
  const trial = order.slice();
  trial.splice(index, 0, node);
  const pos = new Map(trial.map((id, i) => [id, i]));
  const i = pos.get(node)!;
  let cost = 0;
  for (const other of order) {
    const w = affinity.get(pairKey(node, other)) ?? 0;
    if (w <= 0) continue;
    cost += w * Math.abs(i - pos.get(other)!);
  }
  return cost;
}

function arrangeComponent(ids: string[], affinity: Map<string, number>, names: Map<string, string>): string[] {
  const sorted = [...ids].sort((a, b) => compareProjects(a, b, names));
  if (sorted.length <= 2) return sorted;

  let bestEdge = { a: sorted[0], b: sorted[1], w: -1 };
  for (let i = 0; i < sorted.length; i++) {
    for (let j = i + 1; j < sorted.length; j++) {
      const w = affinity.get(pairKey(sorted[i], sorted[j])) ?? 0;
      if (
        w > bestEdge.w
        || (w === bestEdge.w && pairKey(sorted[i], sorted[j]) < pairKey(bestEdge.a, bestEdge.b))
      ) {
        bestEdge = { a: sorted[i], b: sorted[j], w };
      }
    }
  }

  const order = compareProjects(bestEdge.a, bestEdge.b, names) <= 0
    ? [bestEdge.a, bestEdge.b]
    : [bestEdge.b, bestEdge.a];
  const unused = sorted.filter(id => id !== order[0] && id !== order[1]);

  while (unused.length > 0) {
    let pick = 0;
    let pickScore = -1;
    for (let i = 0; i < unused.length; i++) {
      let score = 0;
      for (const placed of order) {
        score = Math.max(score, affinity.get(pairKey(unused[i], placed)) ?? 0);
      }
      if (
        score > pickScore
        || (score === pickScore && compareProjects(unused[i], unused[pick], names) < 0)
      ) {
        pick = i;
        pickScore = score;
      }
    }
    const node = unused.splice(pick, 1)[0];
    const neighbor = strongestNeighbor(node, order, affinity);
    const at = order.indexOf(neighbor);
    const left = pathCostAt(order, at, node, affinity);
    const right = pathCostAt(order, at + 1, node, affinity);
    if (left < right || (left === right && compareProjects(node, neighbor, names) < 0)) {
      order.splice(at, 0, node);
    } else {
      order.splice(at + 1, 0, node);
    }
  }
  return order;
}

/**
 * Circular order so projects that share people sit next to each other.
 * Isolated projects follow in name order. Deterministic.
 */
export function orderProjectsByAffinity(
  projectIds: string[],
  people: DirectoryPerson[],
  names: Map<string, string> = new Map(),
): string[] {
  const unique = uniquePreserve(projectIds);
  if (unique.length <= 1) return unique;
  const affinity = projectAffinity(people);
  const remaining = new Set(unique);
  const seeds = [...unique].sort((a, b) => compareProjects(a, b, names));
  const components: string[][] = [];
  for (const seed of seeds) {
    if (!remaining.has(seed)) continue;
    components.push(arrangeComponent(componentOf(seed, remaining, affinity), affinity, names));
  }
  return improveCircular(components.flat(), affinity);
}

export function clusterAngleStep(projectCount: number): number {
  if (projectCount <= 1) return 0;
  return Math.min((Math.PI * 2) / projectCount, MAX_NEIGHBOR_ANGLE);
}

/**
 * Place project centroids around you, in the given order.
 *
 * Related projects are ordered first (`orderProjectsByAffinity`) so neighbors
 * on this ring are the ones that share people. Adjacent centroids never sit
 * more than 60° apart — two projects sharing a person are side by side, not
 * opposite — and the arc is centered toward the camera (+Z).
 *
 * Unassigned sits directly below you so it never collides with the hub or
 * the project ring.
 */
export function clusterPositions(projectIds: string[]): Map<string, GraphCluster> {
  const unique = uniquePreserve(projectIds);
  const map = new Map<string, GraphCluster>();
  const n = unique.length;
  const step = clusterAngleStep(n);
  const start = Math.PI / 2 - ((n - 1) * step) / 2;
  unique.forEach((id, i) => {
    const angle = start + i * step;
    map.set(id, {
      id,
      name: id,
      x: Math.cos(angle) * CLUSTER_RADIUS,
      y: 0,
      z: Math.sin(angle) * CLUSTER_RADIUS,
    });
  });
  map.set(UNASSIGNED_CLUSTER_ID, {
    id: UNASSIGNED_CLUSTER_ID,
    name: 'No project',
    x: 0,
    y: -2.2,
    z: 0,
  });
  return map;
}

export function layoutPeopleGraph(people: DirectoryPerson[]): PeopleGraphLayout {
  const namedProjects = new Map<string, string>();
  for (const person of people) {
    for (const project of person.projects) {
      namedProjects.set(project.project_id, project.project_name);
    }
  }
  const ordered = orderProjectsByAffinity([...namedProjects.keys()], people, namedProjects);
  const centroids = clusterPositions(ordered);
  for (const [id, name] of namedProjects) {
    const cluster = centroids.get(id);
    if (cluster) cluster.name = name;
  }

  const nodes: GraphNode[] = people.map(person => {
    const projectIds = person.projects.map(p => p.project_id);
    const keys = projectIds.length > 0 ? projectIds : [UNASSIGNED_CLUSTER_ID];
    let x = 0;
    let y = 0;
    let z = 0;
    for (const key of keys) {
      const c = centroids.get(key)!;
      x += c.x;
      y += c.y;
      z += c.z;
    }
    const n = keys.length;
    return {
      id: person.entity_uuid,
      name: person.display_name,
      company: person.company,
      role: person.role,
      photoUrl: person.photo_url,
      lastContactAt: person.last_contact_at,
      projectIds,
      kind: 'person',
      x: x / n + jitter(person.entity_uuid, 'x'),
      y: y / n + jitter(person.entity_uuid, 'y') * 0.45,
      z: z / n + jitter(person.entity_uuid, 'z'),
    };
  });

  const byProject = new Map<string, string[]>();
  for (const node of nodes) {
    const keys = node.projectIds.length > 0 ? node.projectIds : [UNASSIGNED_CLUSTER_ID];
    for (const key of keys) {
      const list = byProject.get(key) ?? [];
      list.push(node.id);
      byProject.set(key, list);
    }
  }

  const edges: GraphEdge[] = [];
  const seen = new Set<string>();
  for (const [via, ids] of byProject) {
    // Unassigned people cluster together visually; they are not "connected
    // between projects", so they get no project edges. They still get an ego
    // edge below — they are in the directory.
    if (via === UNASSIGNED_CLUSTER_ID) continue;
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const a = ids[i] < ids[j] ? ids[i] : ids[j];
        const b = ids[i] < ids[j] ? ids[j] : ids[i];
        const key = `${a}|${b}|${via}`;
        if (seen.has(key)) continue;
        seen.add(key);
        edges.push({ from: a, to: b, via, kind: 'project' });
      }
    }
  }

  const egoEdges: GraphEdge[] = nodes.map(node => ({
    from: EGO_NODE_ID,
    to: node.id,
    via: EGO_NODE_ID,
    kind: 'ego',
  }));

  const clusters = [...centroids.values()].filter(c => {
    if (c.id === UNASSIGNED_CLUSTER_ID) {
      return people.some(p => p.projects.length === 0);
    }
    return people.some(p => p.projects.some(pr => pr.project_id === c.id));
  });

  return { clusters, nodes: [{ ...YOU }, ...nodes], edges: [...egoEdges, ...edges] };
}

/** True when a person belongs to more than one project — the bridge case. */
export function isBridge(node: GraphNode): boolean {
  return node.kind === 'person' && node.projectIds.length > 1;
}

export function isYou(node: GraphNode): boolean {
  return node.kind === 'you';
}
