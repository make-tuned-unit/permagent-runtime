import { BufferGeometry, Float32BufferAttribute } from 'three';
import {
  OBSERVATORY_PAD,
  observatoryPadOutline,
  OBSERVATORY_WING_HEIGHT,
  observatoryBridgeMetreCells,
  onObservatoryBridge,
  onObservatoryPad,
} from './habitatObservatoryWing';

type Point2=[number,number];
const floorHeight=(z:number)=>z>=-8?0:z<=-14?3.4:(-8-z)*3.4/6;

/** A single rounded outline shared by paving, hull and coping. Straight sides
 * are split at terrace seams so the lower hull follows the walkable slope. */
export function terracePerimeter(inset=0):Point2[]{
  const corners:Point2[]=[[12,12],[-12,12],[-12,-24],[12,-24]],arcs:Point2[][]=[];
  corners.forEach(([cx,cz],corner)=>arcs.push(Array.from({length:13},(_,i)=>{
    const angle=(corner+i/12)*Math.PI/2;return [cx+Math.cos(angle)*(4-inset),cz+Math.sin(angle)*(4-inset)];
  })));
  const result:Point2[]=[];
  arcs.forEach((arc,index)=>{
    result.push(...arc);const from=arc[arc.length-1],to=arcs[(index+1)%4][0],distance=Math.hypot(to[0]-from[0],to[1]-from[1]);
    const cuts=Array.from({length:Math.max(0,Math.ceil(distance)-1)},(_,i)=>(i+1)/Math.ceil(distance));
    if(to[1]!==from[1])for(const z of [-14,-8]){const t=(z-from[1])/(to[1]-from[1]);if(t>0&&t<1)cuts.push(t);}
    for(const t of [...new Set(cuts)].sort((a,b)=>a-b))result.push([from[0]+(to[0]-from[0])*t,from[1]+(to[1]-from[1])*t]);
  });return result;
}

/** Rounded outline of the observatory garden pad (independent of the main terrace). */
export const observatoryPadPerimeter=observatoryPadOutline;

function makeGeometry(vertices:number[],indices:number[]){
  const geometry=new BufferGeometry();geometry.setAttribute('position',new Float32BufferAttribute(vertices,3));
  geometry.setAttribute('uv',new Float32BufferAttribute(vertices.flatMap((v,i)=>i%3===1?[]:[v]),2));
  geometry.setIndex(indices);geometry.computeVertexNormals();return geometry;
}

function clipToOutline(polygon:Point2[],outline:Point2[]):Point2[]{
  let current=polygon;
  for(let i=0;i<outline.length&&current.length;i++){
    const a=outline[i],b=outline[(i+1)%outline.length],cross=(p:Point2)=>(b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]);
    const next:Point2[]=[];
    for(let j=0;j<current.length;j++){
      const p=current[j],q=current[(j+1)%current.length],dp=cross(p),dq=cross(q);
      if(dp>=-1e-8)next.push(p);
      if((dp>=0)!==(dq>=0)){const t=dp/(dp-dq);next.push([p[0]+(q[0]-p[0])*t,p[1]+(q[1]-p[1])*t]);}
    }
    current=next;
  }
  return current;
}

function appendPolygon(vertices:number[],indices:number[],polygon:Point2[],height:number){
  if(polygon.length<3)return;
  const base=vertices.length/3;
  polygon.forEach(([px,pz])=>vertices.push(px,height,pz));
  for(let i=1;i<polygon.length-1;i++)indices.push(base,base+i,base+i+1);
}

/** Inward offset for a point on the wing rounded-rect outline. */
function wingOutlineInset(x:number,z:number,inset:number):Point2{
  const {cx,cz,hx,hz,corner}=OBSERVATORY_PAD;
  const r=Math.max(0.05,corner);
  const innerX=hx-r,innerZ=hz-r;
  const left=cx-hx,right=cx+hx,top=cz+hz,bottom=cz-hz;
  if(x>=cx-innerX&&x<=cx+innerX&&z>=top-r&&z<=top){
    return [x,Math.min(top-inset,z-inset)];
  }
  if(x>=cx-innerX&&x<=cx+innerX&&z>=bottom&&z<=bottom+r){
    return [x,Math.max(bottom+inset,z+inset)];
  }
  if(z>=cz-innerZ&&z<=cz+innerZ&&x>=right-r&&x<=right){
    return [Math.min(right-inset,x-inset),z];
  }
  if(z>=cz-innerZ&&z<=cz+innerZ&&x>=left&&x<=left+r){
    return [Math.max(left+inset,x+inset),z];
  }
  const cornerCenters:Point2[]=[[cx+innerX,cz+innerZ],[cx-innerX,cz+innerZ],[cx-innerX,cz-innerZ],[cx+innerX,cz-innerZ]];
  let nearest=cornerCenters[0],best=Infinity;
  for(const c of cornerCenters){
    const d=Math.hypot(x-c[0],z-c[1]);
    if(d<best){best=d;nearest=c;}
  }
  const dx=x-nearest[0],dz=z-nearest[1],len=Math.hypot(dx,dz)||1;
  const radial=Math.max(0,r-inset);
  return [nearest[0]+dx/len*radial,nearest[1]+dz/len*radial];
}

