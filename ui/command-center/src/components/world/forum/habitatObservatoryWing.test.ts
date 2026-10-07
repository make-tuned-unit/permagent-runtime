import { describe, expect, it } from 'vitest';
import { Mesh, MeshBasicMaterial, Raycaster, Vector3 } from 'three';
import {
  OBSERVATORY_BRIDGE,
  OBSERVATORY_GARDEN_PLACE,
  OBSERVATORY_PAD,
  OBSERVATORY_PLINTH,
  OBSERVATORY_SEAM_OVERLAP,
  OBSERVATORY_WING_HEIGHT,
  isOutsideObservatoryPlinth,
  mainTerraceAreaEstimate,
  observatoryBridgeMetreCells,
  observatorySeamSamples,
  observatoryWingAdditionalArea,
  observatoryWingUnionArea,
  onObservatoryBridge,
  onObservatoryPad,
  onObservatoryWing,
} from './habitatObservatoryWing';
import { findHabitatPath } from './habitatMotion';
import { HABITAT_START, habitatCollisionSource, habitatWalkable, onMainTerrace, onHabitat } from './habitatLayout';
import { observatoryWingHullGeometry, terraceSurfaceGeometry } from './habitatTerrace';
import { registerForumCollision, attemptWalk } from './walkCollision';

