import { MeshBVH, acceleratedRaycast } from 'three-mesh-bvh';
import { Mesh, Object3D, Raycaster, Vector3, Matrix3 } from 'three';

let colliders: Mesh[] = [];
/** CPU-only meshes share authored geometry; they add no draw calls. */
export function registerForumCollision(scene: Object3D) {
  scene.updateMatrixWorld(true);
  const owned: Mesh[] = [];
  scene.traverse(o => {
    if (!(o instanceof Mesh)) return;
    const geometry = o.geometry.clone();
    geometry.boundsTree = new MeshBVH(geometry);
    const collider = new Mesh(geometry, o.material);
    collider.raycast = acceleratedRaycast;
    collider.applyMatrix4(o.matrixWorld);
    collider.updateMatrixWorld(true);
    owned.push(collider);
  });
  colliders = owned;
  return () => { if (colliders === owned) colliders = []; owned.forEach(m=>m.geometry.dispose()); };
}
export const getForumColliders = () => colliders;
const ray = new Raycaster();
ray.firstHitOnly = true;
const down = new Vector3(0,-1,0);
const direction = new Vector3();
const normal = new Vector3();
const normalMatrix = new Matrix3();
const origin = new Vector3();
const radius = .28;
const maxStep = .4;

/** Grounded exploration: swept body probes, step-up, bounded step-down, no
 * flight over gaps. Operates on the exported architecture, including stairs.
 * This is a lightweight walk controller, not a general rigid-body solver.
 */
export function attemptWalk(feet: Vector3, dx: number, dz: number, surfaces: Mesh[] = colliders): boolean {
  const length = Math.hypot(dx,dz);
  if (!surfaces.length || length < 1e-8) return false;
  origin.set(feet.x+dx,feet.y+maxStep+.015,feet.z+dz);
  ray.set(origin,down);ray.far=maxStep+.515;
  const ground = ray.intersectObjects(surfaces,false).find(hit=>{
    if (!hit.face) return false;
    normal.copy(hit.face.normal).applyMatrix3(normalMatrix.getNormalMatrix(hit.object.matrixWorld)).normalize();
    return normal.y > .65;
  });
  if (!ground) return false;
  const mesh = ground.object as Mesh;
  const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
  if (materials.some(m=>m.name==='Water')) return false;
  if (ground.point.y-feet.y > maxStep || feet.y-ground.point.y > .5) return false;
  // Probe from the candidate step height so successive stair treads do not
  // masquerade as a wall while the player is stepping upward.
  direction.set(dx/length,0,dz/length);
  for (const offset of [-radius,0,radius]) {
    for (const height of [maxStep+.03,.65,.9,1.15,1.4,1.65]) {
      origin.set(feet.x-direction.z*offset,Math.max(feet.y,ground.point.y)+height,feet.z+direction.x*offset);
      ray.set(origin,direction); ray.far = length+radius;
      if (ray.intersectObjects(surfaces,false).length) return false;
    }
  }
  feet.set(feet.x+dx,ground.point.y,feet.z+dz);
  return true;
}
