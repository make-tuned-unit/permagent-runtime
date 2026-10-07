import { describe, expect, it } from 'vitest';
import { Mesh, Vector3 } from 'three';
import { advanceHabitatAgent, commandHabitatAgent, findHabitatPath, getHabitatTravel, initializeHabitatAgent } from './habitatMotion';
import { getForumMotion } from './ForumInhabitants';
import { HABITAT_STATIONS, HABITAT_START, ORCHESTRATOR_TABLE_SEAT, habitatCollisionSource, habitatElevation, habitatWalkable, type Point3 } from './habitatLayout';
import { attemptWalk, registerForumCollision } from './walkCollision';

describe('inhabited garden paths',()=>{
  it('walks each specialist from arrival to their actual workstation and back over production colliders',()=>{
    const source=habitatCollisionSource(),release=registerForumCollision(source);
    try{
      for(const station of [...HABITAT_STATIONS,{id:'conversation seat',position:ORCHESTRATOR_TABLE_SEAT}])for(const [start,end] of [[HABITAT_START,station.position],[station.position,HABITAT_START]]){
        expect(habitatWalkable(...[end[0],end[2]] as [number,number]),`${station.id} destination clear`).toBe(true);
        const path=findHabitatPath(start,end);expect(path,`${station.id} has a route`).not.toBeNull();const feet=new Vector3(...start);
        for(const target of path!){let budget=2000;
          while(Math.hypot(feet.x-target[0],feet.z-target[2])>.025){
            expect(--budget,`${station.id} step budget`).toBeGreaterThan(0);
            const dx=target[0]-feet.x,dz=target[2]-feet.z,d=Math.hypot(dx,dz),step=Math.min(.05,d);
            expect(attemptWalk(feet,dx/d*step,dz/d*step),`${station.id} blocked at ${feet.toArray()} toward ${target}`).toBe(true);
          }
        }
        expect(feet.distanceTo(new Vector3(...end))).toBeLessThan(.04);
      }
    }finally{release();const materials=new Set<{dispose:()=>void}>();source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();(Array.isArray(o.material)?o.material:[o.material]).forEach(m=>materials.add(m));}});materials.forEach(m=>m.dispose());}
  },60000);
  it('rejects water, occupied desks and destinations outside the bounded landscape',()=>{
    for(const to of [[4,0,0],[-3,0,3],[20,0,0],[0,0,-55]] as Point3[])expect(findHabitatPath(HABITAT_START,to)).toBeNull();
  });
  it.each([false,true])('keeps moving to the observatory around another agent (moves into route: %s)',movesIntoRoute=>{
    const source=habitatCollisionSource(),release=registerForumCollision(source);
    try{
      HABITAT_STATIONS.forEach(s=>initializeHabitatAgent(s.id));
      const destination:Point3=[2,3.4,-23];
      expect(commandHabitatAgent('henry',destination)).toBe(true);
      if(movesIntoRoute){
        const lead=getForumMotion('henry')!,target=getHabitatTravel('henry')!.path.find(p=>Math.hypot(p[0]-lead.x,p[2]-lead.z)>2)!;
        const distance=Math.hypot(target[0]-lead.x,target[2]-lead.z),council=getForumMotion('council')!;
        council.x=lead.x+(target[0]-lead.x)/distance*1.2;council.z=lead.z+(target[2]-lead.z)/distance*1.2;
        council.y=habitatElevation(council.x,council.z);
      }
      for(let frame=0;frame<2400&&getHabitatTravel('henry')?.moving;frame++){
        advanceHabitatAgent('henry',.05);
        const lead=getForumMotion('henry')!;
        for(const s of HABITAT_STATIONS.filter(s=>s.id!=='henry')){
          const other=getForumMotion(s.id)!;
          expect(Math.hypot(lead.x-other.x,lead.z-other.z),`clear of ${s.id}`).toBeGreaterThanOrEqual(.71);
        }
      }
      expect(getHabitatTravel('henry')?.label).toBe('Arrived');
      if(movesIntoRoute)expect(getHabitatTravel('henry')!.replans).toBeGreaterThan(0);
      const lead=getForumMotion('henry')!;
      expect(Math.hypot(lead.x-destination[0],lead.z-destination[2])).toBeLessThan(.08);
    }finally{
      release();const materials=new Set<{dispose:()=>void}>();
      source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();(Array.isArray(o.material)?o.material:[o.material]).forEach(m=>materials.add(m));}});materials.forEach(m=>m.dispose());
    }
  },60000);
});
