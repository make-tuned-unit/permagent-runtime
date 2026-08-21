import { describe, expect, it } from 'vitest';
import type { DirectoryPerson } from '../projects/types';
import {
  clusterAngleStep,
  EGO_NODE_ID,
  isBridge,
  isYou,
  layoutPeopleGraph,
  orderProjectsByAffinity,
  UNASSIGNED_CLUSTER_ID,
} from './peopleGraph';

function person(
  id: string,
  name: string,
  projects: { project_id: string; project_name: string }[],
): DirectoryPerson {
  return {
    entity_uuid: id,
    canonical_id: `person:${id}`,
    display_name: name,
    role: null,
    company: null,
    email: null,
    phone: null,
    notes: null,
    last_contact_at: null,
    birthday: null,
    relationship_strength: null,
    how_met: null,
    linkedin: null,
    x_handle: null,
    facebook: null,
    instagram: null,
    personal_site: null,
    photo_url: null,
    find_online_hints: null,
    created_at: 't',
    updated_at: 't',
    projects,
  };
}

function projectEdges(layout: ReturnType<typeof layoutPeopleGraph>) {
  return layout.edges.filter(e => e.kind === 'project');
}

describe('layoutPeopleGraph', () => {
  it('places you at the center and connects every person to you', () => {
    const layout = layoutPeopleGraph([
      person('a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }]),
      person('b', 'Bea', []),
    ]);
    const you = layout.nodes.find(n => n.id === EGO_NODE_ID)!;
    expect(isYou(you)).toBe(true);
    expect(you.x).toBe(0);
    expect(you.z).toBe(0);
    expect(isBridge(you)).toBe(false);
    expect(layout.edges.filter(e => e.kind === 'ego')).toEqual([
      { from: EGO_NODE_ID, to: 'a', via: EGO_NODE_ID, kind: 'ego' },
      { from: EGO_NODE_ID, to: 'b', via: EGO_NODE_ID, kind: 'ego' },
    ]);
    // Unassigned people sit below you, not on top of the hub.
    const bea = layout.nodes.find(n => n.id === 'b')!;
    expect(bea.y).toBeLessThan(you.y - 0.8);
  });

  it('still shows you when the directory is empty — the network starts with you', () => {
    const layout = layoutPeopleGraph([]);
    expect(layout.nodes).toHaveLength(1);
    expect(isYou(layout.nodes[0])).toBe(true);
    expect(layout.edges).toEqual([]);
  });

  it('draws a project edge when two people share a project', () => {
    const layout = layoutPeopleGraph([
      person('a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }]),
      person('b', 'Bea', [{ project_id: 'p1', project_name: 'Alpha' }]),
    ]);
    expect(projectEdges(layout)).toEqual([{ from: 'a', to: 'b', via: 'p1', kind: 'project' }]);
    expect(layout.clusters.map(c => c.id)).toEqual(['p1']);
  });

  it('does not connect unassigned people who share no project (except through you)', () => {
    const layout = layoutPeopleGraph([
      person('a', 'Ada', []),
      person('b', 'Bea', []),
    ]);
    expect(projectEdges(layout)).toEqual([]);
    expect(layout.clusters.some(c => c.id === UNASSIGNED_CLUSTER_ID)).toBe(true);
  });

  it('places a person on two projects between those centroids (the bridge)', () => {
    const layout = layoutPeopleGraph([
      person('bridge', 'Casey', [
        { project_id: 'p1', project_name: 'Alpha' },
        { project_id: 'p2', project_name: 'Beta' },
      ]),
      person('only-a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }]),
      person('only-b', 'Bea', [{ project_id: 'p2', project_name: 'Beta' }]),
    ]);
    const bridge = layout.nodes.find(n => n.id === 'bridge')!;
    const ada = layout.nodes.find(n => n.id === 'only-a')!;
    const bea = layout.nodes.find(n => n.id === 'only-b')!;
    expect(isBridge(bridge)).toBe(true);
    expect(isBridge(ada)).toBe(false);
    expect(projectEdges(layout)).toEqual(
      expect.arrayContaining([
        { from: 'bridge', to: 'only-a', via: 'p1', kind: 'project' },
        { from: 'bridge', to: 'only-b', via: 'p2', kind: 'project' },
      ]),
    );
    const dxA = Math.hypot(bridge.x - ada.x, bridge.z - ada.z);
    const dxB = Math.hypot(bridge.x - bea.x, bridge.z - bea.z);
    const dxAB = Math.hypot(ada.x - bea.x, ada.z - bea.z);
    expect(dxA).toBeGreaterThan(0.4);
    expect(dxB).toBeGreaterThan(0.4);
    expect(dxA + dxB).toBeLessThan(dxAB + 2.5);
    // Two shared projects sit beside each other, not opposite — otherwise the
    // average ("between") collapses onto you and the person looks like they
    // jumped to whichever cluster was associated last.
    const alpha = layout.clusters.find(c => c.id === 'p1')!;
    const beta = layout.clusters.find(c => c.id === 'p2')!;
    const clusterSep = Math.hypot(alpha.x - beta.x, alpha.z - beta.z);
    const opposite = 2 * 6;
    expect(clusterSep).toBeLessThan(opposite * 0.55);
    expect(Math.hypot(bridge.x, bridge.z)).toBeGreaterThan(3.5);
    expect(dxA).toBeLessThan(dxAB);
    expect(dxB).toBeLessThan(dxAB);
  });

  it('reorients the ring so two projects sharing a person sit side by side', () => {
    const people = [
      person('a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }]),
      person('b', 'Bea', [{ project_id: 'p2', project_name: 'Beta' }]),
      person('c', 'Cara', [{ project_id: 'p3', project_name: 'Gamma' }]),
      person('bridge', 'Casey', [
        { project_id: 'p1', project_name: 'Alpha' },
        { project_id: 'p3', project_name: 'Gamma' },
      ]),
    ];
    expect(orderProjectsByAffinity(['p1', 'p2', 'p3'], people, new Map([
      ['p1', 'Alpha'],
      ['p2', 'Beta'],
      ['p3', 'Gamma'],
    ]))).toEqual(['p1', 'p3', 'p2']);
    const layout = layoutPeopleGraph(people);
    const ids = layout.clusters.filter(c => c.id !== UNASSIGNED_CLUSTER_ID).map(c => c.id);
    const i1 = ids.indexOf('p1');
    const i3 = ids.indexOf('p3');
    const n = ids.length;
    const ringGap = Math.min(Math.abs(i1 - i3), n - Math.abs(i1 - i3));
    expect(ringGap).toBe(1);
  });

  it('keeps three projects that share one person as a contiguous arc', () => {
    const people = [
      person('bridge', 'Casey', [
        { project_id: 'p1', project_name: 'Alpha' },
        { project_id: 'p2', project_name: 'Beta' },
        { project_id: 'p3', project_name: 'Gamma' },
      ]),
      person('d', 'Dee', [{ project_id: 'p4', project_name: 'Delta' }]),
    ];
    const order = orderProjectsByAffinity(['p1', 'p2', 'p3', 'p4'], people, new Map([
      ['p1', 'Alpha'],
      ['p2', 'Beta'],
      ['p3', 'Gamma'],
      ['p4', 'Delta'],
    ]));
    const triple = ['p1', 'p2', 'p3'].map(id => order.indexOf(id)).sort((a, b) => a - b);
    expect(triple[1] - triple[0]).toBe(1);
    expect(triple[2] - triple[1]).toBe(1);
  });

  it('caps neighbor spacing so two projects are never opposite on the ring', () => {
    expect(clusterAngleStep(2)).toBeLessThan(Math.PI / 2);
    expect(clusterAngleStep(6)).toBeCloseTo(Math.PI / 3, 5);
  });

  it('keeps layout stable across calls (hash jitter, not random)', () => {
    const people = [person('a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }])];
    const first = layoutPeopleGraph(people);
    const second = layoutPeopleGraph(people);
    expect(first.nodes).toEqual(second.nodes);
  });

  it('copies a stored photo onto the node so the renderer can draw a face', () => {
    const withPhoto = {
      ...person('a', 'Ada', [{ project_id: 'p1', project_name: 'Alpha' }]),
      photo_url: 'https://cdn.example.com/ada.jpg',
    };
    const layout = layoutPeopleGraph([withPhoto]);
    expect(layout.nodes.find(n => n.id === 'a')?.photoUrl).toBe('https://cdn.example.com/ada.jpg');
  });
});