describe('observatory garden wing (r27)', () => {
  it('adds at least 40% floor area beyond the main terrace footprint', () => {
    const main = mainTerraceAreaEstimate();
    const additional = observatoryWingAdditionalArea(onMainTerrace);
    expect(main).toBeGreaterThan(1380);
    expect(additional / main).toBeGreaterThanOrEqual(0.40);
    expect(observatoryWingUnionArea()).toBeGreaterThan(additional);
  });

  it('clips bridge metre cells to the declared rectangle without whole-metre spill', () => {
    for (const { x, z } of observatoryBridgeMetreCells()) {
      expect(x + 0.5).toBeGreaterThanOrEqual(OBSERVATORY_BRIDGE.x0);
      expect(x + 0.5).toBeLessThanOrEqual(OBSERVATORY_BRIDGE.x1);
      expect(z + 0.5).toBeGreaterThanOrEqual(OBSERVATORY_BRIDGE.z0);
      expect(z + 0.5).toBeLessThanOrEqual(OBSERVATORY_BRIDGE.z1);
    }
    expect(observatoryBridgeMetreCells().some(c => c.x + 0.5 < OBSERVATORY_BRIDGE.x0)).toBe(false);
    expect(observatoryBridgeMetreCells().some(c => c.x + 0.5 > OBSERVATORY_BRIDGE.x1)).toBe(false);
  });

  it('maintains bridge/pad seam overlap and continuous walkability', () => {
    const midX = (OBSERVATORY_BRIDGE.x0 + OBSERVATORY_BRIDGE.x1) / 2;
    const seamZ = OBSERVATORY_BRIDGE.z0 + OBSERVATORY_SEAM_OVERLAP * 0.5;
    expect(onObservatoryBridge(midX, seamZ, 0)).toBe(true);
    expect(onObservatoryPad(midX, seamZ, 0)).toBe(true);
    expect(OBSERVATORY_BRIDGE.z0 + OBSERVATORY_SEAM_OVERLAP).toBeLessThanOrEqual(OBSERVATORY_PAD.cz + OBSERVATORY_PAD.hz + 0.01);

    const source = habitatCollisionSource();
    const release = registerForumCollision(source);
    try {
      for (const [x, z] of observatorySeamSamples()) {
        expect(onHabitat(x, z, 0.8), `union at ${x},${z}`).toBe(true);
        expect(habitatWalkable(x, z, 0.47), `walkable seam ${x},${z}`).toBe(true);
      }
    } finally {
      release();
      const materials = new Set<{ dispose: () => void }>();
      source.traverse(o => {
        if (o instanceof Mesh) {
          o.geometry.dispose();
          (Array.isArray(o.material) ? o.material : [o.material]).forEach(m => materials.add(m));
        }
      });
      materials.forEach(m => m.dispose());
    }
  });

  it('blocks walking through the bridge guard and across the surrounding void',()=>{
    const source=habitatCollisionSource(),release=registerForumCollision(source);
    try{
      const z=(OBSERVATORY_PAD.cz+OBSERVATORY_PAD.hz-28)/2;
      const inside=new Vector3(OBSERVATORY_BRIDGE.x1-.4,OBSERVATORY_WING_HEIGHT,z);
      expect(attemptWalk(inside,.12,0)).toBe(false);
      const outside=new Vector3(OBSERVATORY_BRIDGE.x0-.1,OBSERVATORY_WING_HEIGHT,z);
      expect(attemptWalk(outside,-.1,0)).toBe(false);
      expect(findHabitatPath(HABITAT_START,[OBSERVATORY_BRIDGE.x0-1,OBSERVATORY_WING_HEIGHT,z])).toBeNull();
    }finally{
      release();const materials=new Set<{dispose:()=>void}>();
      source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();(Array.isArray(o.material)?o.material:[o.material]).forEach(m=>materials.add(m));}});
      materials.forEach(m=>m.dispose());
    }
  });

  it('places the destination outside the instrument plinth collider', () => {
    const [x, , z] = OBSERVATORY_GARDEN_PLACE.position;
    expect(isOutsideObservatoryPlinth(x, z)).toBe(true);
    expect(Math.hypot(x - OBSERVATORY_PLINTH.x, z - OBSERVATORY_PLINTH.z)).toBeGreaterThan(OBSERVATORY_PLINTH.radius + 0.5);
    expect(onObservatoryPad(x, z, 0.5)).toBe(true);
  });

  it('routes from arrival across the bridge to the observatory garden place', () => {
    const destination = OBSERVATORY_GARDEN_PLACE.position;
    expect(habitatWalkable(destination[0], destination[2])).toBe(true);
    const source = habitatCollisionSource();
    const release = registerForumCollision(source);
    try {
      const path = findHabitatPath(HABITAT_START, destination);
      expect(path).not.toBeNull();
      const feet = new Vector3(...HABITAT_START);
      for (const target of path!) {
        let budget = 4000;
        while (Math.hypot(feet.x - target[0], feet.z - target[2]) > 0.025) {
          expect(--budget).toBeGreaterThan(0);
          const dx = target[0] - feet.x;
          const dz = target[2] - feet.z;
          const d = Math.hypot(dx, dz);
          const step = Math.min(0.05, d);
          expect(attemptWalk(feet, dx / d * step, dz / d * step)).toBe(true);
        }
      }
      expect(feet.distanceTo(new Vector3(...destination))).toBeLessThan(0.08);
      expect(onObservatoryBridge(feet.x, feet.z, 0)).toBe(false);
      expect(onObservatoryPad(feet.x, feet.z, 0)).toBe(true);
    } finally {
      release();
      const materials = new Set<{ dispose: () => void }>();
      source.traverse(o => {
        if (o instanceof Mesh) {
          o.geometry.dispose();
          (Array.isArray(o.material) ? o.material : [o.material]).forEach(m => materials.add(m));
        }
      });
      materials.forEach(m => m.dispose());
    }
  }, 60000);

  it('keeps wing floor geometry at the wing height with a hull beneath the pad', () => {
    const surface = terraceSurfaceGeometry();
    const hull = observatoryWingHullGeometry();
    const ray = new Raycaster();
    const material = new MeshBasicMaterial();
    const mesh = new Mesh(surface, material);
    try {
      for (let i = 0; i < surface.getAttribute('position').count; i++) {
        const x = surface.getAttribute('position').getX(i);
        const y = surface.getAttribute('position').getY(i);
        const z = surface.getAttribute('position').getZ(i);
        if (!onObservatoryWing(x, z, 0)) continue;
        expect(y).toBeCloseTo(OBSERVATORY_WING_HEIGHT, 4);
      }
      ray.set(new Vector3(OBSERVATORY_PLINTH.x, 12, OBSERVATORY_PLINTH.z), new Vector3(0, -1, 0));
      const hit = ray.intersectObject(mesh)[0];
      expect(hit?.point.y).toBeCloseTo(OBSERVATORY_WING_HEIGHT, 3);

      const hullPositions = hull.getAttribute('position');
      let maxHullY = -Infinity;
      for (let i = 0; i < hullPositions.count; i++) {
        maxHullY = Math.max(maxHullY, hullPositions.getY(i));
      }
      expect(maxHullY).toBeLessThanOrEqual(OBSERVATORY_WING_HEIGHT + 0.001);
    } finally {
      surface.dispose();
      hull.dispose();
      material.dispose();
    }
  });
});

