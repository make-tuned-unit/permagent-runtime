import { useEffect, useMemo, useRef } from 'react';
import { useGLTF } from '@react-three/drei';
import { useFrame } from '@react-three/fiber';
import { Group, Mesh, MeshStandardMaterial, Object3D } from 'three';
import { extendForumLoader } from './forumGltf';
import { AgentStateSources, ROSTER } from '../agents';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { STATE } from '../shared/palette';
import { ensureForumMotion, getForumMotion } from './ForumInhabitants';
import { GARDEN_DECOR, GARDEN_AGENT_SEATS } from './gardenLayout';
import { instanceGardenDecor } from './staticGardenInstances';
import { getReduceMotion } from '../../../styles/tokens';

const KIT_URL = `${import.meta.env.BASE_URL}world/garden-hero/hero-kit-v3b.glb?v=e114f28bedaa`;
function cloneAsset(scene: Object3D, name: string) {
  const original=scene.getObjectByName(name);
  if(!original)throw new Error(`Hero kit missing ${name}`);
  const copy=original.clone(true);
  copy.position.set(0,0,0);
  copy.traverse(o=>{if(o instanceof Mesh){o.castShadow=true;o.receiveShadow=true;}});
  return copy;
}
export function HeroGardenDecor() {
  const {scene}=useGLTF(KIT_URL,false,false,extendForumLoader);
  const decor=useMemo(()=>instanceGardenDecor(scene,GARDEN_DECOR),[scene]);
  useEffect(()=>()=>decor.dispose(),[decor]);
  return <primitive object={decor.group} dispose={null} />;
}
function HeroRobot({ id,index,state,onSelect,onHover }: {id:string;index:number;state:keyof typeof STATE;onSelect:(id:string)=>void;onHover:(id:string|null)=>void}) {
  const {scene}=useGLTF(KIT_URL,false,false,extendForumLoader);
  const root=useRef<Group>(null);
  const owned=useMemo(()=>{
    const object=cloneAsset(scene,'robot');const materials:MeshStandardMaterial[]=[];
    object.traverse(o=>{if(o instanceof Mesh){
      const copy=(m:MeshStandardMaterial)=>{const c=m.clone();materials.push(c);return c;};
      o.material=Array.isArray(o.material)?o.material.map(m=>copy(m as MeshStandardMaterial)):copy(o.material as MeshStandardMaterial);
    }});
    return {object,materials};
  },[scene]);
  const pose=useMemo(()=>{
    const [x,y,z]=GARDEN_AGENT_SEATS[index];const m=ensureForumMotion(id,{x,y,z});
    Object.assign(m,{x,y,z,heading:0,walking:false,strideDistance:0,engaged:'none'});return m;
  },[id,index]);
  useEffect(()=>()=>owned.materials.forEach(m=>m.dispose()),[owned]);
  useEffect(()=>{
    owned.materials.forEach(m=>{if(m.name.startsWith('Eyes')){m.color.set(STATE[state]);m.emissive.set(STATE[state]);m.emissiveIntensity=.6;}});
  },[owned,state]);
  useFrame(({clock,camera},dt)=>{
    const host=root.current;if(!host)return;
    host.position.set(pose.x,pose.y,pose.z);
    const head=owned.object.getObjectByName('Head');
    if(head && !getReduceMotion()){
      const target=Math.atan2(camera.position.x-pose.x,camera.position.z-pose.z);
      const angle=Math.max(-.6,Math.min(.6,target));
      head.rotation.y+=(angle-head.rotation.y)*(1-Math.exp(-dt*3));
    }
    if(!getReduceMotion() && state!=='working')host.position.y+=Math.sin(clock.elapsedTime*1.1+index*1.7)*.008;
  });
  return <group ref={root} onClick={e=>{e.stopPropagation();onSelect(id);}} onPointerOver={e=>{e.stopPropagation();onHover(id);}} onPointerOut={()=>onHover(null)}><primitive object={owned.object} dispose={null} /></group>;
}
export function HeroGardenAgents({ onSelect,onHover }: {onSelect:(id:string)=>void;onHover:(id:string|null)=>void}) {
  const states=useAgentRuntimeStates();
  return <group><AgentStateSources />{ROSTER.map((a,i)=><HeroRobot key={a.id} id={a.id} index={i} state={states.find(s=>s.id===a.id)?.hudState??'idle'} onSelect={onSelect} onHover={onHover} />)}</group>;
}
// Shared forum motion positions remain available to Find and E-to-inspect.
export const getHeroPosition = getForumMotion;
