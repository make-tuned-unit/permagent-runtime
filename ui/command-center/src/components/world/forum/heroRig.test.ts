import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { Mesh, Vector3 } from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { heroNode, replaceFixedHeroMouth } from './heroRig';

it('binds real GLTF joint names and replaces only the fixed mouth without changing shared geometry', async () => {
  const bytes = readFileSync(new URL('../../../../public/world/garden-hero/hero-kit-v3b.glb', import.meta.url));
  const size = bytes.readUInt32LE(12), json = JSON.parse(bytes.subarray(20, 20 + size).toString());
  // Mesh transforms and geometry are untouched. Omit material references so
  // the actual loader can run without a browser texture decoder in this test.
  for (const mesh of json.meshes) for (const primitive of mesh.primitives) delete primitive.material;
  const text = Buffer.from(JSON.stringify(json)), padded = Buffer.alloc(Math.ceil(text.length / 4) * 4, 32); text.copy(padded);
  const bin = bytes.subarray(20 + size), header = Buffer.alloc(20); bytes.copy(header, 0, 0, 12);
  header.writeUInt32LE(20 + padded.length + bin.length, 8); header.writeUInt32LE(padded.length, 12); header.writeUInt32LE(0x4e4f534a, 16);
  const fixture = Buffer.concat([header, padded, bin]);
  const gltf = await new GLTFLoader().parseAsync(fixture.buffer.slice(fixture.byteOffset, fixture.byteOffset + fixture.byteLength), '');
  const robot = heroNode(gltf.scene, 'robot')!;
  expect(robot).toBeTruthy();
  for (const name of ['Head', 'Arm.L', 'Arm.R', 'Leg.L', 'Leg.R']) {
    const joint = heroNode(robot, name)!; expect(joint, name).toBeTruthy();
    joint.updateWorldMatrix(true, true); const before = joint.localToWorld(new Vector3(0, -.2, 0));
    joint.rotation.x = .4; joint.updateWorldMatrix(true, true);
    expect(joint.localToWorld(new Vector3(0, -.2, 0)).distanceTo(before)).toBeGreaterThan(.04);
  }
  const clone = robot.clone(true), head = heroNode(clone, 'Head')!, brass = heroNode(head, 'Head_Brass • brushed hardware') as Mesh;
  const original = brass.geometry, count = original.index!.count;
  const mouth = replaceFixedHeroMouth(head)!;
  expect(mouth).not.toBe(original); expect(mouth.index!.count).toBeLessThan(count);
  expect(mouth.index!.count).toBeGreaterThan(count / 2); expect(original.index!.count).toBe(count);
  mouth.dispose(); gltf.scene.traverse(n => { if (n instanceof Mesh) n.geometry.dispose(); });
});
