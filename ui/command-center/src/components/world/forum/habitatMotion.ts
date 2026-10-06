import { useSyncExternalStore } from 'react';
import { Vector3 } from 'three';
import { ensureForumMotion, getForumMotion } from './ForumInhabitants';
import { habitatElevation, habitatWalkable, HABITAT_STATIONS, stationFor, type Point3 } from './habitatLayout';
import { attemptWalk, getForumStaticColliders } from './walkCollision';

const STEP=.5;
const key=(x:number,z:number)=>`${x},${z}`;
export function clearHabitatSegment(a:Point3,b:Point3,occupied:readonly Point3[]=[]) {
  const dx=b[0]-a[0],dz=b[2]-a[2],lengthSq=dx*dx+dz*dz;
  for(const other of occupied){
    const t=lengthSq?Math.max(0,Math.min(1,((other[0]-a[0])*dx+(other[2]-a[2])*dz)/lengthSq)):0;
    // A stopped actor may be inside the planning margin already. Permit a
    // route that moves away, while never cutting closer to that neighbour.
    const clearance=Math.min(.84,Math.hypot(a[0]-other[0],a[2]-other[2]));
    if(Math.hypot(a[0]+dx*t-other[0],a[2]+dz*t-other[2])<clearance-1e-6)return false;
  }
  const n=Math.max(1,Math.ceil(Math.hypot(a[0]-b[0],a[2]-b[2])/.15));
  for(let i=0;i<=n;i++)if(!habitatWalkable(a[0]+(b[0]-a[0])*i/n,a[2]+(b[2]-a[2])*i/n))return false;
  return true;
}
/** A* over the authored solid footprints, with swept-segment shortcutting. */
export function findHabitatPath(from:Point3,to:Point3,occupied:readonly Point3[]=[]):Point3[] | null {
  if(!habitatWalkable(...[to[0],to[2]] as [number,number]))return null;
  if(occupied.some(p=>Math.hypot(p[0]-to[0],p[2]-to[2])<.84))return null;
  const clear=(a:Point3,b:Point3)=>clearHabitatSegment(a,b,occupied);
  if(clear(from,to))return [[to[0],habitatElevation(to[0],to[2]),to[2]]];
  const nearest=(p:Point3)=>{
    const x=Math.round(p[0]/STEP),z=Math.round(p[2]/STEP);
    const candidates=[];
    for(let dx=-2;dx<=2;dx++)for(let dz=-2;dz<=2;dz++){
      const q:[number,number,number]=[(x+dx)*STEP,0,(z+dz)*STEP];q[1]=habitatElevation(q[0],q[2]);
      if(clear(p,q))candidates.push({x:x+dx,z:z+dz,d:Math.hypot(q[0]-p[0],q[2]-p[2])});
    }
    return candidates.sort((a,b)=>a.d-b.d)[0];
  };
  const start=nearest(from),end=nearest(to);if(!start||!end)return null;
  type Node={x:number;z:number;g:number;f:number;parent?:Node};
  const first:Node={...start,g:0,f:0},open=[first],best=new Map([[key(start.x,start.z),0]]),closed=new Set<string>();
  while(open.length){
    let lowest=0;for(let i=1;i<open.length;i++)if(open[i].f<open[lowest].f)lowest=i;
    const n=open.splice(lowest,1)[0],k=key(n.x,n.z);if(closed.has(k))continue;closed.add(k);
    if(n.x===end.x&&n.z===end.z){
      const points:Point3[]=[[to[0],habitatElevation(to[0],to[2]),to[2]]];let cursor:Node|undefined=n;
      while(cursor){points.unshift([cursor.x*STEP,habitatElevation(cursor.x*STEP,cursor.z*STEP),cursor.z*STEP]);cursor=cursor.parent;}
      const result:Point3[]=[];let anchor=from,i=0;
      while(i<points.length){let furthest=i;while(furthest+1<points.length&&clear(anchor,points[furthest+1]))furthest++;result.push(points[furthest]);anchor=points[furthest];i=furthest+1;}
      return result;
    }
    for(let dx=-1;dx<=1;dx++)for(let dz=-1;dz<=1;dz++){
      if(!dx&&!dz)continue;const x=n.x+dx,z=n.z+dz,nk=key(x,z),g=n.g+Math.hypot(dx,dz);
      if(closed.has(nk)||g>=(best.get(nk)??Infinity))continue;
      if(!clear([n.x*STEP,0,n.z*STEP],[x*STEP,0,z*STEP]))continue;
      best.set(nk,g);open.push({x,z,g,f:g+Math.hypot(x-end.x,z-end.z),parent:n});
    }
  }
  return null;
}
export type TravelState={id:string;label:string;moving:boolean;path:Point3[];destination:Point3|null;blocked:number;replans:number;workArrival:boolean};
const travel=new Map<string,TravelState>();
const listeners=new Set<()=>void>();let revision=0;
const publish=()=>{revision++;listeners.forEach(fn=>fn());};
export function initializeHabitatAgent(id:string) {
  const s=stationFor(id),[x,y,z]=s.position,m=ensureForumMotion(id,{x,y,z});
  Object.assign(m,{x,y,z,heading:s.heading,walking:false,strideDistance:0,engaged:'none'});
  travel.set(id,{id,label:'At workstation',moving:false,path:[],destination:null,blocked:0,replans:0,workArrival:false});
  return m;
}
export function getHabitatTravel(id:string){return travel.get(id);}
export function useHabitatTravel(id:string){useSyncExternalStore(fn=>{listeners.add(fn);return()=>listeners.delete(fn);},()=>revision);return travel.get(id);}
function agentObstacles(id:string){
  const points:Point3[]=[];
  for(const s of HABITAT_STATIONS){if(s.id===id)continue;const m=getForumMotion(s.id);if(m)points.push([m.x,m.y,m.z]);}
  return points;
}
export function commandHabitatAgent(id:string,to:Point3,label='Walking to destination',workArrival=false) {
  const m=getForumMotion(id);if(!m)return false;
  const path=findHabitatPath([m.x,m.y,m.z],to,agentObstacles(id));
  const next:TravelState={id,label:path?label:'No clear route — choose another place',moving:!!path,path:path??[],destination:path?to:null,blocked:0,replans:0,workArrival};
  travel.set(id,next);publish();return !!path;
}
export function stopHabitatAgent(id:string){const t=travel.get(id),m=getForumMotion(id);if(t){t.path=[];t.moving=false;t.label='Movement paused';}if(m)m.walking=false;publish();}
const feet=new Vector3();
export function advanceHabitatAgent(id:string,rawDt:number) {
  const t=travel.get(id),m=getForumMotion(id);if(!m||!t)return;
  m.walking=false;const target=t.path[0];if(!target)return;
  const dt=Math.min(rawDt,.05),dx=target[0]-m.x,dz=target[2]-m.z,d=Math.hypot(dx,dz);
  if(d<.075){
    t.path.shift();if(!t.path.length){t.moving=false;t.label=t.workArrival?'At workstation':'Arrived';if(t.workArrival)m.heading=stationFor(id).heading;publish();}return;
  }
  const distance=Math.min(d,stationFor(id).pace*dt),nx=dx/d*distance,nz=dz/d*distance;
  const occupied=HABITAT_STATIONS.some(s=>{if(s.id===id)return false;const other=getForumMotion(s.id);return other&&Math.hypot(other.x-m.x-nx,other.z-m.z-nz)<.72;});
  feet.set(m.x,m.y,m.z);
  if(!occupied&&attemptWalk(feet,nx,nz,getForumStaticColliders())){
    m.x=feet.x;m.y=feet.y;m.z=feet.z;m.walking=true;m.strideDistance=(m.strideDistance??0)+distance;
    const yaw=Math.atan2(dx,dz);m.heading+=Math.atan2(Math.sin(yaw-m.heading),Math.cos(yaw-m.heading))*(1-Math.exp(-dt*9));t.blocked=0;
  }else{
    t.blocked+=dt;
    if(occupied&&t.destination&&t.blocked>.25&&t.replans<3){
      t.replans++;
      const route=findHabitatPath([m.x,m.y,m.z],t.destination,agentObstacles(id));
      if(route){t.path=route;t.blocked=0;return;}
    }
    if(t.blocked>3){t.moving=false;t.path=[];t.label='Route occupied — choose another destination';publish();}
  }
}
