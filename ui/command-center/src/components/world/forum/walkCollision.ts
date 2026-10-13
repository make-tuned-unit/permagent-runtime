import { measureForumStartup } from './forumStartup';
import { MeshBVH, acceleratedRaycast } from 'three-mesh-bvh';
import { Mesh, Object3D, Raycaster, Vector3, Matrix3 } from 'three';

let colliders: Mesh[] = [];
let actorColliders: Mesh[] = [];
let combinedColliders: Mesh[] = [];
const refreshColliders=()=>{combinedColliders=[...colliders,...actorColliders];};
/** Dynamic avatar bodies are owned by their renderer and updated at its frame boundary. */
export function registerForumActorCollisions(meshes:Mesh[]){actorColliders=meshes;refreshColliders();return()=>{if(actorColliders===meshes){actorColliders=[];refreshColliders();}};}
export const getForumStaticColliders=()=>colliders;
/** CPU-only meshes share authored geometry; they add no draw calls. */
export function registerForumCollision(scene: Object3D) {
  const steps=prepareForumCollision(scene);let next=steps.next();while(!next.done)next=steps.next();return next.value;
}
/** Publish atomically only after all BVHs are ready; paused initialization must
 * never replace the live world's collision set with a partial one. */
export function* prepareForumCollision(scene:Object3D):Generator<void,()=>void>{
  scene.updateMatrixWorld(true);
  const owned:Mesh[]=[],sources:Mesh[]=[];let completed=false;
  scene.traverse(o=>{if(o instanceof Mesh)sources.push(o);});
  try {
    for(const o of sources){
      const geometry=o.geometry.clone();
      const collider=new Mesh(geometry,o.material);owned.push(collider);
      geometry.boundsTree=measureForumStartup('collision BVH',()=>new MeshBVH(geometry));
      collider.name=o.name;collider.raycast=acceleratedRaycast;
      collider.applyMatrix4(o.matrixWorld);collider.updateMatrixWorld(true);
      yield;
    }
    colliders=owned;refreshColliders();completed=true;
    return()=>{if(colliders===owned){colliders=[];refreshColliders();}owned.forEach(m=>m.geometry.dispose());};
  } finally {if(!completed)owned.forEach(m=>m.geometry.dispose());}
}

export const getForumColliders = () => combinedColliders;
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
export function attemptWalk(feet: Vector3, dx: number, dz: number, surfaces: Mesh[] = combinedColliders): boolean {
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

/**
 * Where the water surface is at a world position, or `null` if that spot is
 * not water.
 *
 * The ships needed this because guessing was wrong: the first version sailed
 * them along ellipses sized from a constant, and a good part of the plaza is
 * inside that radius, so boats crossed the ground. The world already knows
 * where its water is — the GLB names that material `Water`, which is the same
 * fact `attemptWalk` uses to refuse to walk on it — so the honest fix is to ask
 * it rather than to encode a radius.
 *
 * Rays start well above the island and take the FIRST hit, so a boat cannot be
 * placed on water that runs under a bridge or a terrace.
 */
export function waterSurfaceAt(x: number, z: number, surfaces: Mesh[] = colliders): number | null {
  if (!surfaces.length) return null;
  origin.set(x, 400, z);
  ray.set(origin, down);
  ray.far = 800;
  const hit = ray.intersectObjects(surfaces, false)[0];
  if (!hit) return null;
  const mesh = hit.object as Mesh;
  const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
  return materials.some(m => m.name === 'Water') ? hit.point.y : null;
}

/**
 * What the skiff would be landing on at a world position.
 *
 * Returns the surface height and whether it can be set down on. The rules are
 * the walker's own, because a place you can land is a place you can then walk:
 * water is out (same `Water` material the walker refuses), and so is anything
 * steeper than the walker's slope limit — a skiff parked on a 60-degree roof
 * would strand you.
 *
 * `null` means there is nothing below at all, which over open sea or off the
 * edge of the world is the honest answer, and the flight model treats it as
 * clear air rather than as ground at zero.
 */
export function landingSampleAt(
  x: number,
  z: number,
  surfaces: Mesh[] = colliders,
): { height: number; landable: boolean } | null {
  if (!surfaces.length) return null;
  origin.set(x, 1200, z);
  ray.set(origin, down);
  ray.far = 2400;
  const hit = ray.intersectObjects(surfaces, false)[0];
  if (!hit || !hit.face) return null;
  const mesh = hit.object as Mesh;
  const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
  if (materials.some(m => m.name === 'Water')) return { height: hit.point.y, landable: false };
  normal.copy(hit.face.normal).applyMatrix3(normalMatrix.getNormalMatrix(hit.object.matrixWorld)).normalize();
  return { height: hit.point.y, landable: normal.y > .65 };
}
