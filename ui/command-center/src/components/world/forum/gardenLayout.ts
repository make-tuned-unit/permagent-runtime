import { BoxGeometry, CylinderGeometry, Group, Mesh, MeshStandardMaterial } from 'three';
import { DISTRICTS } from './districts';

/** A finite, level garden: every listed destination shares the same walkable floor. */
export const GARDEN_ARRIVAL = { position: [-.4, 1.7, 8.2], target: [-2.3, 1.3, 1], fov: 58 } as const;
export const GARDEN_START: [number, number, number] = [-.4, 0, 8.2];
export const GARDEN_PLACES = [
  { ...DISTRICTS[0], name: 'Arrival table', subtitle: 'A place to begin', description: 'Meet your orchestrator beside the garden table. All paths in this compact garden are connected.', position: [-3, 0, 3] as [number,number,number] },
  { ...DISTRICTS.find(p => p.id === 'gallery')!, name: 'Evidence archive', subtitle: 'Inspect, choose, understand', description: 'A demonstration about completion evidence. Read the records, make a choice and inspect the explanation.', position: [-6, 0, -7] as [number,number,number] },
  { ...DISTRICTS.find(p => p.id === 'maker')!, name: 'Water garden', subtitle: 'Follow the planted water edge', description: 'A small water garden with a continuous path on each side. Follow the path toward the overlook.', position: [7, 0, 2] as [number,number,number] },
  { ...DISTRICTS.find(p => p.id === 'council_garden')!, name: 'Horizon overlook', subtitle: 'Return by either garden path', description: 'An open prospect over the stars. The west archive path and east water path both return to the arrival table.', position: [0, 0, -9] as [number,number,number] },
];
export const GARDEN_ROUTE: [number,number,number][] = [[0,0,8],[-6,0,8],[-6,0,-5],[-6,0,-9],[0,0,-9],[7,0,-9],[7,0,3],[7,0,8],[0,0,8]];
export const GARDEN_OBSTACLES = [
  { name: 'table', center: [-3,.6,3], size: [2.8,1.2,2.8] },
  { name: 'water', center: [4,.08,-2], size: [4,.16,6] },
  { name: 'archive', center: [-7,.6,-7], size: [1.2,1.2,1.2] },
  { name: 'pergola west-front', center: [-7,1.7,7], size: [.18,3.4,.18] },
  { name: 'pergola east-front', center: [-1,1.7,7], size: [.18,3.4,.18] },
  { name: 'pergola west-back', center: [-7,1.7,1], size: [.18,3.4,.18] },
  { name: 'pergola east-back', center: [-1,1.7,1], size: [.18,3.4,.18] },
  { name: 'west border', center: [-8.8,.5,0], size: [.4,1,24] },
  { name: 'east border', center: [8.8,.5,0], size: [.4,1,24] },
  { name: 'north border', center: [0,.5,-11.8], size: [18,1,.4] },
  { name: 'south border', center: [0,.5,11.8], size: [18,1,.4] },
] as const;
export type GardenDecorPlacement = {asset:string;position:[number,number,number];scale:number;heading:number;solid?:[number,number,number]};
/** One placement manifest drives visible geometry and its solid collision proxies. */
export const GARDEN_DECOR: GardenDecorPlacement[] = [
  {asset:'table',position:[-3,0,3],scale:1,heading:0},
  {asset:'instrument',position:[-3,.85,3],scale:1,heading:0},
  ...([[-4.5,0,5.4],[-4.5,0,-10.2],[3,0,9.8]] as [number,number,number][]).map((position,i)=>({asset:'bench',position,scale:1,heading:i===1?0:Math.PI,solid:[1.62,1.015,.54043] as [number,number,number]})),
  ...([[-8,.15,7],[-8,.15,0],[-8,.15,-9],[8,.15,8],[8,.15,-8]] as [number,number,number][]).map((position,i)=>({asset:'canopy',position,scale:.88+i%3*.06,heading:i*1.2,solid:[.32,3.15,.32] as [number,number,number]})),
];
const planters: [number,number,number][]=[[-8,0,4],[-8,0,-3],[-8,0,-6],[8,0,5],[8,0,-4],[3,0,-10],[5,0,-10],[-3,0,-10],[4,0,1.7]];
planters.forEach((position,i)=>{
  GARDEN_DECOR.push({asset:'planter',position,scale:.72,heading:0,solid:[1.1,.67,1.1]});
  GARDEN_DECOR.push({asset:i%2?'shrub':'fern',position:[position[0],.44,position[2]],scale:.7,heading:i*.81});
});
for(let i=0;i<14;i++)GARDEN_DECOR.push({asset:i%3===0?'flowers':'fern',position:[-8+.28*(i%2),.1,9.5-i*1.4],scale:.42,heading:i*1.8});
for(let i=0;i<8;i++)GARDEN_DECOR.push({asset:'shrub',position:[8,.05,9-i*2.6],scale:.65,heading:i*.63});
for(let i=0;i<5;i++)GARDEN_DECOR.push({asset:'vine',position:[-7+i*1.5,3.25,1],scale:.8,heading:i%2*.2});
export const GARDEN_AGENT_SEATS: [number,number,number][]=[[-1.1,0,3.1],[-4.8,0,-7.5],[6.2,0,4],[-1.3,0,-10.2],[-6.7,0,.5],[.5,0,9.4],[6.2,0,6.2],[-6.8,0,8.5],[.5,0,-5.5],[-4.5,0,-2],[.5,0,0],[5,0,-8.3]];
export function gardenCollisionSource() {
  const root = new Group();
  const material = new MeshStandardMaterial();
  const floor = new Mesh(new BoxGeometry(18,.3,24), material);
  floor.position.y = -.15; root.add(floor);
  for (const obstacle of GARDEN_OBSTACLES) {
    const geometry = obstacle.name === 'table' ? new CylinderGeometry(1.12,1.12,.8245,48) : new BoxGeometry(obstacle.size[0],obstacle.size[1],obstacle.size[2]);
    const mesh = new Mesh(geometry, material);
    if(obstacle.name === 'water'){const water = material.clone(); water.name='Water';mesh.material=water;}
    mesh.name = obstacle.name; mesh.position.set(obstacle.center[0],obstacle.name === 'table' ? .41225 : obstacle.center[1],obstacle.center[2]); root.add(mesh);
  }
  for(const item of GARDEN_DECOR){
    if(!item.solid)continue;
    const [sx,sy,sz]=item.solid.map(v=>v*item.scale);
    const shape = item.asset === 'planter' || item.asset === 'canopy' ? new CylinderGeometry(sx/2,sx/2,sy,24) : new BoxGeometry(sx,sy,sz);
    const collider=new Mesh(shape,material);
    collider.name=item.asset;collider.position.set(item.position[0],item.position[1]+sy/2,item.position[2]);collider.rotation.y=item.heading;root.add(collider);
  }
  for(const seat of GARDEN_AGENT_SEATS){
    const body=new Mesh(new CylinderGeometry(.32,.32,1.65,24),material);body.name='agent body';body.position.set(seat[0],seat[1]+.825,seat[2]);root.add(body);
  }
  root.updateMatrixWorld(true);
  return root;
}
