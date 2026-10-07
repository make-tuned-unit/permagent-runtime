/**
 * QA 2026-09-24: at the real window size the People graph (~660x560) clipped
 * faces at the left edge, cut "HARBOURVIEW RESIDENTS' A…" at the right, and
 * stacked the "No project" faces on each other and on the front project's
 * chip. The camera was a constant tuned for a wide window, and the faces and
 * chips are fixed-pixel overlays that never shrink. These pin the fit.
 */
import { describe, expect, it } from 'vitest';
import type { DirectoryPerson } from '../projects/types';
import { layoutPeopleGraph } from './peopleGraph';
import {
  chipHalfExtent,
  faceHalfExtent,
  fitPeopleGraph,
  GRAPH_FOV,
  insetsAvoiding,
  LABEL_LIFT,
  projectPoint,
  type CameraPose,
  type Insets,
  type Rect,
  type Vec3,
  type Viewport,
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

const HRA: [string, string] = ['hra', "Harbourview Residents' Association"];
const LNS: [string, string] = ['lns', 'Love Ontario'];
const SOL: [string, string] = ['sol', 'Solar Forum'];
const PER: [string, string] = ['per', 'Permagent'];

/** Roughly the shape of the real directory in the QA walk. */
const PEOPLE: DirectoryPerson[] = [
  person('bt', 'Ben Tran', [HRA]), person('tb', 'Tara Boyd', [HRA]), person('ta', 'Tom Ash', [HRA]),
  person('mk', 'Mei Kato', [HRA, LNS]),
  person('ln1', 'Liam Neil', [LNS]), person('ln2', 'Nora Lee', [LNS]),
  person('s1', 'Sam Oduya', [SOL]), person('s2', 'Ola Sun', [SOL]), person('s3', 'Ivy Ross', [SOL, PER]),
  person('p1', 'Pat Quinn', [PER]), person('p2', 'Ravi Das', [PER]),
  ...Array.from({ length: 12 }, (_, i) => person(`u${i}`, `Unassigned ${i}`, [])),
];

const VIEWPORT: Viewport = { width: 660, height: 560 };
/** PeopleGraphCanvas's GRAPH_INSETS: filter field at the top, 16px elsewhere. */
const BASE: Insets = { top: 56, right: 16, bottom: 16, left: 16 };
/** The Key, measured in the real-size harness at 660x560 (canvas coords). */
const KEY_OPEN: Rect = { left: 16, top: 73, right: 346, bottom: 544 };
const KEY_CHIP: Rect = { left: 16, top: 514, right: 60, bottom: 544 };
const INSETS: Insets = insetsAvoiding(VIEWPORT, BASE, KEY_CHIP);

interface Box { id: string; x: number; y: number; hw: number; hh: number }

function boxes(cam: CameraPose, nodes: Map<string, Vec3>, labels: Map<string, Vec3>): Box[] {
  const layout = layoutPeopleGraph(PEOPLE);
  const out: Box[] = [];
  for (const n of layout.nodes) {
    const s = projectPoint(nodes.get(n.id)!, cam, VIEWPORT);
    out.push({ id: n.id, x: s.x, y: s.y, ...faceHalfExtent(n) });
  }
  for (const c of layout.clusters) {
    const s = projectPoint(labels.get(c.id)!, cam, VIEWPORT);
    out.push({ id: `chip:${c.name}`, x: s.x, y: s.y, ...chipHalfExtent(c.name) });
  }
  return out;
}

function clipped(bs: Box[]): string[] {
  return bs.filter(b =>
    b.x - b.hw < INSETS.left - 0.5 || b.x + b.hw > VIEWPORT.width - INSETS.right + 0.5
    || b.y - b.hh < INSETS.top - 0.5 || b.y + b.hh > VIEWPORT.height - INSETS.bottom + 0.5,
  ).map(b => b.id);
}

function overlaps(bs: Box[], tolerance: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < bs.length; i++) {
    for (let j = i + 1; j < bs.length; j++) {
      const ox = bs[i].hw + bs[j].hw - Math.abs(bs[i].x - bs[j].x);
      const oy = bs[i].hh + bs[j].hh - Math.abs(bs[i].y - bs[j].y);
      if (ox > tolerance && oy > tolerance) out.push(`${bs[i].id}×${bs[j].id}`);
    }
  }
  return out;
}

describe('the old fixed camera at the real size (what QA saw)', () => {
  it('clips bodies and stacks the faces', () => {
    const layout = layoutPeopleGraph(PEOPLE);
    const cam: CameraPose = { position: [0, 7.5, 14], target: [0, 0, 0], fov: GRAPH_FOV };
    const nodes = new Map(layout.nodes.map(n => [n.id, [n.x, n.y, n.z] as Vec3]));
    const labels = new Map(layout.clusters.map(c => [c.id, [c.x, c.y + LABEL_LIFT, c.z] as Vec3]));
    const bs = boxes(cam, nodes, labels);
    expect(clipped(bs).length).toBeGreaterThan(0);
    expect(overlaps(bs, 2).length).toBeGreaterThan(0);
  });
});

