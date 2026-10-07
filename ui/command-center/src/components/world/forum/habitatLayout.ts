import { BoxGeometry, BufferGeometry, Float32BufferAttribute, Group, Mesh, MeshStandardMaterial } from 'three';
import {
  OBSERVATORY_BRIDGE,
  OBSERVATORY_BENCHES,
  OBSERVATORY_PAD,
  observatoryGuardSegments,
  OBSERVATORY_GARDEN_PLACE,
  OBSERVATORY_PLINTH,
  OBSERVATORY_WING_HEIGHT,
  onObservatoryWing,
} from './habitatObservatoryWing';
import { terraceSurfaceGeometry } from './habitatTerrace';
import { GARDEN_PLACES, PLANT_BASE_Y, type GardenDecorPlacement } from './gardenLayout';

export type Point3 = [number, number, number];
/** Human-scale arrival: clear of every station/body and solid, with the host,
 * table, and water route sharing the eye-level frame. */
export const HABITAT_ARRIVAL = {position: [-0.5, 1.60, 5.5] as Point3, target: [-1.1, 1.30, -1.5] as Point3, fov: 72};
/** The overview keeps the full compact footprint in frame at both supported
 * 16:10 review sizes. Its lower target gives the connected terraces room to
 * read without returning to the rejected elevated surveillance stare. */
export const HABITAT_OVERVIEW = {position: [40, 43, 37] as Point3, target: [0, -3, -12] as Point3, fov: 48};
/** Reverse arrival composition used by the authored rear planting/arbor
 * contract. Orbiting around the arrival should reveal a second lived-in
 * foreground, not an empty service side of the set. */
