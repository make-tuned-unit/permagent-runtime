import { describe, expect, it } from 'vitest';
import { Mesh, Vector3 } from 'three';
import { GARDEN_PLACES, GARDEN_ROUTE, GARDEN_START, gardenCollisionSource } from './gardenLayout';
import { attemptWalk, registerForumCollision, getForumColliders } from './walkCollision';

describe('compact garden traversal against production collision geometry', () => {
  function releaseSource(source: ReturnType<typeof gardenCollisionSource>){const materials=new Set();source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();for(const m of Array.isArray(o.material)?o.material:[o.material])materials.add(m);}});materials.forEach(m=>(m as {dispose:()=>void}).dispose());}
  function route(points: number[][]) {
    const feet = new Vector3(...points[0] as [number,number,number]);
    for (const point of points.slice(1)) {
      const goal = new Vector3(...point as [number,number,number]);
      const maxSteps=Math.ceil(feet.distanceTo(goal)/.05)+4;
      let steps=0;
      while(feet.distanceTo(goal)>.025){
        expect(++steps, `step budget from ${feet.toArray()} to ${goal.toArray()}`).toBeLessThanOrEqual(maxSteps);
        const before=feet.distanceTo(goal);
        const step=goal.clone().sub(feet);step.multiplyScalar(Math.min(.06,step.length())/step.length());
        expect(attemptWalk(feet,step.x,step.z), `blocked at ${feet.toArray()} toward ${goal.toArray()}`).toBe(true);
        expect(feet.distanceTo(goal), 'each accepted step must make positive progress').toBeLessThan(before-1e-6);
      }
    }
  }
  it('walks both directions around the entire listed landscape', () => {
    const source=gardenCollisionSource();const dispose=registerForumCollision(source);
    try {route(GARDEN_ROUTE);route([...GARDEN_ROUTE].reverse());}
    finally {dispose();releaseSource(source);}
  });
  it('reaches every destination from arrival without teleporting', () => {
    const source=gardenCollisionSource();const dispose=registerForumCollision(source);
    try {
      for(const place of GARDEN_PLACES){
        const [x,,z]=place.position;
        const destination = place.id==='commons' ? [-.2,0,3] : [x,0,z];
        const west=place.id==='gallery';
        const side=west?-6:7;
        route([GARDEN_START,[side,0,8.2],[side,0,z],destination]);
      }
    } finally {dispose();releaseSource(source);}
  });
  it('refuses water, furniture and the outer boundary',()=>{
    const source=gardenCollisionSource();const dispose=registerForumCollision(source);
    try{
      expect(getForumColliders().some(m=>m.name==='bench')).toBe(true);
      expect(getForumColliders().some(m=>m.name==='planter')).toBe(true);
      expect(getForumColliders().some(m=>m.name==='canopy')).toBe(true);
      expect(getForumColliders().some(m=>m.name==='agent body')).toBe(true);
      expect(attemptWalk(new Vector3(1.7,0,-2),.5,0)).toBe(false);
      expect(attemptWalk(new Vector3(-1.5,0,3),-.2,0)).toBe(false);
      expect(attemptWalk(new Vector3(8.3,0,3),.15,0)).toBe(false);
    }finally{dispose();releaseSource(source);}
  });
});
