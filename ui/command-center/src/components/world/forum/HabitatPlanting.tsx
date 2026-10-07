import { Suspense, useEffect, useMemo } from 'react';
import { useTexture } from '@react-three/drei';
import { BufferGeometry, Color, DoubleSide, Float32BufferAttribute, Group, InstancedMesh, Mesh, MeshStandardMaterial, Object3D, SRGBColorSpace, type Texture } from 'three';
import { HABITAT_BEDS, habitatHeight } from './habitatLayout';
import { DetailBuilder, finishDetails, material } from './habitatGeometry';
import { GardenAssetBoundary } from './GardenAssetBoundary';

/** Readable forms kept explicit so a future placement edit cannot silently
 * collapse the garden into one repeated crown silhouette. */
export const HABITAT_PLANT_FORMS = ['canopy tree', 'broadleaf shrub', 'fern groundcover', 'climbing vine', 'flowering accent'] as const;

/** Solid leaves fill out the atlas foliage at table distance. Static instances
 * share one folded leaf, so density does not add thousands of draw calls. */
export function HabitatPlanting() {
  const fallback=<GardenPlanting/>;
  return <GardenAssetBoundary onFailure={()=>{}} fallback={fallback}><Suspense fallback={fallback}><TexturedPlanting/></Suspense></GardenAssetBoundary>;
}
function TexturedPlanting(){
  const texture=useTexture(`${import.meta.env.BASE_URL}world/garden-materials/laurel-leaf-v1.webp`);
  return <GardenPlanting texture={texture}/>;
}
function GardenPlanting({texture}:{texture?:Texture}) {
  const built = useMemo(() => {
    let seed = 84319;
    const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296; };
    const geometry = new BufferGeometry();
    const vertices:number[]=[],indices:number[]=[],uvs:number[]=[];
    for(let row=0;row<=6;row++){
      const t=row/6,width=texture?.29:Math.pow(Math.sin(Math.PI*t),.8)*.21,curve=Math.sin(t*Math.PI)*.10;
      vertices.push(-width,t*.92,curve,0,t*.92,curve+.04*Math.sin(t*Math.PI),width,t*.92,curve);
      uvs.push(0,t,.5,t,1,t);
      if(row<6){const k=row*3;indices.push(k,k+3,k+1,k+1,k+3,k+4,k+1,k+4,k+2,k+2,k+4,k+5);}
    }
    geometry.setAttribute('position', new Float32BufferAttribute(vertices,3));
    geometry.setAttribute('uv',new Float32BufferAttribute(uvs,2));
    geometry.setIndex(indices); geometry.computeVertexNormals();
    const map=texture?.clone();if(map){map.colorSpace=SRGBColorSpace;map.anisotropy=4;map.needsUpdate=true;}
    const leaf = new MeshStandardMaterial({ color: '#ffffff', map:map??null, alphaTest:map?.48:0, side: DoubleSide, roughness: .76 });
    const matrices: Object3D[] = [], colors: Color[] = [];
    const details = new DetailBuilder(), stem = material('#56643a'), petal = material('#e5b575'), pollen = material('#d1b249');
    const addLeaf = (x:number,y:number,z:number,length:number,heading:number,tilt:number) => {
      const o = new Object3D(); o.position.set(x,y,z); o.rotation.set(tilt,heading,(random()-.5)*.5); o.scale.set(length*(.7+random()*.65),length,length); o.updateMatrix(); matrices.push(o);
      const hue=random(),saturation=random(),lightness=random();
      colors.push(map?new Color().setHSL(.12+hue*.10,.08+saturation*.15,.67+lightness*.27):new Color().setHSL(.22+hue*.10,.45+saturation*.25,.12+lightness*.14));
    };
    const clump = (x:number,y:number,z:number,scale:number,flowers:boolean) => {
      for(let branch=0;branch<7;branch++) {
        const a=branch*2.4+random(),r=.18+random()*.24,top=.5+random()*.7;
        const tx=x+Math.cos(a)*r*scale,tz=z+Math.sin(a)*r*scale;
        details.tube([[x,y,z],[tx,y+top*.5*scale,tz],[tx,y+top*scale,tz]],.008*scale,stem);
        for(let j=0;j<6;j++) for(const side of [-1,1]) addLeaf(tx,y+(j/7)*top*scale,tz,(.25+random()*.19)*scale,a+side*1.4,-.95-random()*.6);
        if(flowers&&branch%2===0){const fy=y+top*scale;details.sphere([tx,fy,tz],[.036,.035,.036],pollen);for(let k=0;k<5;k++){const p=k*Math.PI*.4;details.sphere([tx+Math.cos(p)*.05,fy-.012,tz+Math.sin(p)*.05],[.045,.018,.039],petal);}}
      }
    };
    for(const bed of HABITAT_BEDS)for(let i=0,n=bed.groundcover?24:14;i<n;i++) {
      const a=i*2.39996,r=Math.sqrt((i+.3)/n)*.83;
      clump(bed.x+Math.cos(a)*bed.rx*r,habitatHeight(bed.z)+.42,bed.z+Math.sin(a)*bed.rz*r,(bed.groundcover?.8:.70)+random()*.45,i%3===0);
    }
    // Four deliberately clustered flowering accents hold the arrival frame at
    // eye level. They share the merged detail materials, so this adds no draw
    // submission per flower and keeps the route through the arbor open.
    for(const [x,z] of [[-5.9,4.9],[-.7,4.9],[-6.1,2.0],[-.5,2.0]] as [number,number][]) clump(x,2.72,z,.82,true);
    for(const side of [-1,1]) for(let i=0;i<24;i++)clump(side*(14.4+random()*.5),habitatHeight(12-i*1.55)+.10,12-i*1.55,.8+random()*.45,i%4===0);
    // A hanging garden canopy over the commons, keeping the face-to-face view clear.
    for(let i=0;i<58;i++){
      const x=-7+random()*6.4,z=1.15+random()*5.2,y=3.65+random()*.22;
      for(let j=0;j<9;j++)addLeaf(x+(random()-.5)*.6,y,z+(random()-.5)*.6,.35+random()*.2,random()*Math.PI*2,-Math.PI/2+(random()-.5));
    }
    // Loose, tapering vines replace the repeated straight ladder-like asset.
    // Keep the central face and the routes below the arbor unobstructed.
    for(let i=0;i<18;i++){
      const x=i<9?-7.1+random()*.35:-.75-random()*.25,z=1.2+(i%9)*.60,drop=.7+random()*1.15;
      const points:[number,number,number][]=[];
      for(let j=0;j<=12;j++){
        const t=j/12,px=x+Math.sin(t*4+i)*.11,pz=z+Math.sin(t*3+i)*.08,y=3.72-drop*t;
        points.push([px,y,pz]);
        if(j>1)for(const side of [-1,1])addLeaf(px,y,pz,(.19+random()*.1)*(1-t*.35),i+side*1.3,1.1+random()*.6);
      }
      details.tube(points,.007,stem);
    }
    const mesh = new InstancedMesh(geometry,leaf,matrices.length);mesh.name='Layered living garden';mesh.castShadow=true;mesh.receiveShadow=true;
    matrices.forEach((o,i)=>{mesh.setMatrixAt(i,o.matrix);mesh.setColorAt(i,colors[i]);});mesh.computeBoundingSphere();
    const group = new Group();group.name='Five-form planted habitat';group.userData.readableForms=[...HABITAT_PLANT_FORMS];group.add(mesh,details.finish('Garden stems, vines and flowering accents'));group.traverse(o=>{if(o instanceof Mesh)o.castShadow=false;});
    return {group,geometry,map,materials:[leaf,stem,petal,pollen]};
  },[texture]);
  useEffect(()=>()=>{finishDetails(built.group);built.materials.forEach(m=>m.dispose());built.map?.dispose();},[built]);
  return <primitive object={built.group} dispose={null}/>;
}
