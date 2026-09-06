import { BufferAttribute, BufferGeometry, Color, Mesh, Object3D } from 'three';
import { mergeGeometries } from 'three/examples/jsm/utils/BufferGeometryUtils.js';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { BONE_NAMES } from './poses';

export type BlenderArmor = Record<'metal' | 'trim' | 'visor', BufferGeometry>;
const CHANNELS = ['metal', 'trim', 'visor'] as const;

/** Rebind authored rigid armor to the EXISTING pose skeleton, never a second
 * animation system. Blender exports Y-up meter geometry and explicit bone extras.
 * World matrices are baked before binding, so scaled/rotated source parts retain
 * their authored placement, rather than exploding around local mesh origins. */
export function decodeBlenderArmor(scene: Object3D): BlenderArmor {
  const chunks: Record<string, BufferGeometry[]> = { metal: [], trim: [], visor: [] };
  const owned: BufferGeometry[] = [];
  let vertices = 0;
  scene.updateMatrixWorld(true);
  try {
    scene.traverse(object => {
      if (!(object instanceof Mesh)) return;
      const { bone, channel, schema } = object.userData;
      const boneIndex = BONE_NAMES.indexOf(bone);
      if (schema !== 'permagent.rigid-armor.v1' || boneIndex < 0 || !CHANNELS.includes(channel)) {
        throw new Error('Blender armor has invalid bone/channel metadata');
      }
      const source = object.geometry;
      const geo = source.index ? source.toNonIndexed() : source.clone();
      owned.push(geo);
      geo.applyMatrix4(object.matrixWorld);
      if (!geo.attributes.normal) geo.computeVertexNormals();
      const position = geo.getAttribute('position');
      vertices += position.count;
      if (vertices > 90_000) throw new Error('Blender armor exceeds vertex budget');
      for (let i = 0; i < position.count; i++) {
        const xyz = [position.getX(i), position.getY(i), position.getZ(i)];
        if (xyz.some(v => !Number.isFinite(v) || Math.abs(v) > 4)) {
          throw new Error('Blender armor must be finite meter-scale geometry');
        }
      }
      // Uniform attributes let all parts of one channel become one draw.
      for (const attribute of Object.keys(geo.attributes)) {
        if (attribute !== 'position' && attribute !== 'normal') geo.deleteAttribute(attribute);
      }
      const indices = new Uint16Array(position.count * 4);
      const weights = new Float32Array(position.count * 4);
      const colors = new Float32Array(position.count * 3);
      const material = Array.isArray(object.material) ? object.material[0] : object.material;
      const color = (material as { color?: Color }).color ?? new Color(1, 1, 1);
      for (let i = 0; i < position.count; i++) {
        indices[i * 4] = boneIndex;
        weights[i * 4] = 1;
        colors.set([color.r, color.g, color.b], i * 3);
      }
      geo.setAttribute('skinIndex', new BufferAttribute(indices, 4));
      geo.setAttribute('skinWeight', new BufferAttribute(weights, 4));
      geo.setAttribute('color', new BufferAttribute(colors, 3));
      chunks[channel].push(geo);
    });
    const result = {} as BlenderArmor;
    for (const channel of CHANNELS) {
      if (!chunks[channel].length) throw new Error('Blender armor missing channel: ' + channel);
      const merged = mergeGeometries(chunks[channel], false);
      if (!merged) throw new Error('Blender armor could not be batched');
      owned.push(merged);
      result[channel] = merged;
    }
    // Only the three merged geometries escape; intermediates are disposed.
    for (const geo of owned) if (!Object.values(result).includes(geo)) geo.dispose();
    return result;
  } catch (error) {
    for (const geo of owned) geo.dispose();
    throw error;
  }
}

const cache = new Map<string, Promise<BlenderArmor>>();
export function loadBlenderArmor(identity: string): Promise<BlenderArmor> {
  if (!/^[a-z_]+$/.test(identity)) return Promise.reject(new Error('Invalid character identity'));
  let pending = cache.get(identity);
  if (!pending) {
    pending = new GLTFLoader().loadAsync(`${import.meta.env.BASE_URL}world/characters/${identity}.glb`)
      .then(gltf => {
        try { return decodeBlenderArmor(gltf.scene); }
        finally {
          gltf.scene.traverse(object => {
            if (!(object instanceof Mesh)) return;
            object.geometry.dispose();
            const materials = Array.isArray(object.material) ? object.material : [object.material];
            materials.forEach(material => material.dispose());
          });
        }
      });
    cache.set(identity, pending);
  }
  return pending;
}
