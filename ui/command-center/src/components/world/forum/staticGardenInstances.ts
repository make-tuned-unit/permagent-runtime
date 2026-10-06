import { BufferGeometry, Euler, Group, InstancedMesh, Matrix4, Mesh, Quaternion, Vector3, type Material, type Object3D } from 'three';
import type { GardenDecorPlacement } from './gardenLayout';

type Prototype = { geometry: BufferGeometry; material: Material; sourceName: string; materialSlot: number };
export type GardenInstanceCensus = { placements: number; primitiveBatches: number; instances: number; triangles: number };

/** Owns baked primitive geometry/instance buffers; loader-owned materials are borrowed.
 * Authored nested transforms (including shear from rotated nonuniform scales) are
 * baked once, so Three's instance normal transform sees only uniform placement scale.
 */
export function instanceGardenDecor(scene: Object3D, placements: readonly GardenDecorPlacement[]) {
  const group = new Group();
  group.name = 'Instanced garden decor';
  const owned: BufferGeometry[] = [];
  const census: GardenInstanceCensus = { placements: placements.length, primitiveBatches: 0, instances: 0, triangles: 0 };
  const byAsset = new Map<string, GardenDecorPlacement[]>();
  for (const placement of placements) {
    if (!Number.isFinite(placement.scale) || placement.scale <= 0) throw new Error('Static placement scale must be positive');
    const rows = byAsset.get(placement.asset) ?? [];
    rows.push(placement); byAsset.set(placement.asset, rows);
  }
  try {
    for (const [asset, rows] of byAsset) {
      const source = scene.getObjectByName(asset);
      if (!source) throw new Error(`Hero kit missing ${asset}`);
      // Match the former cloneAsset/place contract exactly: root translation
      // and scale are placement-owned, Y Euler is the heading, authored X/Z
      // rotations remain. Descendant transforms are authored asset-local data.
      const root = source.clone(true);
      const rootRotation = root.rotation.clone();
      root.position.set(0,0,0); root.scale.set(1,1,1); root.quaternion.identity();
      root.updateMatrix(); root.updateMatrixWorld(true);
      const primitives: Prototype[] = [];
      root.traverseVisible(object => {
        if (!(object instanceof Mesh)) return;
        if ('isSkinnedMesh' in object || object.morphTargetInfluences?.length) throw new Error('Animated meshes cannot enter static garden batching');
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        const ranges = Array.isArray(object.material)
          ? object.geometry.groups.map(range => ({ ...range, materialIndex: range.materialIndex ?? 0 }))
          : [{ start: object.geometry.drawRange.start, count: object.geometry.drawRange.count, materialIndex: 0 }];
        for (const range of ranges) {
          const material = materials[range.materialIndex];
          if (!material || material.visible === false) continue;
          const geometry = object.geometry.clone();
          owned.push(geometry);
          geometry.clearGroups();
          const available = geometry.index?.count ?? geometry.getAttribute('position').count;
          const start = Math.max(range.start, object.geometry.drawRange.start);
          const end = Math.min(available, range.start + range.count, object.geometry.drawRange.start + object.geometry.drawRange.count);
          if (end <= start) continue;
          geometry.setDrawRange(start, end - start);
          geometry.applyMatrix4(object.matrixWorld);
          // Bounds include the whole source primitive's vertices. They are
          // conservative for multi-material ranges and never cut away geometry.
          geometry.computeBoundingBox(); geometry.computeBoundingSphere();
          primitives.push({ geometry, material, sourceName: object.name, materialSlot: range.materialIndex });
        }
      });
      for (const primitive of primitives) {
        const mesh = new InstancedMesh(primitive.geometry, primitive.material, rows.length);
        mesh.name = `${asset} / ${primitive.sourceName} / ${primitive.materialSlot}`;
        mesh.userData.asset = asset; mesh.userData.sourceName = primitive.sourceName; mesh.userData.materialSlot = primitive.materialSlot;
        mesh.castShadow = true; mesh.receiveShadow = true; mesh.frustumCulled = true;
        rows.forEach((placement, index) => {
          const rotation = new Quaternion().setFromEuler(new Euler(rootRotation.x,placement.heading,rootRotation.z,rootRotation.order));
          const matrix = new Matrix4().compose(new Vector3(...placement.position), rotation, new Vector3().setScalar(placement.scale));
          mesh.setMatrixAt(index, matrix);
        });
        mesh.instanceMatrix.needsUpdate = true;
        mesh.computeBoundingBox(); mesh.computeBoundingSphere();
        group.add(mesh);
        census.primitiveBatches++; census.instances += rows.length;
        census.triangles += primitive.geometry.drawRange.count / 3 * rows.length;
      }
    }
  } catch (error) {
    group.children.forEach(object => (object as InstancedMesh).dispose());
    owned.forEach(geometry => geometry.dispose());
    throw error;
  }
  return { group, census, dispose: () => {
    group.children.forEach(object => (object as InstancedMesh).dispose());
    owned.forEach(geometry => geometry.dispose());
  } };
}
