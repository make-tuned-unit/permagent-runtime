import { readFileSync, existsSync } from 'node:fs';
import { expect, it } from 'vitest';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { Mesh, Texture } from 'three';
import { withMeshopt } from './forumGltf';
import { decodeBlenderArmor } from '../agents/blenderArmor';
import { ROSTER } from '../agents/roster';
import { FORUM_AGENT_PROFILES } from './agentProfiles';
it('gives every real roster identity a unique character brief and silhouette',()=>{
  expect(Object.keys(FORUM_AGENT_PROFILES).sort()).toEqual(ROSTER.map(a=>a.id).sort());
  expect(new Set(Object.values(FORUM_AGENT_PROFILES).map(p=>p.signature)).size).toBe(ROSTER.length);
  expect(new Set(Object.values(FORUM_AGENT_PROFILES).map(p=>p.voice)).size).toBe(ROSTER.length);
});
it.each(ROSTER.map(a=>a.id))('binds %s forum armor to the existing rig within budget',async(id)=>{
  const path=`public/world/forum-characters/${id}`;
  expect(existsSync(`${path}.png`)).toBe(true);
  const f=readFileSync(`${path}.glb`);
  const gltf=await new GLTFLoader().parseAsync(f.buffer.slice(f.byteOffset,f.byteOffset+f.byteLength),'');
  const armor=decodeBlenderArmor(gltf.scene);
  let vertices=0;
  for(const geometry of Object.values(armor)) {
    vertices+=geometry.getAttribute('position').count;
    geometry.computeBoundingBox();
    expect(geometry.boundingBox!.min.y).toBeGreaterThan(-.1);
    expect(geometry.boundingBox!.max.y).toBeLessThan(2.8);
    geometry.dispose();
  }
  expect(vertices).toBeLessThan(90_000);
});

// The forum GLB ships compressed (scripts/compress-world-glb.mjs). Blender's
// raw export is ~62 MB, which GitHub warns about, bloats the Tauri bundle and
// has no business in git history. Git LFS is deliberately not used. This guard
// fails if the compression step is skipped or silently stops working, and it
// proves the file still decodes to the geometry the walk routes depend on.
const FORUM_GLB = 'public/world/solar-forum.glb';
const FORUM_GLB_BUDGET = 20 * 1024 * 1024;

function glbJson(file: Buffer): { extensionsUsed?: string[]; extensionsRequired?: string[] } {
  // GLB container: 12-byte header, then length-prefixed chunks; the first is JSON.
  const chunkLength = file.readUInt32LE(12);
  return JSON.parse(file.subarray(20, 20 + chunkLength).toString('utf8'));
}

it('ships the forum GLB meshopt-compressed and inside the runtime budget', async () => {
  const file = readFileSync(FORUM_GLB);
  expect(file.byteLength).toBeLessThan(FORUM_GLB_BUDGET);

  const json = glbJson(file);
  expect(json.extensionsRequired).toContain('EXT_meshopt_compression');
  expect(json.extensionsUsed).toContain('KHR_mesh_quantization');

  const manifest = JSON.parse(readFileSync('public/world/solar-forum.manifest.json', 'utf8'));
  expect(manifest.bytes).toBe(file.byteLength);
  expect(manifest.rawBytes).toBeGreaterThan(file.byteLength);

  const loader = withMeshopt(new GLTFLoader());
  // Node has no browser `self`/Image for the embedded JPEG textures; the
  // geometry is what this test is about. Same seam as forumWalkRoutes.test.ts.
  loader.register(() => ({ name: 'geometry-only-textures', loadTexture: () => Promise.resolve(new Texture()) }));
  const gltf = await loader.parseAsync(file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength), '');

  let meshes = 0, triangles = 0;
  const materialNames = new Set<string>();
  gltf.scene.updateMatrixWorld(true);
  gltf.scene.traverse(object => {
    if (!(object instanceof Mesh)) return;
    meshes += 1;
    const index = object.geometry.getIndex();
    triangles += (index ? index.count : object.geometry.getAttribute('position').count) / 3;
    for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
      materialNames.add(material.name);
    }
  });
  // Mesh and triangle counts catch a compression step that starts joining,
  // simplifying or flattening: walkCollision.ts raycasts this exact geometry.
  expect(meshes).toBe(manifest.meshes);
  expect(triangles).toBe(manifest.triangles);
  // Material names survive: walkCollision.ts refuses to walk onto 'Water',
  // and ForumSurfaceMaterials.ts classifies every surface by name.
  expect(materialNames.size).toBe(manifest.meshes);
  expect(materialNames).toContain('Water');
});
