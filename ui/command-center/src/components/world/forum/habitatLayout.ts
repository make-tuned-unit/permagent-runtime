import { BoxGeometry, BufferGeometry, Float32BufferAttribute, Group, Mesh, MeshStandardMaterial } from 'three';
import { GARDEN_PLACES, PLANT_BASE_Y, type GardenDecorPlacement } from './gardenLayout';

export type Point3 = [number, number, number];
export const HABITAT_ARRIVAL = {position: [1.8, 2.15, 8.6] as Point3, target: [-3, 1.35, -2.5] as Point3, fov: 58};
export const HANK_TABLE_SEAT: Point3 = [-3,0,1.1];
export const TABLE_CONVERSATION_CAMERA = {position:[-3,1.50,4.80] as Point3,target:[-3,1.28,1.1] as Point3};
export const HABITAT_START: Point3 = [1.8, 0, 8.6];
/** All three levels belong to one continuous, gently sloping walking surface. */
export function habitatHeight(z: number) { return z >= -8 ? 0 : z <= -14 ? 3.4 : (-8-z)*(3.4/6); }
export function onGardenBridge(x:number,z:number){return x>=1&&x<=7&&z>=-3.2&&z<=-1.2;}
export function habitatElevation(x:number,z:number){return onGardenBridge(x,z)?Math.min(.45,(x-1)*.45,(7-x)*.45):habitatHeight(z);}
export function gardenBridgeGeometry(){
  const p:number[]=[],indices:number[]=[];
  for(let x=1;x<7;x+=.25){const i=p.length/3;p.push(x,habitatElevation(x,-2.2),-3.2,x,habitatElevation(x,-2.2),-1.2,x+.25,habitatElevation(x+.25,-2.2),-1.2,x+.25,habitatElevation(x+.25,-2.2),-3.2);indices.push(i,i+1,i+2,i,i+2,i+3);}
  const g=new BufferGeometry();g.setAttribute('position',new Float32BufferAttribute(p,3));g.setIndex(indices);g.computeVertexNormals();return g;
}
export function onHabitat(x: number, z: number, margin = 0) {
  const qx=Math.max(Math.abs(x)-12,0), qz=Math.max(Math.abs(z+6)-18,0);
  return Math.hypot(qx,qz) <= 4-margin && Math.abs(x)<=16-margin && z>=-28+margin && z<=16-margin;
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
  {...GARDEN_PLACES[2],id:'conservatory',name:'Conservatory',subtitle:'Cultivate & measure',position:[10,0,-7] as Point3,agent:'growth_measurement'},
];
export type HabitatSolid = {name:string;at:Point3;size:Point3;water?:boolean};
export const HABITAT_SOLIDS: HabitatSolid[] = [
  {name:'commons table',at:[-3,.43,3],size:[2.7,.86,2.7]},
  {name:'water garden',at:[4,.05,-2],size:[3.8,.1,6]},
  {name:'bridge north rail',at:[4,.95,-3.23],size:[4,.9,.07]},
  {name:'bridge south rail',at:[4,.95,-1.17],size:[4,.9,.07]},
  {name:'cascade basin',at:[4,.6,-8],size:[3.6,1.2,3.2]},
  ...HABITAT_STATIONS.filter(s=>s.id!=='henry').map(s=>({name:`${s.id} desk`,at:[s.position[0],s.position[1]+.42,s.position[2]-1.05] as Point3,size:[1.9,.84,.7] as Point3})),
  ...[-13,-7].map(x=>({name:'archive shelves',at:[x,4.9,-22] as Point3,size:[2.6,3,.5] as Point3})),
  {name:'archive rear wall',at:[-10,5,-23],size:[8,3.2,.35]},
  ...[-14,-6].map(x=>({name:'archive pillar',at:[x,5.2,-16] as Point3,size:[.3,3.6,.3] as Point3})),
  ...[7,14].map(x=>({name:'workshop pillar',at:[x,5,-16] as Point3,size:[.2,3.2,.2] as Point3})),
  ...[-.2,6.2].flatMap(x=>[-21.7,-26.3].map(z=>({name:'observatory pillar',at:[x,4.5,z] as Point3,size:[.16,2.2,.16] as Point3}))),
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
];
HABITAT_BEDS.forEach(b=>HABITAT_SOLIDS.push({name:'raised planting bed',at:[b.x,habitatHeight(b.z)+.25,b.z],size:[b.rx*2,.5,b.rz*2]}));
export const HABITAT_DECOR: GardenDecorPlacement[] = [
  {asset:'table',position:[-3,0,3],scale:1.2,heading:0},
  {asset:'instrument',position:[-3.82,1.02,2.95],scale:.55,heading:0},
  ...([[-4.5,0,5.3],[-5.3,0,1.5],[-12,3.4,-18],[0,3.4,-22],[7,0,10]] as Point3[]).map((position,i)=>({asset:'bench',position,scale:1.05,heading:i===1?Math.PI/2:Math.PI,solid:[1.62,1.015,.55] as Point3})),
];
function plant(asset:keyof typeof PLANT_BASE_Y,x:number,z:number,scale:number,heading:number) {
  HABITAT_DECOR.push({asset,position:[x,habitatHeight(z)+.025-PLANT_BASE_Y[asset]*scale,z],scale,heading,supportY:habitatHeight(z)+.025,...(asset==='canopy'?{solid:[.98,3.15,.98] as Point3}:asset==='shrub'?{solid:[1.44,.97,1.44] as Point3}:{})});
}
// Layered planting beds frame routes, with clear trunks and working sightlines.
const trees: [number,number,number][]=[[-7.8,5.8,1.6],[7.7,10,1.5],[-13,11,1.4],[-14,2,1.5],[-14,-10,1.6],[-14,-24,1.65],[-5,-25,1.55],[8,-26,1.45],[14,-12,1.4],[14,0,1.3],[0,-12,1.2]];
trees.forEach(([x,z,s],i)=>plant('canopy',x,z,s,i*1.31));
for(let side=-1;side<=1;side+=2) for(let i=0;i<22;i++) {
  const z=12-i*1.7, x=side*(14.5+Math.sin(i*2.1)*.35);
  if(onHabitat(x,z,1)) { plant('shrub',x,z,.75+(i%3)*.13,i*2.4); plant('fern',x-side*1.0,z+.35,.68,i*1.7); plant('flowers',x-side*.7,z-.45,.65,i); }
}
for(let i=0;i<17;i++) {
  const x=-12+i*1.5,z=-25.7+Math.sin(i)*.5;
  plant('shrub',x,z,.72+(i%3)*.14,i);plant('flowers',x,z+1,.8,i*1.6);
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
  const x=s.position[0]+1.3,z=s.position[2]-1.4,y=habitatHeight(z);
  HABITAT_DECOR.push({asset:'planter',position:[x,y,z],scale:.7,heading:0,solid:[1.1,.67,1.1]});
  HABITAT_DECOR.push({asset:'fern',position:[x,y+.4025-PLANT_BASE_Y.fern*.7,z],scale:.7,heading:x});
}
export function terrainGeometry() {
  const vertices:number[]=[],indices:number[]=[];
  // Regular cells also make the slope continuous at the terrace seams.
  for(let z=-28;z<16;z++)for(let x=-16;x<16;x++) {
    if(!onHabitat(x+.5,z+.5,.1))continue;
    const i=vertices.length/3;
    vertices.push(x,habitatHeight(z),z,x,habitatHeight(z+1),z+1,x+1,habitatHeight(z+1),z+1,x+1,habitatHeight(z),z);
    indices.push(i,i+1,i+2,i,i+2,i+3);
  }
  const geometry=new BufferGeometry();geometry.setAttribute('position',new Float32BufferAttribute(vertices,3));geometry.setIndex(indices);geometry.computeVertexNormals();return geometry;
}
export function habitatCollisionSource() {
  const root=new Group(), material=new MeshStandardMaterial();
  const ground=new Mesh(terrainGeometry(),material);ground.name='connected garden terraces';root.add(ground);const bridge=new Mesh(gardenBridgeGeometry(),material);bridge.name='walkable garden bridge';root.add(bridge);
  const solids=[...HABITAT_SOLIDS];
  // Physical rails coincide with the visible boundary. Rounded corners remain planted.
  solids.push({name:'west rail',at:[-15.7,2.4,-6],size:[.25,6,38]}, {name:'east rail',at:[15.7,2.4,-6],size:[.25,6,38]}, {name:'north rail',at:[0,4.2,-27.6],size:[25,2,.25]}, {name:'south rail',at:[0,.5,15.6],size:[25,1,.25]});
  HABITAT_DECOR.forEach(p=>{if(p.solid)solids.push({name:p.asset,at:[p.position[0],p.position[1]+p.solid[1]*p.scale/2,p.position[2]],size:p.solid.map(v=>v*p.scale) as Point3});});
  for(const s of solids){const m=s.name==='water garden'?material.clone():material;if(s.name==='water garden')m.name='Water';const mesh=new Mesh(new BoxGeometry(...s.size),m);mesh.name=s.name;mesh.position.set(...s.at);root.add(mesh);}
  root.updateMatrixWorld(true);return root;
}
/** Navigation uses the same solid footprint as rendering/collision. */
export function habitatWalkable(x:number,z:number,margin=.47) {
  if(!onHabitat(x,z,.8))return false;
  if(HABITAT_SOLIDS.some(s=>!(s.name==='water garden'&&onGardenBridge(x,z))&&Math.abs(x-s.at[0])<s.size[0]/2+margin&&Math.abs(z-s.at[2])<s.size[2]/2+margin))return false;
  return !HABITAT_DECOR.some(p=>p.solid&&Math.abs(x-p.position[0])<p.solid[0]*p.scale/2+margin&&Math.abs(z-p.position[2])<p.solid[2]*p.scale/2+margin);
}
