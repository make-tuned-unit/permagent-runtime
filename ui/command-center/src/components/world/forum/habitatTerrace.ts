import { BufferGeometry, Float32BufferAttribute } from 'three';

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

function makeGeometry(vertices:number[],indices:number[]){
  const geometry=new BufferGeometry();geometry.setAttribute('position',new Float32BufferAttribute(vertices,3));
  geometry.setAttribute('uv',new Float32BufferAttribute(vertices.flatMap((v,i)=>i%3===1?[]:[v]),2));
  geometry.setIndex(indices);geometry.computeVertexNormals();return geometry;
}
/** Clip each metre cell against the true rounded border. All triangles remain
 * on the three connected planes; no stair steps or unsupported corner tiles. */
export function terraceSurfaceGeometry(omitPool=false){
  const outline=terracePerimeter(),vertices:number[]=[],indices:number[]=[];
  for(let z=-28;z<16;z++)for(let x=-16;x<16;x++){
    if(omitPool&&x>=2&&x<6&&z>=-5&&z<1)continue;
    let polygon:Point2[]=[[x,z],[x,z+1],[x+1,z+1],[x+1,z]];
    for(let i=0;i<outline.length&&polygon.length;i++){
      const a=outline[i],b=outline[(i+1)%outline.length],cross=(p:Point2)=>(b[0]-a[0])*(p[1]-a[1])-(b[1]-a[1])*(p[0]-a[0]);
      const next:Point2[]=[];
      for(let j=0;j<polygon.length;j++){
        const p=polygon[j],q=polygon[(j+1)%polygon.length],dp=cross(p),dq=cross(q);
        if(dp>=-1e-8)next.push(p);
        if((dp>=0)!==(dq>=0)){const t=dp/(dp-dq);next.push([p[0]+(q[0]-p[0])*t,p[1]+(q[1]-p[1])*t]);}
      }polygon=next;
    }
    if(polygon.length<3)continue;
    const base=vertices.length/3;polygon.forEach(([px,pz])=>vertices.push(px,floorHeight(pz),pz));
    for(let i=1;i<polygon.length-1;i++)indices.push(base,base+i,base+i+1);
  }return makeGeometry(vertices,indices);
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
  // A closed underside stays entirely beneath the walking surface.
  const center=vertices.length/3;vertices.push(0,-2.608,-6);
  const base=(sections.length-1)*n;
  for(let i=0;i<n;i++)indices.push(center,base+i,base+(i+1)%n);
  return makeGeometry(vertices,indices);
}