describe('fitPeopleGraph at the real size', () => {
  const layout = layoutPeopleGraph(PEOPLE);
  const fit = fitPeopleGraph(layout.nodes, layout.clusters, VIEWPORT, INSETS);
  const bs = boxes(fit.camera, fit.nodes, fit.labels);

  it('keeps every face and chip inside the viewport, clear of the chrome', () => {
    expect(clipped(bs)).toEqual([]);
  });

  it('leaves no face or chip on top of another', () => {
    expect(overlaps(bs, 2)).toEqual([]);
  });

  it('keeps you where you were', () => {
    const you = layout.nodes.find(n => n.kind === 'you')!;
    expect(fit.nodes.get(you.id)).toEqual([you.x, you.y, you.z]);
  });

  it('is deterministic', () => {
    const again = fitPeopleGraph(layout.nodes, layout.clusters, VIEWPORT, INSETS);
    expect(again.camera).toEqual(fit.camera);
    expect([...again.nodes]).toEqual([...fit.nodes]);
  });

  it('also fits a wide window and a tiny one', () => {
    for (const vp of [{ width: 1400, height: 800 }, { width: 420, height: 380 }]) {
      const f = fitPeopleGraph(layout.nodes, layout.clusters, vp, INSETS);
      for (const n of layout.nodes) {
        const s = projectPoint(f.nodes.get(n.id)!, f.camera, vp);
        const h = faceHalfExtent(n);
        expect(s.x - h.hw).toBeGreaterThanOrEqual(INSETS.left - 0.5);
        expect(s.x + h.hw).toBeLessThanOrEqual(vp.width - INSETS.right + 0.5);
        expect(s.y - h.hh).toBeGreaterThanOrEqual(INSETS.top - 0.5);
        expect(s.y + h.hh).toBeLessThanOrEqual(vp.height - INSETS.bottom + 0.5);
      }
    }
  });
});

function intersects(b: Box, r: Rect): boolean {
  return b.x + b.hw > r.left && b.x - b.hw < r.right && b.y + b.hh > r.top && b.y - b.hh < r.bottom;
}

function bounds(bs: Box[]) {
  return {
    left: Math.min(...bs.map(b => b.x - b.hw)), right: Math.max(...bs.map(b => b.x + b.hw)),
    top: Math.min(...bs.map(b => b.y - b.hh)), bottom: Math.max(...bs.map(b => b.y + b.hh)),
  };
}

describe('fitPeopleGraph around the Key (QA 2026-09-24 follow-up)', () => {
  const layout = layoutPeopleGraph(PEOPLE);

  for (const [name, key] of [['key closed (chip)', KEY_CHIP], ['key open (panel)', KEY_OPEN]] as const) {
    describe(name, () => {
      const insets = insetsAvoiding(VIEWPORT, BASE, key);
      const fit = fitPeopleGraph(layout.nodes, layout.clusters, VIEWPORT, insets);
      const bs = boxes(fit.camera, fit.nodes, fit.labels);

      it('no face or chip intersects the key rect', () => {
        expect(bs.filter(b => intersects(b, key)).map(b => b.id)).toEqual([]);
      });

      it('nothing is clipped and nothing is stacked', () => {
        const avail = bs.filter(b =>
          b.x - b.hw < insets.left - 0.5 || b.x + b.hw > VIEWPORT.width - insets.right + 0.5
          || b.y - b.hh < insets.top - 0.5 || b.y + b.hh > VIEWPORT.height - insets.bottom + 0.5);
        expect(avail.map(b => b.id)).toEqual([]);
        expect(overlaps(bs, 2)).toEqual([]);
      });

      it('is centred in the free rectangle and fills it along its binding side', () => {
        const b = bounds(bs);
        const aw = VIEWPORT.width - insets.left - insets.right;
        const ah = VIEWPORT.height - insets.top - insets.bottom;
        expect(Math.abs((b.left + b.right) / 2 - (insets.left + aw / 2))).toBeLessThan(6);
        expect(Math.abs((b.top + b.bottom) / 2 - (insets.top + ah / 2))).toBeLessThan(6);
        expect(Math.max((b.right - b.left) / aw, (b.bottom - b.top) / ah)).toBeGreaterThan(0.9);
      });
    });
  }

  it('the key-closed fit uses most of the height too (the old fit left the bottom third empty)', () => {
    const insets = insetsAvoiding(VIEWPORT, BASE, KEY_CHIP);
    const fit = fitPeopleGraph(layout.nodes, layout.clusters, VIEWPORT, insets);
    const b = bounds(boxes(fit.camera, fit.nodes, fit.labels));
    const ah = VIEWPORT.height - insets.top - insets.bottom;
    expect((b.bottom - b.top) / ah).toBeGreaterThan(0.8);
  });
});

describe('insetsAvoiding', () => {
  it('keeps the larger free side of an obstacle', () => {
    const i = insetsAvoiding(VIEWPORT, BASE, KEY_OPEN, 8);
    expect(i.left).toBe(KEY_OPEN.right + 8);
    expect(i.top).toBe(BASE.top);
  });
  it('ignores an obstacle outside the area and a missing one', () => {
    expect(insetsAvoiding(VIEWPORT, BASE, null)).toBe(BASE);
    expect(insetsAvoiding(VIEWPORT, BASE, { left: 0, top: 0, right: 10, bottom: 10 })).toBe(BASE);
  });
});
