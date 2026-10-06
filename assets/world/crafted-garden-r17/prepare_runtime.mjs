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
const [name]=process.argv.slice(2);
const settings={table:{dimensions:[2.24,.825,2.24],triangles:18000,texture:2048}}[name];
if(!settings)throw Error('Expected table');
const variant='hunyuan';
const source=path.join(here,'raw',`${name}.glb`);
const destination=path.join(here,'processed');
const io=new NodeIO().registerExtensions(ALL_EXTENSIONS);
const document=await io.read(source),root=document.getRoot(),scene=root.getDefaultScene()??root.listScenes()[0];
const triangles=()=>root.listMeshes().reduce((sum,m)=>sum+m.listPrimitives().reduce((n,p)=>n+(p.getIndices()?.getCount()??p.getAttribute('POSITION').getCount())/3,0),0);
// Hunyuan exports specularColorFactor=2; use standard physical PBR response.
if(variant==='hunyuan')for(const m of root.listMaterials()){m.setExtension('KHR_materials_specular',null);m.setNormalScale(name==='fern'?.45:.7);}
const before={bytes:(await fs.stat(source)).size,triangles:triangles(),bounds:getBounds(scene),textures:root.listTextures().map(t=>({size:t.getSize(),mimeType:t.getMimeType()}))};
await MeshoptSimplifier.ready;
await document.transform(weld(),simplify({simplifier:MeshoptSimplifier,ratio:Math.min(1,settings.triangles/before.triangles),error:.001,lockBorder:true}),textureCompress({encoder:sharp,targetFormat:'webp',resize:[settings.texture,settings.texture],quality:90,effort:6}),prune());
// Preserve authored node transforms below a normalized metre-sized asset root.
// The instancer owns the named root transform, so normalization is a CHILD node.
const bounds=getBounds(scene),desired=settings.dimensions;
const scale=desired.map((d,i)=>d/(bounds.max[i]-bounds.min[i]));
const offset=[-(bounds.min[0]+bounds.max[0])/2,-bounds.min[1],-(bounds.min[2]+bounds.max[2])/2];
const asset=document.createNode(name),normalized=document.createNode(`${name} metre normalization`).setScale(scale).setTranslation(offset.map((v,i)=>v*scale[i]));
for(const child of [...scene.listChildren()]){scene.removeChild(child);normalized.addChild(child);}
asset.addChild(normalized);scene.addChild(asset);
let repair=null;
await fs.mkdir(destination,{recursive:true});
const output=path.join(destination,`${name}-v1.glb`);
await io.write(output,document);
const bytes=await fs.readFile(output);
const report={name,variant,source:path.relative(here,source),output:path.relative(here,output),sha256:createHash('sha256').update(bytes).digest('hex'),before,repair,after:{bytes:bytes.length,triangles:triangles(),bounds:getBounds(scene),primitives:root.listMeshes().reduce((s,m)=>s+m.listPrimitives().length,0),textures:root.listTextures().map(t=>({size:t.getSize(),mimeType:t.getMimeType()}))},normalization:{scale,offset,dimensions:desired},review:'pending runtime and exported front/reverse inspection'};
await fs.writeFile(path.join(here,`${name}-${variant}-runtime.json`),JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify(report,null,2));
