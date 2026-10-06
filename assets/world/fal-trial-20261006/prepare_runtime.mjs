// Run from any directory. Uses the lane's locked local dependencies; no download.
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { NodeIO } from '../../../ui/command-center/node_modules/@gltf-transform/core/dist/index.js';
import { ALL_EXTENSIONS } from '../../../ui/command-center/node_modules/@gltf-transform/extensions/dist/index.js';
import { getBounds, weld, simplify, textureCompress, prune } from '../../../ui/command-center/node_modules/@gltf-transform/functions/dist/index.js';
import { MeshoptSimplifier } from '../../../ui/command-center/node_modules/meshoptimizer/index.js';
const require = createRequire(new URL('../../../ui/command-center/package.json', import.meta.url));
const sharp = require('sharp');

const here=path.dirname(fileURLToPath(import.meta.url));
const [name,variant='hunyuan']=process.argv.slice(2);
if(!['planter','instrument'].includes(name)||!['hunyuan','trellis'].includes(variant)) throw Error('Expected planter|instrument hunyuan|trellis');
const source=path.join(here,'raw',`${name}-${variant}.glb`);
const destination=path.join(here,'processed');
const io=new NodeIO().registerExtensions(ALL_EXTENSIONS);
const document=await io.read(source),root=document.getRoot(),scene=root.getDefaultScene()??root.listScenes()[0];
const triangles=()=>root.listMeshes().reduce((sum,m)=>sum+m.listPrimitives().reduce((n,p)=>n+(p.getIndices()?.getCount()??p.getAttribute('POSITION').getCount())/3,0),0);
const before={bytes:(await fs.stat(source)).size,triangles:triangles(),bounds:getBounds(scene),textures:root.listTextures().map(t=>({size:t.getSize(),mimeType:t.getMimeType()}))};
await MeshoptSimplifier.ready;
await document.transform(weld(),simplify({simplifier:MeshoptSimplifier,ratio:Math.min(1,(name==='planter'?12000:18000)/before.triangles),error:.001,lockBorder:true}),textureCompress({encoder:sharp,targetFormat:'webp',resize:[1024,1024],quality:90,effort:6}),prune());
// Preserve authored node transforms below a normalized metre-sized asset root.
// The instancer owns the named root transform, so normalization is a CHILD node.
const bounds=getBounds(scene),desired=name==='planter'?[1.08,.665,1.08]:[.80,.64,.68];
const scale=desired.map((d,i)=>d/(bounds.max[i]-bounds.min[i]));
const offset=[-(bounds.min[0]+bounds.max[0])/2,-bounds.min[1],-(bounds.min[2]+bounds.max[2])/2];
const asset=document.createNode(name),normalized=document.createNode(`${name} metre normalization`).setScale(scale).setTranslation(offset.map((v,i)=>v*scale[i]));
for(const child of [...scene.listChildren()]){scene.removeChild(child);normalized.addChild(child);}
asset.addChild(normalized);scene.addChild(asset);
let repair=null;
if(name==='planter'&&variant==='trellis') {
  // This export is axis-aligned. Keep its stone/rim; replace the disconnected
  // triangles in the soil region with a small level soil cap.
  const removed=[];
  for(const mesh of root.listMeshes()) for(const p of mesh.listPrimitives()) {
    const pos=p.getAttribute('POSITION'),indices=p.getIndices(),kept=[];
    if(!indices)throw Error('Planter repair requires indexed geometry');
    for(let i=0;i<indices.getCount();i+=3){
      const ids=[0,1,2].map(n=>indices.getScalar(i+n));
      const points=ids.map(id=>pos.getElement(id,[]).map((v,a)=>(v+offset[a])*scale[a]));
      const c=[0,1,2].map(a=>points.reduce((sum,v)=>sum+v[a],0)/3);
      if(Math.hypot(c[0],c[2])<.465&&c[1]>.46&&c[1]<.607)removed.push(i/3);
      else kept.push(...ids);
    }
    p.setIndices(document.createAccessor().setType('SCALAR').setArray(new Uint32Array(kept)).setBuffer(root.listBuffers()[0]));
  }
  const positions=[],normals=[],colors=[],indices=[],radius=.491,height=.595,segments=96;
  positions.push(0,height,0);normals.push(0,1,0);colors.push(.075,.041,.021);
  for(let i=0;i<segments;i++) {const a=i/segments*Math.PI*2;positions.push(Math.cos(a)*radius,height,Math.sin(a)*radius);normals.push(0,1,0);const shade=1;colors.push(.075*shade,.041*shade,.021*shade);}
  for(let i=0;i<segments;i++)indices.push(0,(i+1)%segments+1,i+1);
  const accessor=(type,array)=>document.createAccessor().setType(type).setArray(array).setBuffer(root.listBuffers()[0]);
  const soilMaterial=document.createMaterial('Level dark soil').setBaseColorFactor([1,1,1,1]).setRoughnessFactor(1).setMetallicFactor(0).setDoubleSided(true);
  const soil=document.createPrimitive().setAttribute('POSITION',accessor('VEC3',new Float32Array(positions))).setAttribute('NORMAL',accessor('VEC3',new Float32Array(normals))).setAttribute('COLOR_0',accessor('VEC3',new Float32Array(colors))).setIndices(accessor('SCALAR',new Uint16Array(indices))).setMaterial(soilMaterial);
  asset.addChild(document.createNode('Repaired level soil').setMesh(document.createMesh().addPrimitive(soil)));
  await document.transform(weld(),prune());
  repair={removedTriangles:removed.length,soilHeight:height,soilRadius:radius,addedTriangles:segments};
}
await fs.mkdir(destination,{recursive:true});
const output=path.join(destination,`${name}-v1.glb`);
await io.write(output,document);
const bytes=await fs.readFile(output);
const report={name,variant,source:path.relative(here,source),output:path.relative(here,output),sha256:createHash('sha256').update(bytes).digest('hex'),before,repair,after:{bytes:bytes.length,triangles:triangles(),bounds:getBounds(scene),primitives:root.listMeshes().reduce((s,m)=>s+m.listPrimitives().length,0),textures:root.listTextures().map(t=>({size:t.getSize(),mimeType:t.getMimeType()}))},normalization:{scale,offset,dimensions:desired},review:'pending runtime and exported front/reverse inspection'};
await fs.writeFile(path.join(here,`${name}-${variant}-runtime.json`),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify(report,null,2));
