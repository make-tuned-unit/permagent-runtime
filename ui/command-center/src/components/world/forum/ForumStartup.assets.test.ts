/** @vitest-environment jsdom */
import { readFileSync, writeFileSync } from 'node:fs';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { Mesh } from 'three';
import { expect, it } from 'vitest';
import { withMeshopt } from './forumGltf';

it('records real shipped GLB geometry parsing and an asynchronous CPU image decode surrogate',async()=>{
  const bytes=readFileSync('public/world/garden-hero/hero-kit-v3b.glb');
  const jsonSize=bytes.readUInt32LE(12),start=performance.now();
  const json=JSON.parse(bytes.subarray(20,20+jsonSize).toString());
  const jsonMs=performance.now()-start;
  const bin=bytes.subarray(28+jsonSize);
  const image=json.images?.find((image:{bufferView?:number})=>image.bufferView!==undefined);
  let decodeMs:number|null=null;
  if(image){
    const sharp=(await import('sharp')).default,view=json.bufferViews[image.bufferView];
    const imageBytes=bin.subarray(view.byteOffset??0,(view.byteOffset??0)+view.byteLength);
    const start=performance.now();await sharp(imageBytes).ensureAlpha().raw().toBuffer();decodeMs=performance.now()-start;
  }
  // Keep the actual geometry/nodes; omit materials so jsdom does not pretend
  // to have a browser image decoder. The image measurement above is labelled
  // explicitly as Sharp, not native WebKit or the new bitmap loader.
  for(const mesh of json.meshes)for(const primitive of mesh.primitives)delete primitive.material;
  const text=Buffer.from(JSON.stringify(json)),padded=Buffer.alloc(Math.ceil(text.length/4)*4,32);text.copy(padded);
  const tail=bytes.subarray(20+jsonSize),header=Buffer.alloc(20);bytes.copy(header,0,0,12);
  header.writeUInt32LE(20+padded.length+tail.length,8);header.writeUInt32LE(padded.length,12);header.writeUInt32LE(0x4e4f534a,16);
  const fixture=Buffer.concat([header,padded,tail]),loader=withMeshopt(new GLTFLoader());
  const payload=new ArrayBuffer(fixture.byteLength);new Uint8Array(payload).set(fixture);
  const parseStart=performance.now(),pending=loader.parseAsync(payload,'');
  const dispatchMs=performance.now()-parseStart,gltf=await pending,parseWallMs=performance.now()-parseStart;
  const report={scope:'supplementary unchanged shipped GLB parser; geometry real, GLTF materials omitted; Sharp image decode surrogate',bytes:bytes.length,jsonParseSyncMs:jsonMs,parseDispatchSyncMs:dispatchMs,parseAsyncWallMs:parseWallMs,sharpImageDecodeAsyncMs:decodeMs,meshoptCompressed:json.extensionsUsed?.includes('EXT_meshopt_compression')??false};
  console.info('G5 asset trace',JSON.stringify(report));
  if(process.env.L2_G5_TRACE)writeFileSync('../../reports/world-production/l2-g5-assets.json',JSON.stringify(report,null,2)+'\n');
  expect(gltf.scene.children.length).toBeGreaterThan(0);
  gltf.scene.traverse(object=>{if(object instanceof Mesh){object.geometry.dispose();for(const material of Array.isArray(object.material)?object.material:[object.material])material.dispose();}});
});