export const HABITAT_ARRIVAL_REVERSE = {position: [-1.5, 1.60, 4.5] as Point3, target: [-1.5, 2.0, 10.0] as Point3, fov: 80};
export const ORCHESTRATOR_TABLE_SEAT: Point3 = [-3,0,1.1];
export const TABLE_CONVERSATION_CAMERA = {position:[-3,1.50,4.80] as Point3,target:[-3,1.28,1.1] as Point3};
// Both the crafted table and its original fallback meet this usable surface.
export const COMMONS_SURFACE_Y=.989;
export const HABITAT_START: Point3 = [1.8, 0, 8.6];
/** All three levels belong to one continuous, gently sloping walking surface. */
export function habitatHeight(z: number) { return z >= -8 ? 0 : z <= -14 ? 3.4 : (-8-z)*(3.4/6); }
export function onGardenBridge(x:number,z:number){return x>=1&&x<=7&&z>=-3.2&&z<=-1.2;}
export function habitatElevation(x:number,z:number){
  if(onObservatoryWing(x,z,0))return OBSERVATORY_WING_HEIGHT;
  return onGardenBridge(x,z)?Math.min(.45,(x-1)*.45,(7-x)*.45):habitatHeight(z);
}
export function gardenBridgeGeometry(){
  const p:number[]=[],indices:number[]=[];
  for(let x=1;x<7;x+=.25){const i=p.length/3;p.push(x,habitatElevation(x,-2.2),-3.2,x,habitatElevation(x,-2.2),-1.2,x+.25,habitatElevation(x+.25,-2.2),-1.2,x+.25,habitatElevation(x+.25,-2.2),-3.2);indices.push(i,i+1,i+2,i,i+2,i+3);}
  const g=new BufferGeometry();g.setAttribute('position',new Float32BufferAttribute(p,3));g.setIndex(indices);g.computeVertexNormals();return g;
}
export function onMainTerrace(x: number, z: number, margin = 0) {
  const qx=Math.max(Math.abs(x)-12,0), qz=Math.max(Math.abs(z+6)-18,0);
  return Math.hypot(qx,qz) <= 4-margin && Math.abs(x)<=16-margin && z>=-28+margin && z<=16-margin;
}
export function onHabitat(x: number, z: number, margin = 0) {
  if (onObservatoryWing(x, z, margin)) return true;
  return onMainTerrace(x, z, margin);
}
export type Station = {id:string;name:string;detail:string;position:Point3;heading:number;color:string;cloth:string;pace:number;gesture:'explain'|'read'|'type'|'scan'|'tend'};
const station = (id:string,name:string,detail:string,x:number,z:number,color:string,cloth:string,pace:number,gesture:Station['gesture']):Station => ({id,name,detail,position:[x,habitatHeight(z),z],heading:Math.PI,color,cloth,pace,gesture});
export const HABITAT_STATIONS: Station[] = [
  {...station('henry','The commons','Plans, briefs & coordination',-1.1,3.1,'#b69963','#ddd2b5',1.0,'explain'),heading:-Math.PI/2},
  station('librarian','Garden archive','Memory, sources & provenance',-10,-19,'#af91c5','#65516f',.8,'read'),
  station('reader','Reading room','Documents & interpretation',-6,-17,'#71bdba','#386e72',1.12,'read'),
  station('watcher','Lookout','Signals & observations',11,-23,'#93b6d0','#344e6b',.95,'scan'),
  station('steward','Maker workshop','Repositories, repairs & tools',10,-17,'#daa15e','#986338',1.06,'type'),
  station('strix','Sentinel station','Security findings & evidence',12,-3,'#b3becb','#35424e',1.1,'scan'),
  station('financier','Ledger court','Accounts, sources & calculations',-11,-3,'#c69e7f','#704851',.9,'type'),
  station('forecaster','Observatory','Scenarios & changing horizons',3,-24,'#97bed1','#597b9c',.85,'scan'),
  station('council','Council garden','Perspectives & deliberation',-1,-19,'#c9abdd','#75628d',.82,'explain'),
  station('polybot','Operations bay','Configured rules & process status',11,7,'#d8ac74','#4a575c',1.05,'type'),
  station('picker','Discovery desk','Candidates, criteria & comparisons',-10,8,'#bac88a','#778660',1.18,'read'),
  station('growth_measurement','Conservatory','Experiments & measured outcomes',10,-8,'#8cba86','#53795c',.92,'tend'),
];
export const stationFor = (id:string) => HABITAT_STATIONS.find(s=>s.id===id) ?? HABITAT_STATIONS[0];
export const HABITAT_PLACES = [
  {...GARDEN_PLACES[0],description:'A shared table beneath the vines. Meet your team, prepare a brief, or send an agent to their workstation.'},
  {...GARDEN_PLACES[1],name:'Garden archive',subtitle:'Sources, stories & memory',position:[-8,3.4,-18] as Point3},
  {...GARDEN_PLACES[2],name:'Maker workshop',subtitle:'See the work take shape',position:[9,3.4,-17] as Point3,agent:'steward'},
  {...GARDEN_PLACES[3],name:'Observatory',subtitle:'A wider view',position:[2,3.4,-23] as Point3,agent:'forecaster'},
  {...GARDEN_PLACES[3],id:OBSERVATORY_GARDEN_PLACE.id,name:OBSERVATORY_GARDEN_PLACE.name,subtitle:OBSERVATORY_GARDEN_PLACE.subtitle,description:OBSERVATORY_GARDEN_PLACE.description,position:OBSERVATORY_GARDEN_PLACE.position,agent:'forecaster'},
  {...GARDEN_PLACES[2],id:'conservatory',name:'Conservatory',subtitle:'Cultivate & measure',position:[10,0,-7] as Point3,agent:'growth_measurement'},
];
export type HabitatSolid = {name:string;at:Point3;size:Point3;water?:boolean};
export const ARCHIVE_DECOR:GardenDecorPlacement[]=[
  ...[-13,-10,-7].map(x=>({asset:'bookcase',position:[x,3.4,-22.3] as Point3,scale:1,heading:0,solid:[2.6,2.8,.54] as Point3})),
  ...[-14,-6].map(x=>({asset:'column',position:[x,3.4,-16] as Point3,scale:1,heading:0,solid:[.54,3.6,.54] as Point3})),
];
export const STATION_DESKS:GardenDecorPlacement[]=HABITAT_STATIONS.filter(s=>s.id!=='henry').map(s=>({
  asset:'desk',position:[s.position[0],s.position[1],s.position[2]-.90],scale:1,heading:0,
}));
export const HABITAT_SOLIDS: HabitatSolid[] = [
  ...OBSERVATORY_BENCHES.map(({x,z})=>({name:'observatory bench',at:[x,OBSERVATORY_WING_HEIGHT+.45,z] as Point3,size:[3.4,.9,.75] as Point3})),
  {name:'commons table',at:[-3,.495,3],size:[2.7,.99,2.7]},
  {name:'water garden',at:[4,.05,-2],size:[3.8,.1,6]},
  {name:'bridge north rail',at:[4,.95,-3.23],size:[4,.9,.07]},
  {name:'bridge south rail',at:[4,.95,-1.17],size:[4,.9,.07]},
  {name:'cascade basin',at:[4,.6,-8],size:[3.6,1.2,3.2]},
  {name:'cascade receiving rill',at:[4,.05,-5.7],size:[3.2,.1,1.4]},
  ...HABITAT_STATIONS.filter(s=>s.id!=='henry').map(s=>({name:`${s.id} desk`,at:[s.position[0],s.position[1]+.445,s.position[2]-.90] as Point3,size:[1.9,.89,.72] as Point3})),
  ...ARCHIVE_DECOR.map(p=>({name:p.asset==='column'?'archive pillar':'archive shelves',at:[p.position[0],p.position[1]+p.solid![1]/2,p.position[2]] as Point3,size:p.solid!})),
  {name:'archive rear wall',at:[-10,5,-23],size:[8,3.2,.35]},
  ...[7,14].map(x=>({name:'workshop pillar',at:[x,5,-16] as Point3,size:[.2,3.2,.2] as Point3})),
  ...[-.2,8.8].flatMap(x=>[-21.7,-26.3].map(z=>({name:'observatory pillar',at:[x,4.5,z] as Point3,size:[.16,2.2,.16] as Point3}))),
  {name:'observatory plinth',at:[OBSERVATORY_PLINTH.x,OBSERVATORY_WING_HEIGHT+OBSERVATORY_PLINTH.height/2,OBSERVATORY_PLINTH.z],size:[OBSERVATORY_PLINTH.radius*2,OBSERVATORY_PLINTH.height,OBSERVATORY_PLINTH.radius*2]},
];
export const HABITAT_BEDS: {x:number;z:number;rx:number;rz:number;groundcover?:boolean}[] = [
  {x:-9,z:2,rx:1.8,rz:2.1},{x:-5,z:-6,rx:2.1,rz:2.4},
  {x:10,z:2,rx:1.5,rz:1.8},{x:-1,z:-16,rx:1.7,rz:1.5},
  {x:-12,z:-12,rx:1.7,rz:1.7},{x:12,z:-12,rx:1.6,rz:1.5},
  {x:5.5,z:8,rx:1.4,rz:1.8},
  // Planted contour bands break up the terrace slope while leaving broad
  // walking lanes between the commons, archive and workshop.
  {x:-10.5,z:-10.7,rx:2.5,rz:.7,groundcover:true},
  {x:-1.5,z:-10.7,rx:3.0,rz:.7,groundcover:true},
  {x:10.6,z:-10.7,rx:2.3,rz:.7,groundcover:true},
  {x:-2,z:-13.3,rx:2.8,rz:.65,groundcover:true},
  {x:OBSERVATORY_PAD.cx-10,z:OBSERVATORY_PAD.cz-.5,rx:2.3,rz:3.0,groundcover:true},
  {x:OBSERVATORY_PAD.cx+10,z:OBSERVATORY_PAD.cz-1,rx:2.3,rz:3.0,groundcover:true},
  {x:OBSERVATORY_PAD.cx,z:OBSERVATORY_PAD.cz-8,rx:5.5,rz:1.4,groundcover:true},
];
HABITAT_BEDS.forEach(b=>HABITAT_SOLIDS.push({name:'raised planting bed',at:[b.x,habitatHeight(b.z)+.25,b.z],size:[b.rx*2,.5,b.rz*2]}));
export const HABITAT_DECOR: GardenDecorPlacement[] = [
  {asset:'table',position:[-3,0,3],scale:1.2,heading:0},
  {asset:'instrument',position:[-3.82,.99,2.95],scale:.55,heading:0},
  ...([[-4.5,0,5.3],[-5.3,0,1.5],[-12,3.4,-18],[0,3.4,-22],[7,0,10]] as Point3[]).map((position,i)=>({asset:'bench',position,scale:1.05,heading:i===1?Math.PI/2:Math.PI,solid:[1.62,1.015,.55] as Point3})),
];
function plant(asset:keyof typeof PLANT_BASE_Y,x:number,z:number,scale:number,heading:number) {
  HABITAT_DECOR.push({asset,position:[x,habitatHeight(z)+.025-PLANT_BASE_Y[asset]*scale,z],scale,heading,supportY:habitatHeight(z)+.025,...(asset==='canopy'?{solid:[.98,3.15,.98] as Point3}:asset==='shrub'?{solid:[1.44,.97,1.44] as Point3}:{})});
}
// Layered planting beds frame routes, with clear trunks and working sightlines.
const trees: [number,number,number][]=[[-7.8,5.8,1.6],[7.7,10,1.5],[-13,11,1.4],[-14,2,1.5],[-14,-10,1.6],[-14,-24,1.65],[-5,-25,1.55],[8,-26,1.45],[14,-12,1.4],[14,0,1.3],[0,-12,1.2]];
trees.forEach(([x,z,s],i)=>plant('canopy',x,z,s,i*1.31));
for(const side of [-1,1])plant('canopy',OBSERVATORY_PAD.cx+side*10,OBSERVATORY_PAD.cz-.7,1.1,side*.8);
for(let side=-1;side<=1;side+=2) for(let i=0;i<22;i++) {
  const z=12-i*1.7, x=side*(14.5+Math.sin(i*2.1)*.35);
  if(onHabitat(x,z,1)) { plant('shrub',x,z,.75+(i%3)*.13,i*2.4); plant('fern',x-side*1.0,z+.35,.68,i*1.7); plant('flowers',x-side*.7,z-.45,.65,i); }
}
for(let i=0;i<17;i++) {
  const x=-12+i*1.5,z=-25.7+Math.sin(i)*.5;
  if(x>OBSERVATORY_BRIDGE.x0-.8&&x<OBSERVATORY_BRIDGE.x1+.8)continue;
  plant('shrub',x,z,.72+(i%3)*.14,i);plant('flowers',x,z+1,.8,i*1.6);
}
for(const [dx,dz,s,h] of [[-10,-1,.85,1.2],[-8,-4,.78,2.1],[10,-1,.82,3.4],[9,-4,.74,4.8],[-2,-8,.7,5.6]] as [number,number,number,number][]){
  const x=OBSERVATORY_PAD.cx+dx,z=OBSERVATORY_PAD.cz+dz;
  plant('shrub',x,z,s,h);plant('fern',x+.55,z+.45,.68,h+.5);
}
// Intimate foreground beds and planting around the water, away from circulation.
for(const [x,z] of [[-7,3],[-7,1],[-7,-1],[7,3],[7,-1],[7,-4],[7,-6],[-5,10],[-3,11],[-1,11],[1,11],[-11,13],[10,12]] as [number,number][]) {
  plant('fern',x,z,.9,x);plant('flowers',x+.45,z+.5,.72,z);plant('shrub',x-.5,z-.5,.65,x+z);
}
for(const bed of HABITAT_BEDS.filter(b=>!b.groundcover))for(let i=0;i<16;i++) {
  const a=i*2.39996,r=Math.sqrt((i+.5)/16)*.86;
  const x=bed.x+Math.cos(a)*bed.rx*r,z=bed.z+Math.sin(a)*bed.rz*r;
  const asset=i%4===0?'shrub':i%3===0?'flowers':'fern';
  HABITAT_DECOR.push({asset,position:[x,habitatHeight(bed.z)+.45-PLANT_BASE_Y[asset]*.85,z],scale:.72+(i%3)*.13,heading:a});
}
for(const s of HABITAT_STATIONS.filter(s=>s.id!=='henry')) {
  const x=s.position[0]+(s.id==='forecaster'?-1.35:1.3),z=s.position[2]-1.4,y=habitatHeight(z);
  HABITAT_DECOR.push({asset:'planter',position:[x,y,z],scale:.7,heading:0,solid:[1.1,.67,1.1]});
  HABITAT_DECOR.push({asset:'fern',position:[x,y+.4025-PLANT_BASE_Y.fern*.7,z],scale:.7,heading:x});
}
export const terrainGeometry=()=>terraceSurfaceGeometry();
/** Rectangular benches carry their rotated footprint; planting uses radial proxies. */
export function decorSolidSize(p:GardenDecorPlacement):Point3 {
  const [width,height,depth]=p.solid!;
  const angle=p.asset==='bench'?p.heading:0,c=Math.abs(Math.cos(angle)),s=Math.abs(Math.sin(angle));
  return [(width*c+depth*s)*p.scale,height*p.scale,(width*s+depth*c)*p.scale];
}
export function habitatCollisionSource() {
  const root=new Group(), material=new MeshStandardMaterial();
  const ground=new Mesh(terrainGeometry(),material);ground.name='connected garden terraces';root.add(ground);const bridge=new Mesh(gardenBridgeGeometry(),material);bridge.name='walkable garden bridge';root.add(bridge);
  const solids=[...HABITAT_SOLIDS];
  // Physical rails coincide with the visible boundary. Rounded corners remain planted.
  solids.push({name:'west rail',at:[-15.7,2.4,-6],size:[.25,6,38]}, {name:'east rail',at:[15.7,2.4,-6],size:[.25,6,38]});
  const bridgeGap0=OBSERVATORY_BRIDGE.x0,bridgeGap1=OBSERVATORY_BRIDGE.x1;
  solids.push({name:'north rail west',at:[(-12+bridgeGap0)/2,4.2,-27.6],size:[12+bridgeGap0,2,.25]},{name:'north rail east',at:[(12+bridgeGap1)/2,4.2,-27.6],size:[12-bridgeGap1,2,.25]},{name:'south rail',at:[0,.5,15.6],size:[25,1,.25]});
  for(const {a,b} of observatoryGuardSegments())solids.push({name:'observatory guard',at:[(a[0]+b[0])/2,OBSERVATORY_WING_HEIGHT+.5,(a[1]+b[1])/2],size:[Math.abs(b[0]-a[0])+.08,1,Math.abs(b[1]-a[1])+.08]});
  HABITAT_DECOR.forEach(p=>{if(p.solid)solids.push({name:p.asset,at:[p.position[0],p.position[1]+p.solid[1]*p.scale/2,p.position[2]],size:decorSolidSize(p)});});
  for(const s of solids){const m=s.name==='water garden'?material.clone():material;if(s.name==='water garden')m.name='Water';const mesh=new Mesh(new BoxGeometry(...s.size),m);mesh.name=s.name;mesh.position.set(...s.at);root.add(mesh);}
  root.updateMatrixWorld(true);return root;
}
/** Navigation uses the same solid footprint as rendering/collision. */
export function habitatWalkable(x:number,z:number,margin=.47) {
  if(!onHabitat(x,z,.8))return false;
  if(HABITAT_SOLIDS.some(s=>!(s.name==='water garden'&&onGardenBridge(x,z))&&Math.abs(x-s.at[0])<s.size[0]/2+margin&&Math.abs(z-s.at[2])<s.size[2]/2+margin))return false;
  return !HABITAT_DECOR.some(p=>{if(!p.solid)return false;const size=decorSolidSize(p);return Math.abs(x-p.position[0])<size[0]/2+margin&&Math.abs(z-p.position[2])<size[2]/2+margin;});
}
