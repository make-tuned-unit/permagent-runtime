// Export the actual production geometry for an offline structural review.
// This does not stand in for native app/material/performance acceptance.
import fs from 'node:fs/promises';
import {createRequire} from 'node:module';
import {NodeIO,Document} from '../../../ui/command-center/node_modules/@gltf-transform/core/dist/index.js';
const require=createRequire(new URL('../../../ui/command-center/package.json',import.meta.url));
const esbuild=require('esbuild');
const bundle=await esbuild.build({entryPoints:[new URL('../../../ui/command-center/src/components/world/forum/habitatTerrace.ts',import.meta.url).pathname],bundle:true,platform:'node',format:'esm',write:false,minify:true});
const geometry=await import('data:text/javascript;base64,'+Buffer.from(bundle.outputFiles[0].text).toString('base64'));
const doc=new Document(),buffer=doc.createBuffer(),scene=doc.createScene('Production terrace geometry');
for(const [name,mesh,color] of [['Continuous hull',geometry.terraceHullGeometry(),[.66,.62,.53,1]],['Rounded walking surface',geometry.terraceSurfaceGeometry(),[.84,.78,.66,1]]]){
  const primitive=doc.createPrimitive();
  for(const [attribute,semantic,type] of [['position','POSITION','VEC3'],['normal','NORMAL','VEC3'],['uv','TEXCOORD_0','VEC2']])primitive.setAttribute(semantic,doc.createAccessor().setType(type).setArray(mesh.getAttribute(attribute).array).setBuffer(buffer));
  primitive.setIndices(doc.createAccessor().setType('SCALAR').setArray(mesh.index.array).setBuffer(buffer));
  primitive.setMaterial(doc.createMaterial(name).setBaseColorFactor(color).setRoughnessFactor(.8));
  scene.addChild(doc.createNode(name).setMesh(doc.createMesh(name).addPrimitive(primitive)));mesh.dispose();
}
await new NodeIO().write(new URL('processed/terrace-r16.glb',import.meta.url).pathname,doc);
await fs.writeFile(new URL('terrace-preview-note.txt',import.meta.url),'Actual habitatTerrace.ts geometry exported directly; neutral diagnostic materials. Does not include native lights, surface shader or UI.\n');
console.log('Exported actual production terrace geometry.');
