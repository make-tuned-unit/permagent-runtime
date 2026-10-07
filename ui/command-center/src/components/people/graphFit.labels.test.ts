/**
 * QA 2026-09-24 at 1456px: People → Graph drew its cluster chips ("LOVE NOVA
 * SCOTIA", "PLEKK", "HARBOURVIEW RESIDENTS' ASSOCIATION", "NO PROJECT") far
 * from the faces they name. The chips were relaxed as free bodies at the ring
 * centroid while a big cluster's faces spread out around it. Each chip must sit
 * right over its own members on screen.
 */
import { describe, expect, it } from 'vitest';
import type { DirectoryPerson } from '../projects/types';
import { layoutPeopleGraph, UNASSIGNED_CLUSTER_ID } from './peopleGraph';
import {
  chipHalfExtent, clusterMembers, faceHalfExtent, fitPeopleGraph, projectPoint,
  type Insets, type Viewport,
} from './graphFit';

function person(id: string, name: string, projects: [string, string][]): DirectoryPerson {
  return {
    entity_uuid: id, canonical_id: `person:${id}`, display_name: name,
    role: null, company: null, email: null, phone: null, notes: null, last_contact_at: null,
    birthday: null, relationship_strength: null, how_met: null, linkedin: null, x_handle: null,
    facebook: null, instagram: null, personal_site: null, photo_url: null, find_online_hints: null,
    created_at: 't', updated_at: 't',
    projects: projects.map(([project_id, project_name]) => ({ project_id, project_name })),
  } as DirectoryPerson;
}

const LNS: [string, string] = ['lns', 'Love Ontario'];
const PLK: [string, string] = ['plk', 'Lumio'];
const HRA: [string, string] = ['hra', "Harbourview Residents' Association"];

/** The live shape: 36 people, most of them in no project. */
const PEOPLE: DirectoryPerson[] = [
  ...Array.from({ length: 5 }, (_, i) => person(`l${i}`, `Lns Person ${i}`, [LNS])),
  ...Array.from({ length: 3 }, (_, i) => person(`k${i}`, `Lumio Person ${i}`, [PLK])),
  ...Array.from({ length: 4 }, (_, i) => person(`h${i}`, `Harbour Person ${i}`, [HRA])),
  person('bridge', 'Bridge Person', [LNS, HRA]),
  ...Array.from({ length: 23 }, (_, i) => person(`u${i}`, `Unassigned Person ${i}`, [])),
];

const VIEWPORT: Viewport = { width: 1100, height: 760 };
const INSETS: Insets = { top: 56, right: 16, bottom: 16, left: 16 };

describe('cluster chips sit on their members', () => {
  const layout = layoutPeopleGraph(PEOPLE);
  const fitted = fitPeopleGraph(layout.nodes, layout.clusters, VIEWPORT, INSETS);
  const nodes = layout.nodes.map(n => {
    const p = fitted.nodes.get(n.id)!;
    return { ...n, x: p[0], y: p[1], z: p[2] };
  });

  for (const cluster of layout.clusters) {
    it(`"${cluster.name}" is over its own people`, () => {
      const chip = projectPoint(fitted.labels.get(cluster.id)!, fitted.camera, VIEWPORT);
      const members = clusterMembers(nodes, cluster.id).map(n => {
        const s = projectPoint([n.x, n.y, n.z], fitted.camera, VIEWPORT);
        return { ...s, ...faceHalfExtent(n) };
      });
      expect(members.length).toBeGreaterThan(0);
      const left = Math.min(...members.map(m => m.x - m.hw));
      const right = Math.max(...members.map(m => m.x + m.hw));
      const top = Math.min(...members.map(m => m.y - m.hh));
      const bottom = Math.max(...members.map(m => m.y + m.hh));
      const { hw, hh } = chipHalfExtent(cluster.name);
      // Horizontally: the chip overlaps its members' span.
      expect(chip.x + hw).toBeGreaterThan(left);
      expect(chip.x - hw).toBeLessThan(right);
      // Vertically: within one face-height of the members' box, above or below.
      const gap = chip.y < top ? top - (chip.y + hh) : chip.y > bottom ? (chip.y - hh) - bottom : 0;
      expect(gap).toBeLessThan(48);
    });
  }

  it('shows no chip for a cluster with no people', () => {
    const noUnassigned = layoutPeopleGraph(PEOPLE.filter(p => p.projects.length > 0));
    expect(noUnassigned.clusters.map(c => c.id)).not.toContain(UNASSIGNED_CLUSTER_ID);
  });

  it('names a cluster by its single-project members, not the bridge between two', () => {
    const lns = clusterMembers(nodes, 'lns').map(n => n.id);
    expect(lns).not.toContain('bridge');
    expect(lns).toHaveLength(5);
  });
});
