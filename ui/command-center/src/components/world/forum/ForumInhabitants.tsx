import { useEffect, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { Vector3 } from 'three';
import type { MotionState } from '../agents/motion';
import { ROSTER } from '../agents/roster';
import { getReduceMotion } from '../../../styles/tokens';
import { getAgentRuntimeStates } from '../shared/agentStatus';
import type { AgentRuntimeState } from '../shared/agentStatus';
import { attemptWalk, getForumColliders } from './walkCollision';
import routes from './patrolRoutes.json';
import { patrolTarget } from './patrolMotion';

export type ForumMotionState = Pick<MotionState, 'x' | 'y' | 'z' | 'heading' | 'walking' | 'strideDistance' | 'engaged'>;

const forumMotion = new Map<string, ForumMotionState>();

/** The forum's ambient layer owns these records; live worker motion stays in motion.ts. */
export function getForumMotion(id: string): ForumMotionState | undefined {
  return forumMotion.get(id);
}

export function ensureForumMotion(id: string, home: { x: number; y: number; z: number }): ForumMotionState {
  let motion = forumMotion.get(id);
  if (!motion) {
    motion = { x: home.x, y: home.y, z: home.z, heading: 0, walking: false, strideDistance: 0, engaged: 'none' };
    forumMotion.set(id, motion);
  }
  return motion;
}

/** Ambient inhabitants use authored routes and actual collision, never job progress. */
export interface ForumPerson {
  id:string;route:number[][];motion:ForumMotionState;next:number;direction:number;pause:number;blocked:number;feet:Vector3;speed:number;
}

export function advanceForumInhabitants(
  people: ForumPerson[], rawDt: number, attended: string | null,
  reduced: boolean, states: AgentRuntimeState[], collidersAvailable = true,
): void {
  if(!collidersAvailable)return;
  const dt=Math.min(rawDt,.05);
  for(const p of people){
    const m=p.motion;m.walking=false;
    const hud=states.find(a=>a.id===p.id)?.hudState;
    if(reduced||attended===p.id||hud==='working')continue;
    if(p.pause>0){p.pause-=dt;continue;}
    const target=patrolTarget(p,m);
    if(!target)continue;
    const {dx,dz,distance}=target;
    const yaw=Math.atan2(dx,dz),delta=Math.atan2(Math.sin(yaw-m.heading),Math.cos(yaw-m.heading));
    m.heading+=delta*(1-Math.exp(-dt*4));
    const step=Math.min(distance,p.speed*dt)*Math.max(.15,Math.cos(delta));
    const nx=dx/distance*step,nz=dz/distance*step;
    if(people.some(q=>q!==p&&Math.abs(q.feet.y-m.y)<1&&Math.hypot(q.feet.x-m.x-nx,q.feet.z-m.z-nz)<.8)){p.pause=.4;continue;}
    p.feet.set(m.x,m.y,m.z);
    if(attemptWalk(p.feet,nx,nz)){
      m.x=p.feet.x;m.y=p.feet.y;m.z=p.feet.z;m.walking=true;m.strideDistance=(m.strideDistance??0)+step;p.blocked=0;
    }else if((p.blocked+=dt)>1){
      p.direction*=-1;p.next=(p.next+p.direction+p.route.length)%p.route.length;p.pause=1;p.blocked=0;
    }
  }
}

export function ForumInhabitants({ attended }: { attended: string | null }) {
  const people=useRef<ForumPerson[]>([]);
  useEffect(()=>{
    people.current=ROSTER.map((a,i)=>{
      const route=routes.routes[routes.agents[a.id as keyof typeof routes.agents] as keyof typeof routes.routes];
      const index=Math.floor((i*.381966%1)*route.length),[x,y,z]=route[index];
      const motion=ensureForumMotion(a.id,{x,y,z});
      Object.assign(motion,{x,y,z,walking:false,heading:0,strideDistance:0,engaged:'none'});
      return {id:a.id,route,motion,next:(index+1)%route.length,direction:1,pause:i*.45,blocked:0,feet:new Vector3(x,y,z),speed:.78+(i%4)*.075};
    });
    return ()=>{ people.current=[]; };
  },[]);
  useFrame((_,rawDt)=>{
    advanceForumInhabitants(people.current, rawDt, attended, getReduceMotion(), getAgentRuntimeStates(), getForumColliders().length>0);
  },-2);
  return null;
}
