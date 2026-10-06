import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { Box3, Mesh, MeshStandardMaterial, Vector3, SRGBColorSpace } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { instanceGardenDecor } from './staticGardenInstances';
import { HABITAT_DECOR } from './habitatLayout';

describe('crafted garden planter export', () => {
  it('loads locally, fits every existing collision proxy and batches the repaired mesh', async () => {
    const bytes = readFileSync('public/world/garden-crafted/planter-v1.glb');
    expect(bytes.length).toBeLessThan(1_200_000);
    const oldBitmap = globalThis.createImageBitmap, oldSelf = globalThis.self;
    globalThis.self = globalThis as unknown as Window & typeof globalThis;
    const sharp = (await import('sharp')).default;
    globalThis.createImageBitmap = async (source: ImageBitmapSource) => {
      if (!(source instanceof Blob)) throw Error('Expected embedded texture');
      const decoded = await sharp(Buffer.from(await source.arrayBuffer())).ensureAlpha().raw().toBuffer({resolveWithObject:true});
      return {width:decoded.info.width,height:decoded.info.height,data:decoded.data,close(){}} as unknown as ImageBitmap;
    };
    let gltf;
    try { gltf = await new GLTFLoader().parseAsync(bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),''); }
    finally { globalThis.createImageBitmap = oldBitmap; globalThis.self = oldSelf; }
    const root = gltf.scene.getObjectByName('planter');
    expect(root).toBeDefined();
    gltf.scene.updateMatrixWorld(true);
    const bounds = new Box3().setFromObject(root!);
    expect(bounds.min.y).toBeCloseTo(0,6);
    expect(bounds.max.y).toBeLessThanOrEqual(.67);
    let triangles=0,maxRadius=0,texturedMeshes=0;
    root!.traverse(object => {
      if (!(object instanceof Mesh)) return;
      const positions=object.geometry.getAttribute('position');
      triangles+=(object.geometry.index?.count??positions.count)/3;
      for(let i=0;i<positions.count;i++){
        const p=new Vector3().fromBufferAttribute(positions,i).applyMatrix4(object.matrixWorld);
        expect(p.toArray().every(Number.isFinite)).toBe(true);
        maxRadius=Math.max(maxRadius,Math.hypot(p.x,p.z));
      }
      const material=object.material as MeshStandardMaterial;
      if(material.map){
        texturedMeshes++;
        expect(material.map.colorSpace).toBe(SRGBColorSpace);
        expect(material.map.image).toHaveProperty('width',1024);
        expect(material.map.image).toHaveProperty('height',1024);
        expect(material.roughnessMap).toBeDefined();
        expect(object.geometry.getAttribute('uv')).toBeDefined();
      }
    });
    expect(texturedMeshes).toBe(1);
    expect(triangles).toBeLessThan(7000);
    const placements=HABITAT_DECOR.filter(p=>p.asset==='planter');
    expect(placements.length).toBe(11);
    // Keep a conservative radial envelope inside the existing square footprint.
    for(const p of placements) expect(maxRadius).toBeLessThanOrEqual(p.solid![0]/2*Math.cos(Math.PI/24));
    const result=instanceGardenDecor(gltf.scene,placements);
    try{
      expect(result.census.primitiveBatches).toBe(2);
      expect(result.census.placements).toBe(11);
      expect(result.census.triangles).toBe(triangles*11);
      expect(result.group.children.every(o=>o.userData.asset==='planter')).toBe(true);
    }finally{result.dispose();}
    root!.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();const m=o.material as MeshStandardMaterial;m.map?.dispose();m.roughnessMap?.dispose();m.dispose();}});
  });
});
