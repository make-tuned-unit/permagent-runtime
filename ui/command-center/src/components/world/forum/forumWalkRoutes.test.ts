import { readFileSync } from 'node:fs';
import { beforeAll, afterAll, expect, it } from 'vitest';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { Texture, Vector3 } from 'three';
import { registerForumCollision, attemptWalk } from './walkCollision';
import { withMeshopt } from './forumGltf';
let dispose:()=>void;
beforeAll(async()=>{
  const f=readFileSync('public/world/solar-forum.glb');
  const loader = withMeshopt(new GLTFLoader());
  // Collision verification needs real mesh/index/transform data, but Node has
  // no browser `self`/Image implementation for embedded GLB textures. Supply
  // a neutral texture through the loader plugin seam; browser visual QA owns
  // texture/shader validation.
  loader.register(() => ({ name: 'geometry-only-textures', loadTexture: () => Promise.resolve(new Texture()) }));
  const gltf=await loader.parseAsync(f.buffer.slice(f.byteOffset,f.byteOffset+f.byteLength),'');
  dispose=registerForumCollision(gltf.scene);
});
afterAll(()=>dispose?.());
it('walks the exported bridge from the commons into the arrival garden',()=>{
  const feet=new Vector3(0,0,14);
  for(let i=0;i<270;i++) {
    if(!attemptWalk(feet,0,.15))break;
  }
  expect(feet.z).toBeGreaterThan(54);
});
it('climbs the exported right-hand gallery stair to the upper level',()=>{
  const feet=new Vector3(20.25,0,8.4);
  for(let i=0;i<60;i++) {
    if(!attemptWalk(feet,0,-.15))break;
  }
  expect(feet.z).toBeLessThan(0);
  expect(feet.y).toBeCloseTo(4.32,1);
});

it('climbs the spiral and steps onto the offset observatory deck',()=>{
  const feet=new Vector3(-11.1,4.46,-20);
  for(let i=1;i<=270;i++) {
    const angle=Math.PI+i*(Math.PI*2/28*27)/270;
    const x=-10+1.1*Math.cos(angle), z=-20-1.1*Math.sin(angle);
    if(!attemptWalk(feet,x-feet.x,z-feet.z))break;
  }
  expect(feet.y).toBeGreaterThan(8);
  for(let i=0;i<10;i++) {
    if(!attemptWalk(feet,-.08,0))break;
  }
  expect(feet.x).toBeLessThan(-11.5);
  expect(feet.y).toBeCloseTo(8.26,1);
});

import patrols from './patrolRoutes.json';
it.each(Object.entries(patrols.routes))('grounds the complete %s inhabitant route on exported geometry',(_name,route)=>{
  const feet=new Vector3(...route[0] as [number,number,number]);
  for(let index=1;index<=route.length;index++){
    const target=route[index%route.length];
    while(Math.hypot(target[0]-feet.x,target[2]-feet.z)>.02){
      const dx=target[0]-feet.x,dz=target[2]-feet.z,d=Math.hypot(dx,dz),step=Math.min(.12,d);
      const before=feet.toArray();
      expect(attemptWalk(feet,dx/d*step,dz/d*step),`${_name} segment ${index}: ${before} → ${target}`).toBe(true);
    }
    expect(feet.y).toBeCloseTo(target[1],1);
  }
});
