import { describe, expect, it } from 'vitest';
import { Box3, Group, Mesh, Vector3 } from 'three';
import {
  TELESCOPE_ASSEMBLY_HEIGHT,
  TELESCOPE_BARREL_LENGTH,
  TELESCOPE_BARREL_ROTATION_Z,
  TELESCOPE_ELEVATION,
  buildObservatoryTelescope,
} from './habitatObservatoryTelescope';

const DEW_CAP_LENGTH = 0.14;
const SCALE = 0.92;

function meshCount(root: Group): number {
  let count = 0;
  root.traverse((node) => { if (node instanceof Mesh) count += 1; });
  return count;
}

function triangleCount(root: Group): number {
  let tris = 0;
  root.traverse((node) => {
    if (node instanceof Mesh) {
      const geo = node.geometry;
      if (geo.index) tris += geo.index.count / 3;
      else if ('attributes' in geo && geo.attributes.position) tris += geo.attributes.position.count / 3;
    }
  });
  return tris;
}

function findBarrelGroup(root: Group): Group {
  const barrel = root.getObjectByName('barrel assembly');
  if (!(barrel instanceof Group)) throw new Error('missing barrel assembly group');
  return barrel;
}

describe('observatory telescope (r27)', () => {
  it('keeps all barrel parts on one rotated local Y axis with coherent tip placement', () => {
    const scope = buildObservatoryTelescope();
    const barrelGroup = findBarrelGroup(scope);
    expect(barrelGroup.rotation.z).toBeCloseTo(TELESCOPE_BARREL_ROTATION_Z, 5);
    expect(TELESCOPE_BARREL_ROTATION_Z).toBeCloseTo(-Math.PI / 3, 5);
    expect(TELESCOPE_ELEVATION).toBeCloseTo(Math.PI / 6, 5);

    const dewCap = barrelGroup.children.find((child) => {
      return child instanceof Mesh && child.position.y === TELESCOPE_BARREL_LENGTH - DEW_CAP_LENGTH;
    });
    expect(dewCap).toBeDefined();

    const lens = barrelGroup.children.find((child) => {
      return child instanceof Mesh && child.position.y === TELESCOPE_BARREL_LENGTH;
    });
    expect(lens).toBeDefined();

    expect(lens!.position.x).toBe(0);
    expect(lens!.position.z).toBe(0);
    expect(dewCap!.position.x).toBe(0);
    expect(dewCap!.position.z).toBe(0);
    expect(TELESCOPE_BARREL_LENGTH - dewCap!.position.y).toBeCloseTo(DEW_CAP_LENGTH, 5);

    scope.updateMatrixWorld(true);
    const objectiveTip = new Vector3(0, TELESCOPE_BARREL_LENGTH, 0);
    barrelGroup.localToWorld(objectiveTip);
    const lensWorld = new Vector3();
    lens!.getWorldPosition(lensWorld);
    expect(lensWorld.distanceTo(objectiveTip)).toBeLessThan(0.02);
  });

  it('respects draw-call and triangle budgets with cast shadows on primary meshes', () => {
    const scope = buildObservatoryTelescope();
    expect(meshCount(scope)).toBeLessThanOrEqual(20);
    expect(triangleCount(scope)).toBeLessThan(24000);

    let shadowed = 0;
    scope.traverse((node) => { if (node instanceof Mesh && node.castShadow) shadowed += 1; });
    expect(shadowed).toBe(meshCount(scope));
  });

  it('returns a disposable root group owned by the caller', () => {
    const scope = buildObservatoryTelescope();
    expect(scope).toBeInstanceOf(Group);
    expect(scope.name).toBe('observatory telescope');
    expect(scope.scale.x).toBeCloseTo(SCALE, 5);

    const materials = new Set<unknown>();
    scope.traverse((node) => {
      if (node instanceof Mesh) materials.add(node.material);
    });
    expect(materials.size).toBe(4);

    scope.updateMatrixWorld(true);
    const bounds = new Box3().setFromObject(scope);
    expect(bounds.min.y).toBeGreaterThanOrEqual(0);
    expect(bounds.max.y).toBeLessThan(TELESCOPE_ASSEMBLY_HEIGHT);
    expect(bounds.max.y - bounds.min.y).toBeGreaterThan(2);
  });

  it('supports the barrel midpoint on a transverse axle through both fork arms', () => {
    const scope = buildObservatoryTelescope();
    scope.updateMatrixWorld(true);
    const barrel = findBarrelGroup(scope);
    const pivot = barrel.localToWorld(new Vector3(0, TELESCOPE_BARREL_LENGTH / 2, 0));
    const left = scope.getObjectByName('trunnion -0.3')!;
    const right = scope.getObjectByName('trunnion 0.3')!;
    const a = left.getWorldPosition(new Vector3());
    const b = right.getWorldPosition(new Vector3());
    expect(a.clone().add(b).multiplyScalar(0.5).distanceTo(pivot)).toBeLessThan(1e-6);
    const axis = b.clone().sub(a).normalize();
    const opticalAxis = barrel.localToWorld(new Vector3(0, TELESCOPE_BARREL_LENGTH / 2 + 1, 0)).sub(pivot).normalize();
    expect(Math.abs(axis.dot(opticalAxis))).toBeLessThan(1e-6);
    const collar = new Box3().setFromObject(barrel.getObjectByName('pivot collar')!);
    expect(collar.intersectsBox(new Box3().setFromObject(left))).toBe(true);
    expect(collar.intersectsBox(new Box3().setFromObject(right))).toBe(true);
  });
});
