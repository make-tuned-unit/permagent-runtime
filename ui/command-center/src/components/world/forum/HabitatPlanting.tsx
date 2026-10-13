import { ForumBuildScope } from './ForumBuildScope';
import { useForumBuild } from './useForumBuild';
import { Suspense, useEffect } from 'react';
import { useForumTexture as useTexture } from './useForumTexture';
import { BufferGeometry, Color, DoubleSide, Float32BufferAttribute, Group, InstancedMesh, Mesh, MeshStandardMaterial, Object3D, type Texture } from 'three';
import { HABITAT_BEDS, habitatHeight } from './habitatLayout';
import { DetailBuilder, finishDetails, material as makeMaterial } from './habitatGeometry';
import { GardenAssetBoundary } from './GardenAssetBoundary';

/** Readable forms kept explicit so a future placement edit cannot silently
 * collapse the garden into one repeated crown silhouette. */
export const HABITAT_PLANT_FORMS = ['canopy tree', 'broadleaf shrub', 'fern groundcover', 'climbing vine', 'flowering accent'] as const;

/** Solid leaves fill out the atlas foliage at table distance. Static instances
 * share one folded leaf, so density does not add thousands of draw calls. */
export function HabitatPlanting({onReady}:{onReady?:()=>void}) {
  const fallback=<GardenPlanting onReady={onReady}/>;
  return <GardenAssetBoundary onFailure={()=>{}} fallback={fallback}><Suspense fallback={fallback}><TexturedPlanting onReady={onReady}/></Suspense></GardenAssetBoundary>;
}
function TexturedPlanting({onReady}:{onReady?:()=>void}){
  const texture=useTexture(`${import.meta.env.BASE_URL}world/garden-materials/laurel-leaf-v1.webp`,'leaf');
  return <GardenPlanting texture={texture} onReady={onReady}/>;
}
function GardenPlanting({texture,onReady}:{texture?:Texture;onReady?:()=>void}) {
  const built = useForumBuild('buildHabitatPlanting',()=>buildHabitatPlanting(texture),[texture],disposeBuilt);
  useEffect(()=>{if(built)onReady?.();},[built,onReady]);
  if(!built)return null;
  return <primitive object={built.group} dispose={null}/>;
}

function disposeBuilt(built: ReturnType<typeof buildHabitatPlanting> extends Generator<void,infer T> ? T : never) {finishDetails(built.group);built.materials.forEach(m=>m.dispose());
}

