import { Mesh, Object3D, Vector3 } from 'three';

/** GLTFLoader sanitizes spaces and periods in `name`, retaining the authored
 * name in userData. Resolve that stable source name for articulated joints. */
export function heroNode(root: Object3D, name: string): Object3D | undefined {
  let found: Object3D | undefined;
  root.traverse(node => { if (!found && (node.userData.name === name || node.name === name)) found = node; });
  return found;
}

export function replaceFixedHeroMouth(head: Object3D) {
  const brass = heroNode(head, 'Head_Brass • brushed hardware');
  if (!(brass instanceof Mesh)) return null;
  brass.updateMatrix();
  const geometry = brass.geometry.clone(), positions = geometry.getAttribute('position'), index = geometry.getIndex(), keep: number[] = [], v = new Vector3();
  for (let i = 0; i < (index?.count ?? positions.count); i += 3) {
    const ids = [0, 1, 2].map(j => index ? index.getX(i + j) : i + j);
    const mouth = ids.every(n => { v.fromBufferAttribute(positions, n).applyMatrix4(brass.matrix); return Math.abs(v.x) < .14 && v.y < -.04 && v.z > .23; });
    if (!mouth) keep.push(...ids);
  }
  geometry.setIndex(keep); brass.geometry = geometry;
  return geometry;
}
