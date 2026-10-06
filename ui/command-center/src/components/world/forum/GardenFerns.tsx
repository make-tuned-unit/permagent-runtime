import { useEffect, useMemo } from 'react';
import { useTexture } from '@react-three/drei';
import { BufferGeometry, Color, DoubleSide, Float32BufferAttribute, InstancedMesh, MeshStandardMaterial, Object3D, SRGBColorSpace } from 'three';
import { PLANT_BASE_Y, type GardenDecorPlacement } from './gardenLayout';

/** Curved, individually masked fronds keep the fern's open silhouette. One
 * shared geometry and material cover the entire garden, including planters. */
export function GardenFerns({placements}:{placements:readonly GardenDecorPlacement[]}){
  const source=useTexture(`${import.meta.env.BASE_URL}world/garden-materials/fern-frond-v1.webp`);
  const owned=useMemo(()=>{
    const map=source.clone();map.colorSpace=SRGBColorSpace;map.anisotropy=4;map.needsUpdate=true;
    const geometry=new BufferGeometry(),positions:number[]=[],uvs:number[]=[],indices:number[]=[];
    for(let row=0;row<=12;row++){
      const t=row/12,y=.88*t-.39*t*t*t,z=.57*t*t;
      for(const side of [-1,0,1]){positions.push(side*.27,y+(side===0?.035*Math.sin(t*Math.PI):0),z);uvs.push((side+1)/2,t);}
      if(row<12){const k=row*3;indices.push(k,k+3,k+1,k+1,k+3,k+4,k+1,k+4,k+2,k+2,k+4,k+5);}
    }
    geometry.setAttribute('position',new Float32BufferAttribute(positions,3));geometry.setAttribute('uv',new Float32BufferAttribute(uvs,2));geometry.setIndex(indices);geometry.computeVertexNormals();
    const material=new MeshStandardMaterial({map,alphaTest:.6,side:DoubleSide,roughness:.8});
    const mesh=new InstancedMesh(geometry,material,placements.length*15),object=new Object3D();mesh.name='Layered fern fronds';mesh.receiveShadow=true;mesh.castShadow=false;
    placements.forEach((p,i)=>{
      const base=p.supportY??p.position[1]+PLANT_BASE_Y.fern*p.scale;
      for(let j=0;j<15;j++){
        const inner=j>=10,heading=p.heading+j*2.399963,variation=.88+((i*17+j*7)%13)/50;
        object.position.set(p.position[0]+Math.sin(heading)*.025,base-.006,p.position[2]+Math.cos(heading)*.025);
        object.rotation.set(inner?-.35:0,heading,0);object.scale.setScalar(p.scale*variation*(inner?.65:1.05));object.updateMatrix();
        const index=i*15+j;mesh.setMatrixAt(index,object.matrix);mesh.setColorAt(index,new Color().setHSL(.17,.13,.73+((i+j)%5)*.04));
      }
    });
    mesh.computeBoundingSphere();mesh.computeBoundingBox();return {mesh,geometry,material,map};
  },[source,placements]);
  useEffect(()=>()=>{owned.mesh.dispose();owned.geometry.dispose();owned.material.dispose();owned.map.dispose();},[owned]);
  return <primitive object={owned.mesh} dispose={null}/>;
}
