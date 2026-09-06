import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { Group, Mesh, BoxGeometry, MeshStandardMaterial, SkinnedMesh, Vector3 } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { decodeBlenderArmor } from './blenderArmor';
import { BONE_NAMES } from './poses';
import { ROSTER } from './roster';
import { createAgentRig } from './rig';

describe('Blender character binding', () => {
  it.each(ROSTER.map(agent => agent.id))('binds %s to existing live bones in three bounded draws', async id => {
    const file = readFileSync(`public/world/characters/${id}.glb`);
    const bytes = file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength);
    const gltf = await new GLTFLoader().parseAsync(bytes, '');
    const outfits = new Set<string>();
    gltf.scene.traverse(object => {
      if (!(object instanceof Mesh)) return;
      expect(object.userData.identity).toBe(id);
      expect(object.userData.outfit).toBeTruthy();
      outfits.add(object.userData.outfit);
    });
    expect(outfits.size).toBe(1);
    const result = decodeBlenderArmor(gltf.scene);
    expect(Object.keys(result)).toHaveLength(3);
    let vertices = 0;
    for (const geometry of Object.values(result)) {
      const position = geometry.getAttribute('position');
      const indices = geometry.getAttribute('skinIndex');
      const weights = geometry.getAttribute('skinWeight');
      vertices += position.count;
      expect(position.count).toBeGreaterThan(0);
      for (let i = 0; i < position.count; i++) {
        expect(indices.getX(i)).toBeLessThan(BONE_NAMES.length);
        expect(weights.getX(i)).toBe(1);
      }
      geometry.computeBoundingBox();
      expect(geometry.boundingBox!.max.y).toBeLessThan(2.8);
      expect(geometry.boundingBox!.min.y).toBeGreaterThan(-.1);
      geometry.dispose();
    }
    expect(vertices).toBeLessThan(90_000);
  });

  it('rejects missing bone attribution rather than silently animating the wrong part', () => {
    const scene = new Group();
    scene.add(new Mesh(new BoxGeometry(), new MeshStandardMaterial()));
    expect(() => decodeBlenderArmor(scene)).toThrow('invalid bone/channel');
  });

  it('moves authored forearm vertices with the existing pose bone, not a parallel animation rig', async () => {
    const file = readFileSync('public/world/characters/henry.glb');
    const gltf = await new GLTFLoader().parseAsync(file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength), '');
    const armor = decodeBlenderArmor(gltf.scene);
    const rig = createAgentRig({ trimColor: '#F0E6D0', weathering: 0, crown: true, armor });
    const body = rig.root.children.find(object => object instanceof SkinnedMesh && object.geometry === armor.metal) as SkinnedMesh;
    expect(body).toBeDefined();
    const skin = body.geometry.getAttribute('skinIndex');
    let index = 0;
    while (index < skin.count && skin.getX(index) !== BONE_NAMES.indexOf('foreL')) index++;
    expect(index).toBeLessThan(skin.count);
    rig.root.updateMatrixWorld(true);
    const base = new Vector3().fromBufferAttribute(body.geometry.getAttribute('position'), index);
    const before = body.applyBoneTransform(index, base.clone());
    rig.bones.foreL.rotation.x = .6;
    rig.root.updateMatrixWorld(true);
    const after = body.applyBoneTransform(index, base.clone());
    expect(after.distanceTo(before)).toBeGreaterThan(.005);
    expect(rig.trimMat.color.getHexString()).toBe('f0e6d0');
    expect(rig.stateMat).not.toBe(rig.trimMat);
    rig.dispose();
    Object.values(armor).forEach(geometry => geometry.dispose());
  });
});
