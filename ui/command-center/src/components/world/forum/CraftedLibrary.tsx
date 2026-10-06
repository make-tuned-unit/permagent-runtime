import { Suspense, useEffect, useMemo } from 'react';
import { useGLTF } from '@react-three/drei';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { DetailBuilder, finishDetails, material } from './habitatGeometry';
import { extendForumLoader } from './forumGltf';
import { instanceGardenDecor } from './staticGardenInstances';
import { ARCHIVE_DECOR } from './habitatLayout';

const assets=[{name:'bookcase',hash:'8671cc46e284'},{name:'column',hash:'cc763e099a6d'}];
function LibraryModel({name,hash}:{name:string;hash:string}){
  const {scene}=useGLTF(`${import.meta.env.BASE_URL}world/garden-crafted/${name}-v1.glb?v=${hash}`,false,false,extendForumLoader);
  const owned=useMemo(()=>instanceGardenDecor(scene,ARCHIVE_DECOR.filter(p=>p.asset===name)),[scene,name]);
  useEffect(()=>()=>owned.dispose(),[owned]);
  return <primitive object={owned.group} dispose={null}/>;
}
function LibraryFallback({name}:{name:string}){
  const owned=useMemo(()=>{
    const b=new DetailBuilder(),wood=material('#705239'),stone=material('#d4cbb5'),bronze=material('#92724d',.4,.5),books=[material('#526967'),material('#b8aa88'),material('#785848')];
    for(const p of ARCHIVE_DECOR.filter(p=>p.asset===name)){
      const [x,y,z]=p.position;
      if(name==='column'){
        b.cylinder([x,y+1.8,z],.18,3.6,stone);
        for(const h of [.12,3.5])b.box([x,y+h,z],[.54,.20,.54],stone);
        for(const h of [.28,3.34])b.ring([x,y+h,z],.20,.024,bronze);
      }else{
        b.box([x,y+1.4,z-.20],[2.55,2.8,.1],wood);
        for(const dx of [-1.23,1.23])b.box([x+dx,y+1.4,z],[.09,2.8,.5],wood);
        for(let row=0;row<5;row++){
          b.box([x,y+.08+row*.66,z],[2.55,.07,.5],wood);
          if(row===4)continue;
          for(let j=0;j<12;j++){const h=.32+((j*3+row)%5)*.05;b.box([x-1.08+j*.19,y+.14+row*.66+h/2,z],[.14,h,.34],books[(j+row)%3]);}
        }
      }
    }
    return {group:b.finish(`Fallback ${name}`),materials:[wood,stone,bronze,...books]};
  },[name]);
  useEffect(()=>()=>{finishDetails(owned.group);owned.materials.forEach(m=>m.dispose());},[owned]);
  return <primitive object={owned.group} dispose={null}/>;
}
export function CraftedLibrary(){return <>{assets.map(asset=>{
  const fallback=<LibraryFallback name={asset.name}/>;
  return <GardenAssetBoundary key={asset.name} onFailure={()=>{}} fallback={fallback}><Suspense fallback={fallback}><LibraryModel {...asset}/></Suspense></GardenAssetBoundary>;
})}</>;}