/** Clip each metre cell against the true rounded border. All triangles remain
 * on the three connected planes; no stair steps or unsupported corner tiles.
 * The observatory wing pad + bridge are appended as a second connected deck. */
export function terraceSurfaceGeometry(omitPool=false){
  const outline=terracePerimeter(),wingOutline=observatoryPadPerimeter(),vertices:number[]=[],indices:number[]=[];
  for(let z=-28;z<16;z++)for(let x=-16;x<16;x++){
    if(omitPool&&x>=2&&x<6&&z>=-5&&z<1)continue;
    const polygon=clipToOutline([[x,z],[x,z+1],[x+1,z+1],[x+1,z]],outline);
    appendPolygon(vertices,indices,polygon,floorHeight((polygon[0]?.[1]??z)+.5));
    if(polygon.length>=3){
      const base=(vertices.length/3)-polygon.length;
      polygon.forEach(([,pz],i)=>{vertices[(base+i)*3+1]=floorHeight(pz);});
    }
  }
  // Bridge deck — only fully interior metre cells; skip pad overlap to avoid z-fighting.
  for(const {x,z} of observatoryBridgeMetreCells()){
    const cx=x+.5,cz=z+.5;
    if(onObservatoryPad(cx,cz,0.02))continue;
    if(z>=-28&&onObservatoryBridge(cx,cz,-0.02))continue;
    appendPolygon(vertices,indices,[[x,z],[x,z+1],[x+1,z+1],[x+1,z]],OBSERVATORY_WING_HEIGHT);
  }
  const padMinZ=Math.floor(OBSERVATORY_PAD.cz-OBSERVATORY_PAD.hz);
  const padMaxZ=Math.ceil(OBSERVATORY_PAD.cz+OBSERVATORY_PAD.hz);
  const padMinX=Math.floor(OBSERVATORY_PAD.cx-OBSERVATORY_PAD.hx);
  const padMaxX=Math.ceil(OBSERVATORY_PAD.cx+OBSERVATORY_PAD.hx);
  for(let z=padMinZ;z<padMaxZ;z++)for(let x=padMinX;x<padMaxX;x++){
    const cx=x+.5,cz=z+.5;
    if(!onObservatoryPad(cx,cz,-0.35)&&!onObservatoryPad(x,z,-0.02)&&!onObservatoryPad(x+1,z+1,-0.02))continue;
    const polygon=clipToOutline([[x,z],[x,z+1],[x+1,z+1],[x+1,z]],wingOutline);
    appendPolygon(vertices,indices,polygon,OBSERVATORY_WING_HEIGHT);
  }
  return makeGeometry(vertices,indices);
}

/** Stone shell under the observatory wing pad (bridge stays visually thin). */
export function observatoryWingHullGeometry(){
  const outline=observatoryPadPerimeter(),vertices:number[]=[],indices:number[]=[];
  const sections=[[0,0],[.14,0],[.26,.16],[.58,.16],[.72,.04],[1.55,.7],[2.2,1.4]] as const;
  const {cx,cz}=OBSERVATORY_PAD;
  for(const [drop,inset] of sections)for(const [x,z] of outline){
    const [ix,iz]=wingOutlineInset(x,z,inset);
    vertices.push(ix,OBSERVATORY_WING_HEIGHT-drop-.008,iz);
  }
  const n=outline.length;
  for(let row=0;row<sections.length-1;row++)for(let i=0;i<n;i++){
    const a=row*n+i,b=row*n+(i+1)%n,c=a+n,d=b+n;indices.push(a,b,c,b,d,c);
  }
  const center=vertices.length/3;vertices.push(cx,OBSERVATORY_WING_HEIGHT-2.208,cz);
  const base=(sections.length-1)*n;
  for(let i=0;i<n;i++)indices.push(center,base+i,base+(i+1)%n);
  return makeGeometry(vertices,indices);
}

/** Architectural shell: continuous stone cornice, recessed band and tapered
 * lower support. It stays beneath the same fully traversable footprint. */
export function terraceHullGeometry(){
  const outline=terracePerimeter(),vertices:number[]=[],indices:number[]=[];
  const sections=[[0,0],[.16,0],[.28,.18],[.62,.18],[.76,.04],[1.80,.8],[2.6,1.7]];
  for(const [drop,inset] of sections)for(const [x,z] of outline){
    const dx=x-Math.max(-12,Math.min(12,x)),dz=z-Math.max(-24,Math.min(12,z)),length=Math.hypot(dx,dz);
    vertices.push(x-dx/length*inset,floorHeight(z)-drop-.008,z-dz/length*inset);
  }
  const n=outline.length;
  for(let row=0;row<sections.length-1;row++)for(let i=0;i<n;i++){
    const a=row*n+i,b=row*n+(i+1)%n,c=a+n,d=b+n;indices.push(a,b,c,b,d,c);
  }
  const center=vertices.length/3;vertices.push(0,-2.608,-6);
  const base=(sections.length-1)*n;
  for(let i=0;i<n;i++)indices.push(center,base+i,base+(i+1)%n);
  return makeGeometry(vertices,indices);
}
