import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { Box3, Mesh, MeshStandardMaterial, Raycaster, Vector3, SRGBColorSpace } from 'three';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { instanceGardenDecor } from './staticGardenInstances';
import { ARCHIVE_DECOR, COMMONS_SURFACE_Y, HABITAT_DECOR, habitatCollisionSource, habitatWalkable } from './habitatLayout';

async function loadAsset(file:string) {
  const bytes=readFileSync(file),oldBitmap=globalThis.createImageBitmap,oldSelf=globalThis.self;
  globalThis.self=globalThis as unknown as Window & typeof globalThis;
  const sharp=(await import('sharp')).default;
  globalThis.createImageBitmap=async(source:ImageBitmapSource)=>{
    if(!(source instanceof Blob))throw Error('Expected embedded texture');
    const decoded=await sharp(Buffer.from(await source.arrayBuffer())).ensureAlpha().raw().toBuffer({resolveWithObject:true});
    return {width:decoded.info.width,height:decoded.info.height,data:decoded.data,close(){}} as unknown as ImageBitmap;
  };
  try{return {bytes,gltf:await new GLTFLoader().parseAsync(bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength),'')};}
  finally{globalThis.createImageBitmap=oldBitmap;globalThis.self=oldSelf;}
}
function disposeScene(root:import('three').Object3D){
  const textures=new Set<import('three').Texture>();
  root.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();for(const m of (Array.isArray(o.material)?o.material:[o.material]) as MeshStandardMaterial[]){for(const t of [m.map,m.normalMap,m.roughnessMap,m.metalnessMap])if(t)textures.add(t);m.dispose();}}});
  textures.forEach(t=>t.dispose());
}
const candidates=[
  {name:'table',file:'table-v1.glb',budget:18000,placements:1,limits:[2.25,.826,2.25]},
  {name:'planter',file:'planter-v2.glb',budget:12000,placements:11,limits:[1.1,.67,1.1]},
  {name:'instrument',file:'instrument-v1.glb',budget:18000,placements:1,limits:[.81,.65,.69]},
  {name:'bench',file:'bench-v1.glb',budget:18000,placements:5,limits:[1.62,1.015,.55]},
  {name:'bookcase',file:'bookcase-v1.glb',budget:18000,placements:3,limits:[2.6,2.81,.54]},
  {name:'column',file:'column-v1.glb',budget:12000,placements:2,limits:[.541,3.61,.541]},
];
describe('crafted garden exports',()=>{
  it.each(candidates)('$name loads embedded PBR, fits its footprint and batches its real geometry',async candidate=>{
    const {bytes,gltf}=await loadAsset(`public/world/garden-crafted/${candidate.file}`);
    try{
      const highResolution=['bookcase','table'].includes(candidate.name);
      expect(bytes.length).toBeLessThan(highResolution?2_000_000:1_200_000);
      const root=gltf.scene.getObjectByName(candidate.name);expect(root).toBeDefined();
      gltf.scene.updateMatrixWorld(true);
      const bounds=new Box3().setFromObject(root!),size=bounds.getSize(new Vector3());
      expect(bounds.min.y).toBeCloseTo(0,5);
      for(let axis=0;axis<3;axis++)expect(size.getComponent(axis)).toBeLessThan(candidate.limits[axis]);
      let triangles=0,meshes=0;
      root!.traverse(o=>{
        if(!(o instanceof Mesh))return;meshes++;
        const positions=o.geometry.getAttribute('position');triangles+=(o.geometry.index?.count??positions.count)/3;
        for(let i=0;i<positions.count;i++)expect(new Vector3().fromBufferAttribute(positions,i).toArray().every(Number.isFinite)).toBe(true);
        const material=o.material as MeshStandardMaterial;
        expect(material.map).not.toBeNull();expect(material.map!.colorSpace).toBe(SRGBColorSpace);
        expect(material.normalMap).not.toBeNull();expect(material.roughnessMap).not.toBeNull();expect(material.metalnessMap).not.toBeNull();
        for(const texture of [material.map!,material.normalMap!,material.roughnessMap!]){
          const resolution=highResolution?2048:1024;
          expect(texture.image).toHaveProperty('width',resolution);expect(texture.image).toHaveProperty('height',resolution);
        }
        expect(o.geometry.getAttribute('uv')).toBeDefined();
      });
      expect(meshes).toBe(1);expect(triangles).toBeLessThanOrEqual(candidate.budget);
      const placements=[...HABITAT_DECOR,...ARCHIVE_DECOR].filter(p=>p.asset===candidate.name);expect(placements).toHaveLength(candidate.placements);
      if(['bench','bookcase','column','table'].includes(candidate.name)){
        const colliders=habitatCollisionSource();
        try{for(const p of placements){
          const clone=root!.clone(true);clone.position.set(...p.position);clone.scale.setScalar(p.scale);clone.rotation.y=p.heading;clone.updateMatrixWorld(true);
          const actual=new Box3().setFromObject(clone);
          const name=candidate.name==='bookcase'?'archive shelves':candidate.name==='column'?'archive pillar':candidate.name==='table'?'commons table':'bench';
          const collider=colliders.children.find(o=>o.name===name&&Math.abs(o.position.x-p.position[0])<.001&&Math.abs(o.position.z-p.position[2])<.001)!;
          expect(new Box3().setFromObject(collider).expandByScalar(.00001).containsBox(actual),`${name} collider contains actual mesh at ${p.position}`).toBe(true);
        }
        // Near the end of the sideways commons bench: navigation must see it.
        expect(habitatWalkable(-5.3,2.15,0)).toBe(false);
        }finally{disposeScene(colliders);}
      }
      const result=instanceGardenDecor(gltf.scene,placements);
      try{expect(result.census.primitiveBatches).toBe(1);expect(result.census.triangles).toBe(triangles*candidate.placements);}
      finally{result.dispose();}
      if(candidate.name==='table'){
        const table=root!.clone(true),p=placements[0];table.position.set(...p.position);table.scale.setScalar(p.scale);table.updateMatrixWorld(true);
        const ray=new Raycaster();
        for(const [x,z]of [[-3,3],[-3.82,2.95],[-3.7,3.4],[-2.5,2.8],[-3.2,3.7]]){
          ray.set(new Vector3(x,2,z),new Vector3(0,-1,0));const hit=ray.intersectObject(table)[0];
          expect(hit,`table supports prop at ${x},${z}`).toBeDefined();expect(Math.abs(hit.point.y-COMMONS_SURFACE_Y)).toBeLessThan(.003);
        }
      }
      if(candidate.name==='instrument'){
        const {gltf:kit}=await loadAsset('public/world/garden-crafted/table-v1.glb');
        try{
          const table=HABITAT_DECOR.find(p=>p.asset==='table')!;
          const top=new Box3().setFromObject(kit.scene.getObjectByName('table')!).max.y*table.scale+table.position[1];
          const base=placements[0].position[1]+bounds.min.y*placements[0].scale;
          expect(base-top).toBeGreaterThanOrEqual(-.002);expect(base-top).toBeLessThan(.008);
        }finally{disposeScene(kit.scene);}
      }
    }finally{disposeScene(gltf.scene);}
  });
});
