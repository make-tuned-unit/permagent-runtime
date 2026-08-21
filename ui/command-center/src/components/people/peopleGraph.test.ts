import { describe, expect, it } from 'vitest';
import type { DirectoryPerson } from '../projects/types';
import {
  EGO_NODE_ID,
  isBridge,
  isYou,
  layoutPeopleGraph,
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
