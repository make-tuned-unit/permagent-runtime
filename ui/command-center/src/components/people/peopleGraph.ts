/**
 * Layout for the People 3D graph — an ego network.
 *
 * You sit at the origin. Project centroids sit on a circle around you. Each
 * person is placed at the average of the projects they belong to, with a
 * stable hash jitter so two people in the same project do not occupy one
 * point. A person in two projects sits BETWEEN those centroids.
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

/**
 * Place project centroids evenly on a circle in the XZ plane around you.
 * Unassigned sits directly below you so it never collides with the hub or
 * the one-project ring.
 */
export function clusterPositions(projectIds: string[]): Map<string, GraphCluster> {
  const unique = [...new Set(projectIds)];
  const map = new Map<string, GraphCluster>();
  const n = unique.length;
  unique.forEach((id, i) => {
    const angle = n === 0 ? 0 : (i / n) * Math.PI * 2;
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
  const centroids = clusterPositions([...namedProjects.keys()]);
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
