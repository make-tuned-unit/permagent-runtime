import { describe, expect, it } from 'vitest';
import { Mesh, MeshBasicMaterial, Raycaster, Vector3 } from 'three';
import { terraceHullGeometry, terraceSurfaceGeometry } from './habitatTerrace';
import { habitatHeight, onHabitat } from './habitatLayout';

describe('continuous rounded terrace',()=>{
  it('covers the whole walkable footprint at its exact height without protruding corner tiles',()=>{
    const geometry=terraceSurfaceGeometry(),material=new MeshBasicMaterial(),mesh=new Mesh(geometry,material),ray=new Raycaster();
    try{
      const positions=geometry.getAttribute('position');
      for(let i=0;i<positions.count;i++){
        const x=positions.getX(i),y=positions.getY(i),z=positions.getZ(i);
        expect(onHabitat(x,z,-.00001)).toBe(true);expect(y).toBeCloseTo(habitatHeight(z),5);
      }
      for(let z=-27.75;z<16;z+=1.5)for(let x=-15.75;x<16;x+=1.5){
        if(!onHabitat(x,z,.12))continue;
        ray.set(new Vector3(x,15,z),new Vector3(0,-1,0));const hit=ray.intersectObject(mesh)[0];
        expect(hit,`supported surface at ${x},${z}`).toBeDefined();expect(hit.point.y).toBeCloseTo(habitatHeight(z),5);
      }
    }finally{geometry.dispose();material.dispose();}
  });
  it('keeps the architectural hull beneath the accessible garden',()=>{
    const geometry=terraceHullGeometry();
    try{
      const positions=geometry.getAttribute('position');
      for(let i=0;i<positions.count;i++){
        const x=positions.getX(i),y=positions.getY(i),z=positions.getZ(i);
        expect([x,y,z].every(Number.isFinite)).toBe(true);expect(onHabitat(x,z,-.00001)).toBe(true);
        expect(y).toBeLessThanOrEqual(habitatHeight(z)+.00001);
      }
    }finally{geometry.dispose();}
  });
});
