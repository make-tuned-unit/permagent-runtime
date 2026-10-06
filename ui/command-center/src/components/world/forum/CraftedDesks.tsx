import { Suspense, useEffect, useMemo } from 'react';
import { useGLTF } from '@react-three/drei';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { DetailBuilder, finishDetails, material } from './habitatGeometry';
import { extendForumLoader } from './forumGltf';
import { instanceGardenDecor } from './staticGardenInstances';
import { STATION_DESKS } from './habitatLayout';

function DeskModels(){
  const {scene}=useGLTF(`${import.meta.env.BASE_URL}world/garden-crafted/desk-v1.glb?v=6c23b7e65f98`,false,false,extendForumLoader);
  const owned=useMemo(()=>instanceGardenDecor(scene,STATION_DESKS),[scene]);
  useEffect(()=>()=>owned.dispose(),[owned]);
  return <primitive object={owned.group} dispose={null}/>;
}
function DeskFallback(){
  const owned=useMemo(()=>{
    const b=new DetailBuilder(),wood=material('#68462e',.73),bronze=material('#9f7950',.35,.72);
    for(const p of STATION_DESKS){
      const [x,y,z]=p.position;
      b.box([x,y+.84,z],[1.9,.1,.72],wood);
      for(const dx of [-.76,.76]){
        b.box([x+dx,y+.4,z],[.075,.8,.55],bronze);
        b.box([x+dx,y+.05,z],[.24,.09,.62],bronze);
      }
    }
    return {group:b.finish('Fallback work desks'),materials:[wood,bronze]};
  },[]);
  useEffect(()=>()=>{finishDetails(owned.group);owned.materials.forEach(m=>m.dispose());},[owned]);
  return <primitive object={owned.group} dispose={null}/>;
}
export function CraftedDesks(){
  const fallback=<DeskFallback/>;
  return <GardenAssetBoundary onFailure={()=>{}} fallback={fallback}><Suspense fallback={fallback}><DeskModels/></Suspense></GardenAssetBoundary>;
}
