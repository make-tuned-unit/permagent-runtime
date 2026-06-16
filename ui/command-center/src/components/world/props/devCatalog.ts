// W2 — dev catalog mount points (Cave §4 / §9 W2 handoff).
// LIVE placement now comes from W1's areas/strata/anchors (SCAFFOLD_MOUNTS):
// see areas/strata/CaveConstruction.tsx, which dresses each real ScaffoldMount
// with the matching kit piece. THIS file is the DEV CATALOG ONLY: a hand-placed
// set of mount transforms that compose the construction kit into a self-contained
// "scaffold-state vignette" (the construction-site-at-first-light read) and
// assemble the Cornerstone Gallery template for visual review on the PropCatalog
// stage. NOT part of the product build — consumed only by props/catalog.

type Vec3 = [number, number, number];

export interface MountPoint {
  position: Vec3;
  rotationY?: number;
}

interface ScaffoldMount extends MountPoint {
  bays: number;
  levels: number;
}
interface PileMount extends MountPoint {
  seed: number;
  count: number;
}
interface SurveyMount extends MountPoint {
  size: [number, number];
  seed: number;
}
interface SeededMount extends MountPoint {
  seed: number;
}

/** A scaffold-state vignette: a chamber mid-construction at first light. */
export const SCAFFOLD_VIGNETTE: {
  crane: MountPoint;
  scaffolds: ScaffoldMount[];
  stonePiles: PileMount[];
  surveyLines: SurveyMount[];
  toolRacks: MountPoint[];
  lamp: MountPoint;
  minerals: SeededMount[];
  roots: SeededMount[];
  tending: { sledge: MountPoint; setting: MountPoint };
} = {
  crane: { position: [-6, 0, 0], rotationY: 0.3 },
  scaffolds: [
    { position: [2, 0, -2], rotationY: 0, bays: 3, levels: 3 },
    { position: [7, 0, 1], rotationY: -0.6, bays: 2, levels: 2 },
  ],
  stonePiles: [
    { position: [4, 0, 3], seed: 1, count: 9 },
    { position: [-1, 0, 4], seed: 5, count: 7 },
  ],
  surveyLines: [{ position: [3, 0, -8], rotationY: 0.2, size: [8, 5], seed: 3 }],
  toolRacks: [{ position: [0, 0, 2], rotationY: 0.4 }],
  lamp: { position: [-2, 0, 0.5], rotationY: 0 },
  minerals: [{ position: [9, 0, -3], seed: 2 }],
  roots: [{ position: [-4, 0, 5], seed: 4 }],
  tending: {
    sledge: { position: [-3, 0, 3], rotationY: 0.5 },
    setting: { position: [6, 0, -3], rotationY: -0.3 },
  },
};

interface ReliefMount extends MountPoint {
  slots: number;
}

/** The Cornerstone Gallery template, assembled (Cave §6). */
export const GALLERY_ASSEMBLY: {
  flights: number;
  ascent: MountPoint;
  inscription: MountPoint;
  reliefWall: ReliefMount;
  plinths: MountPoint[];
  crackedStone: MountPoint;
} = {
  flights: 3,
  ascent: { position: [0, 0, 0], rotationY: 0 },
  inscription: { position: [-2.4, 0, -1.5], rotationY: 0.4 },
  reliefWall: { position: [-4.5, 0, 4], rotationY: Math.PI / 2, slots: 4 },
  plinths: [
    { position: [-3.5, 0, 1], rotationY: 0.6 },
    { position: [-3.5, 0, 7], rotationY: 0.9 },
  ],
  crackedStone: { position: [4.5, 0, 1], rotationY: -0.4 },
  // The landing sits at the top of the ascent (see ascentTop()).
};