export function* buildHabitatPlanting(texture?: Texture) {
  const scope=new ForumBuildScope();let completed=false;
  const material=(...args:Parameters<typeof makeMaterial>)=>scope.own(makeMaterial(...args));
  try {
    yield; let seed = 84319;
    yield; const random = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296; };
    yield; const geometry = scope.own(new BufferGeometry());
    yield; const vertices:number[]=[],indices:number[]=[],uvs:number[]=[];
    yield; for(let row=0;row<=6;row++){yield;
      const t=row/6,width=texture?.29:Math.pow(Math.sin(Math.PI*t),.8)*.21,curve=Math.sin(t*Math.PI)*.10;
      vertices.push(-width,t*.92,curve,0,t*.92,curve+.04*Math.sin(t*Math.PI),width,t*.92,curve);
      uvs.push(0,t,.5,t,1,t);
      if(row<6){const k=row*3;indices.push(k,k+3,k+1,k+1,k+3,k+4,k+1,k+4,k+2,k+2,k+4,k+5);}
    }
    yield; geometry.setAttribute('position', new Float32BufferAttribute(vertices,3));
    yield; geometry.setAttribute('uv',new Float32BufferAttribute(uvs,2));
    yield; geometry.setIndex(indices); yield; geometry.computeVertexNormals();
    // Shared prepared texture remains owned by the loader cache.
    yield; const map=texture;
    yield; const leaf = scope.own(new MeshStandardMaterial({ color: '#ffffff', map:map??null, alphaTest:map?.48:0, side: DoubleSide, roughness: .76 }));
    yield; const matrices: Object3D[] = [], colors: Color[] = [];
    yield; const details = scope.own(new DetailBuilder()), stem = material('#56643a'), petal = material('#e5b575'), pollen = material('#d1b249');
    yield; const addLeaf = (x:number,y:number,z:number,length:number,heading:number,tilt:number) => {
      const o = new Object3D(); o.position.set(x,y,z); o.rotation.set(tilt,heading,(random()-.5)*.5); o.scale.set(length*(.7+random()*.65),length,length); o.updateMatrix(); matrices.push(o);
      const hue=random(),saturation=random(),lightness=random();
      colors.push(map?new Color().setHSL(.12+hue*.10,.08+saturation*.15,.67+lightness*.27):new Color().setHSL(.22+hue*.10,.45+saturation*.25,.12+lightness*.14));
    };
    yield; const clump = (x:number,y:number,z:number,scale:number,flowers:boolean) => {
      for(let branch=0;branch<7;branch++) {
        const a=branch*2.4+random(),r=.18+random()*.24,top=.5+random()*.7;
        const tx=x+Math.cos(a)*r*scale,tz=z+Math.sin(a)*r*scale;
        details.tube([[x,y,z],[tx,y+top*.5*scale,tz],[tx,y+top*scale,tz]],.008*scale,stem);
        for(let j=0;j<6;j++) for(const side of [-1,1]) addLeaf(tx,y+(j/7)*top*scale,tz,(.25+random()*.19)*scale,a+side*1.4,-.95-random()*.6);
        if(flowers&&branch%2===0){const fy=y+top*scale;details.sphere([tx,fy,tz],[.036,.035,.036],pollen);for(let k=0;k<5;k++){const p=k*Math.PI*.4;details.sphere([tx+Math.cos(p)*.05,fy-.012,tz+Math.sin(p)*.05],[.045,.018,.039],petal);}}
      }
    };
    yield; for(const bed of HABITAT_BEDS){yield; for(let i=0,n=bed.groundcover?24:14;i<n;i++) {yield;
      const a=i*2.39996,r=Math.sqrt((i+.3)/n)*.83;
      clump(bed.x+Math.cos(a)*bed.rx*r,habitatHeight(bed.z)+.42,bed.z+Math.sin(a)*bed.rz*r,(bed.groundcover?.8:.70)+random()*.45,i%3===0);
    }}
    // Four deliberately clustered flowering accents hold the arrival frame at
    // eye level. They share the merged detail materials, so this adds no draw
    // submission per flower and keeps the route through the arbor open.
    yield; for(const [x,z] of [[-5.9,4.9],[-.7,4.9],[-6.1,2.0],[-.5,2.0]] as [number,number][]) {yield; clump(x,2.72,z,.82,true);}
    yield; for(const side of [-1,1]) {yield; for(let i=0;i<24;i++){yield; clump(side*(14.4+random()*.5),habitatHeight(12-i*1.55)+.10,12-i*1.55,.8+random()*.45,i%4===0);}}
    // A hanging garden canopy over the commons, keeping the face-to-face view clear.
    yield; for(let i=0;i<58;i++){yield;
      const x=-7+random()*6.4,z=1.15+random()*5.2,y=3.65+random()*.22;
      for(let j=0;j<9;j++){yield; addLeaf(x+(random()-.5)*.6,y,z+(random()-.5)*.6,.35+random()*.2,random()*Math.PI*2,-Math.PI/2+(random()-.5));}
    }
    // Loose, tapering vines replace the repeated straight ladder-like asset.
    // Keep the central face and the routes below the arbor unobstructed.
    yield; for(let i=0;i<18;i++){yield;
      const x=i<9?-7.1+random()*.35:-.75-random()*.25,z=1.2+(i%9)*.60,drop=.7+random()*1.15;
      const points:[number,number,number][]=[];
      for(let j=0;j<=12;j++){yield;
        const t=j/12,px=x+Math.sin(t*4+i)*.11,pz=z+Math.sin(t*3+i)*.08,y=3.72-drop*t;
        points.push([px,y,pz]);
        if(j>1)for(const side of [-1,1]){yield; addLeaf(px,y,pz,(.19+random()*.1)*(1-t*.35),i+side*1.3,1.1+random()*.6);}
      }
      details.tube(points,.007,stem);
    }
    // Terrace-edge ground cover and extra hanging strands frame the vista
    // without narrowing the main circulation lanes.
    yield; const moss=material('#4a5f3a');
    yield; for(const [x,z,scale] of [[-14.2,6,.9],[-14.4,0,.85],[-14.1,-8,.95],[14.2,4,.88],[14.3,-2,.92],[14.1,-10,.86],[-6,-26,.78],[6,-26,.82]] as [number,number,number][]){yield;
      for(let i=0;i<14;i++){yield;
        const a=i*2.4+random(),r=.22+random()*.28;
        const px=x+Math.cos(a)*r, pz=z+Math.sin(a)*r*scale;
        clump(px,habitatHeight(pz)+.38,pz,.55+random()*.25,i%2===0);
        if(i%3===0)for(let j=0;j<4;j++){yield; addLeaf(px,habitatHeight(pz)+.55+j*.08,pz,.16+random()*.08,a+j,1.35+random()*.35);}
      }
      details.sphere([x,habitatHeight(z)+.12,z],[.55*scale,.08,.48*scale],moss);
    }
    yield; for(let i=0;i<12;i++){yield;
      const side=i%2===0?-1:1,x=side*(14.35+random()*.15),z=-4-i*2.1,drop=.55+random()*.85;
      const points:[number,number,number][]=[[x,3.55,z]];
      for(let j=1;j<=8;j++){yield;
        const t=j/8,px=x+Math.sin(t*5+i)*.09,pz=z+Math.sin(t*2.5)*.06,y=3.55-drop*t;
        points.push([px,y,pz]);
        if(j>2)addLeaf(px,y,pz,.14+random()*.07,i+t,1.05+random()*.5);
      }
      details.tube(points,.006,stem);
    }
    yield; const mesh = scope.own(new InstancedMesh(geometry,leaf,matrices.length));yield; mesh.name='Layered living garden';yield; mesh.castShadow=true;yield; mesh.receiveShadow=true;
    yield; for(let i=0;i<matrices.length;i++){mesh.setMatrixAt(i,matrices[i].matrix);mesh.setColorAt(i,colors[i]);if(i%256===0)yield;}yield; mesh.computeBoundingSphere();
    yield; const group = new Group();yield; group.name='Five-form planted habitat';yield; group.userData.readableForms=[...HABITAT_PLANT_FORMS];yield; group.add(mesh,(yield* details.finishSteps('Garden stems, vines and flowering accents')));scope.defer(()=>finishDetails(group));yield; group.traverse(o=>{if(o instanceof Mesh)o.castShadow=false;});
    completed=true; return {group,geometry,map,materials:[leaf,stem,petal,pollen,moss]};

  } finally {if(!completed)scope.dispose();}
}
