import { Suspense, useEffect, useMemo, useRef } from 'react';
import { useGLTF } from '@react-three/drei';
import { useFrame } from '@react-three/fiber';
import { CylinderGeometry, Group, Mesh, MeshStandardMaterial, Object3D } from 'three';
import { useCommandCenter } from '../../../lib/store';
import { registerForumActorCollisions } from './walkCollision';
import { extendForumLoader } from './forumGltf';
import { AgentStateSources, ROSTER } from '../agents';
import { useAgentRuntimeStates } from '../shared/agentStatus';
import { STATE } from '../shared/palette';
import { getForumMotion } from './ForumInhabitants';
import { HABITAT_DECOR, ORCHESTRATOR_TABLE_SEAT, stationFor } from './habitatLayout';
import { instanceGardenDecor } from './staticGardenInstances';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { GardenFerns } from './GardenFerns';
import type { GardenDecorPlacement } from './gardenLayout';
import { HabitatNameplate } from './HabitatNameplate';
import { getReduceMotion } from '../../../styles/tokens';
import { createAnalyserEnergy, createMouthState, sampleMouth, stepMouth } from './agentSpeech';
import { avatarCue, NO_CUE } from './agentCue';
import { advanceHabitatAgent, canHabitatWorkReturn, commandHabitatAgent, getHabitatTravel, initializeHabitatAgent } from './habitatMotion';
import { advanceHabitatSocial, resetHabitatSocial, scheduleHabitatSocialPreparation } from './habitatSocial';
import { buildHeroRobot, disposeHeroRobot, HERO_KIT_URL, heroProportions, setHeroEyeMaterial } from './heroRobot';

// Retain the bounded foreground flower population and all authored foliage.
let flowerCount = 0;
const decorPlacements = HABITAT_DECOR.filter(p => p.asset !== 'flowers' || flowerCount++ % 8 === 0);
const craftedDecor = [
  {asset:'table',file:'table-v1.glb',hash:'7b1136d250a1'},
  {asset:'planter',file:'planter-v2.glb',hash:'b7f512af50ce'},
  {asset:'instrument',file:'instrument-v1.glb',hash:'db902c9726a8'},
  {asset:'bench',file:'bench-v1.glb',hash:'9fc2dbcc56f5'},
].map(asset=>({...asset,url:`${import.meta.env.BASE_URL}world/garden-crafted/${asset.file}?v=${asset.hash}`,placements:decorPlacements.filter(p=>p.asset===asset.asset)}));
const fernPlacements=decorPlacements.filter(p=>p.asset==='fern');
const otherPlacements = decorPlacements.filter(p => p.asset!=='fern'&&!craftedDecor.some(asset=>asset.asset===p.asset));
const keepFallback = () => {};

function GardenDecorInstances({scene,placements}:{scene:Object3D;placements:readonly GardenDecorPlacement[]}) {
  const decor=useMemo(()=>{
    const result=instanceGardenDecor(scene,placements);
    result.group.children.forEach(mesh=>{if(['flowers','fern','vine'].includes(mesh.userData.asset))mesh.castShadow=false;});
    return result;
  },[scene,placements]);
  useEffect(()=>()=>decor.dispose(),[decor]);
  return <primitive object={decor.group} dispose={null}/>;
}
function CraftedGardenAsset({url,placements}:{url:string;placements:readonly GardenDecorPlacement[]}) {
  const {scene}=useGLTF(url,false,false,extendForumLoader);
  return <GardenDecorInstances scene={scene} placements={placements}/>;
}
export function HeroGardenDecor(){
  const {scene}=useGLTF(HERO_KIT_URL,false,false,extendForumLoader);
  return <>
    <GardenDecorInstances scene={scene} placements={otherPlacements}/>
    <GardenAssetBoundary onFailure={keepFallback} fallback={<GardenDecorInstances scene={scene} placements={fernPlacements}/>}>
      <Suspense fallback={<GardenDecorInstances scene={scene} placements={fernPlacements}/>}><GardenFerns placements={fernPlacements}/></Suspense>
    </GardenAssetBoundary>
    {craftedDecor.map(asset=>{
      const fallback=<GardenDecorInstances scene={scene} placements={asset.placements}/>;
      return <GardenAssetBoundary key={asset.asset} onFailure={keepFallback} fallback={fallback}>
        <Suspense fallback={fallback}><CraftedGardenAsset url={asset.url} placements={asset.placements}/></Suspense>
      </GardenAssetBoundary>;
    })}
  </>;
}
function HeroRobot({id,index,state,live,selected,hovered,name,conversation,following,directing,onSelect,onHover}:{id:string;index:number;state:keyof typeof STATE;live:boolean;selected:boolean;hovered:boolean;name:string;conversation:boolean;following:string|null;directing:string|null;onSelect:(id:string)=>void;onHover:(id:string|null)=>void}){
  const {scene}=useGLTF(HERO_KIT_URL,false,false,extendForumLoader),root=useRef<Group>(null),station=stationFor(id);
  const owned=useMemo(()=>buildHeroRobot(scene,id),[scene,id]);
  const pose=useMemo(()=>initializeHabitatAgent(id),[id]);
  const speech=useRef({mouth:createMouthState(),analyserEnergy:createAnalyserEnergy()});
  const workReturnAt=useRef(0);
  useEffect(()=>()=>disposeHeroRobot(owned),[owned]);
  useEffect(()=>{setHeroEyeMaterial(owned,id,live,state);
    const userBusy=following===id||directing===id||(conversation&&id==='henry');
    if(!userBusy&&state==='working'&&live&&canHabitatWorkReturn(id))commandHabitatAgent(id,[...station.position],'Returning to workstation',true,'work');
  },[owned,state,live,id,station,conversation,following,directing]);
  useFrame(({clock,camera},dt)=>{
    advanceHabitatAgent(id,dt);const host=root.current;if(!host)return;host.position.set(pose.x,pose.y,pose.z);host.rotation.y=pose.heading;
    // After user travel/Stop clears, retry live work return at most ~1 Hz (not every failed frame).
    const userBusy=following===id||directing===id||(conversation&&id==='henry');
    const travel=getHabitatTravel(id);
    const atDesk=Math.hypot(pose.x-station.position[0],pose.z-station.position[2])<.8;
    if(live&&state==='working'&&!userBusy&&!atDesk&&canHabitatWorkReturn(id)&&travel?.source!=='work'){
      const now=clock.elapsedTime;
      if(now-workReturnAt.current>=1){
        workReturnAt.current=now;
        commandHabitatAgent(id,[...station.position],'Returning to workstation',true,'work');
      }
    }
    const reduced=getReduceMotion(),working=state==='working'&&live&&!getHabitatTravel(id)?.moving&&Math.hypot(pose.x-station.position[0],pose.z-station.position[2])<.8;
    const seated=conversation&&id==='henry'&&!getHabitatTravel(id)?.moving&&Math.hypot(pose.x-ORCHESTRATOR_TABLE_SEAT[0],pose.z-ORCHESTRATOR_TABLE_SEAT[2])<.25;
    owned.seatedBody.visible=seated;if(seated){pose.heading=0;host.rotation.y=0;}
    const engine=id==='henry'?useCommandCenter.getState().voiceEngine:null,speaking=engine?.state==='playing',nowMs=performance.now();
    // Shape comes from the server's word timings on the playback clock; the PCM's own loudness only scales it.
    const heard=speaking?sampleMouth(nowMs):null;let energy=heard?.energy??0,shape=heard?.shape??null;
    if(speaking&&!heard?.active){energy=speech.current.analyserEnergy(engine?.getAnalyser()??null);shape=null;}
    stepMouth(speech.current.mouth,dt,nowMs,{speaking:speaking&&(!!heard?.active||energy>0),shape,energy});
    owned.mouth.scale.set(speech.current.mouth.w,speech.current.mouth.h,.013);
    const cue=conversation&&id==='henry'?avatarCue(engine?.state,clock.elapsedTime,reduced):NO_CUE;
    const stride=(pose.strideDistance??0)*7.5,wave=Math.sin(stride),t=clock.elapsedTime;
    owned.limbs.forEach((limb,i)=>{
      if(!limb)return;limb.visible=!seated;
      let target=pose.walking?wave*(i%2?-1:1)*(i<2?.31:.38):0,spread=0;
      if(working&&i<2){
        const rhythm=reduced?0:Math.sin(t*(station.gesture==='type'?6:station.gesture==='read'?.8:1.6)+i*Math.PI);
        if(station.gesture==='type'){target=-1.11+rhythm*.045;spread=(i===0?-1:1)*.10;}
        else if(station.gesture==='read'){target=-.93+rhythm*.025;spread=(i===0?-1:1)*.15;}
        else if(station.gesture==='tend'){target=-1.04+rhythm*.10;spread=(i===0?-1:1)*.06;}
        else if(station.gesture==='scan'){target=i===0?-.88+rhythm*.07:-.30;}
        else {target=-.75+rhythm*.11;spread=(i===0?1:-1)*.15;}
      }else if(!working&&pose.engaged==='standing'&&i<2){
        // Ambient greeting / inspect pose — visual only, not daemon work.
        const rhythm=reduced?0:Math.sin(t*2.4+i*Math.PI);
        target=-.62+rhythm*.07;spread=(i===0?-1:1)*.12;
      }
      const ease=1-Math.exp(-dt*12);limb.rotation.x+=(target-limb.rotation.x)*ease;limb.rotation.z+=(spread-limb.rotation.z)*ease;
    });
    if(owned.head){let targetYaw=0,targetPitch=working?.14:0;if((selected||hovered)&&!pose.walking){targetYaw=Math.max(-.6,Math.min(.6,Math.atan2(camera.position.x-pose.x,camera.position.z-pose.z)-pose.heading));const horizontal=Math.hypot(camera.position.x-pose.x,camera.position.z-pose.z);targetPitch=Math.max(-.28,Math.min(.24,Math.atan2(camera.position.y-(pose.y+1.55),horizontal)));}else if(!reduced&&!working)targetYaw=Math.sin(t*.3+index)*.10;owned.head.rotation.y+=(targetYaw+cue.yaw-owned.head.rotation.y)*(1-Math.exp(-dt*4));owned.head.rotation.x+=(targetPitch+cue.pitch-owned.head.rotation.x)*(1-Math.exp(-dt*4));owned.head.rotation.z+=(cue.roll-owned.head.rotation.z)*(1-Math.exp(-dt*4));}
    if(reduced)owned.eyes.forEach(eye=>eye.scale.y=cue.eyeScaleY);
    if(!reduced){if(pose.walking)host.position.y+=Math.abs(wave)*.022;else host.position.y+=Math.sin(t*1.0+index)*.006;const blink=Math.sin(t*.72+index*1.37)>.999?.12:1;owned.eyes.forEach(eye=>eye.scale.y=blink*cue.eyeScaleY);}
  });
  const proportions=heroProportions(id);
  return <group ref={root} onClick={e=>{e.stopPropagation();onSelect(id);}} onPointerOver={e=>{e.stopPropagation();onHover(id);}} onPointerOut={()=>onHover(null)}>
    <group scale={proportions as [number,number,number]}><primitive object={owned.object} dispose={null}/></group>
    {selected&&<mesh rotation-x={-Math.PI/2} position={[0,.028,0]}><ringGeometry args={[.46,.49,48]}/><meshBasicMaterial color={station.color} transparent opacity={.7} depthWrite={false}/></mesh>}
    {!conversation&&(selected||hovered)&&<HabitatNameplate position={[0,2.05,0]}><b>{name}</b><span>{live?state:'Ambient presence'}</span></HabitatNameplate>}
  </group>;
}
export function HeroGardenAgents({onSelect,onHover,selected='',hovered='',orchestratorName='Your orchestrator',conversation=false,following=null,directing=null,socialActive=true}:{onSelect:(id:string)=>void;onHover:(id:string|null)=>void;selected?:string;hovered?:string|null;orchestratorName?:string;conversation?:boolean;following?:string|null;directing?:string|null;socialActive?:boolean}){
  const states=useAgentRuntimeStates(),authError=useCommandCenter(s=>s.daemonAuthError);
  const bodies=useMemo(()=>{const geometry=new CylinderGeometry(.33,.33,1.85,16),material=new MeshStandardMaterial();return {geometry,material,meshes:ROSTER.map(a=>{const body=new Mesh(geometry,material);body.name=`${a.id} moving body`;return body;})};},[]);
  useEffect(()=>{
    resetHabitatSocial();
    // Route proving for ambient strolls costs 0.4-1.6 s of main thread in the
    // native WebView. Run it in background slices: synchronously here it landed
    // right after World mounted and blocked the first click out (r34 exit stall).
    scheduleHabitatSocialPreparation();
    const release=registerForumActorCollisions(bodies.meshes);
    return()=>{release();resetHabitatSocial();bodies.geometry.dispose();bodies.material.dispose();};
  },[bodies]);
  // One group scheduler tick before per-agent locomotion (priority -2 < robot frames).
  useFrame((_,dt)=>{
    const liveWorking=new Set<string>();
    for(const state of states){if(!authError&&state.source==='daemon'&&state.hudState==='working')liveWorking.add(state.id);}
    advanceHabitatSocial({dt,active:socialActive,reducedMotion:getReduceMotion(),conversation,liveWorking,following,directing});
  },-2);
  useFrame(()=>{bodies.meshes.forEach((body,i)=>{const pose=getForumMotion(ROSTER[i].id);if(pose){body.position.set(pose.x,pose.y+.925,pose.z);body.updateMatrixWorld(true);}});},-1);

  return <group><AgentStateSources/>{ROSTER.map((a,i)=>{const state=states.find(s=>s.id===a.id);return <HeroRobot key={a.id} id={a.id} index={i} conversation={conversation} following={following} directing={directing} name={a.isHenry?orchestratorName:a.name} selected={selected===a.id} hovered={hovered===a.id} state={state?.hudState??'idle'} live={!authError&&state?.source==='daemon'} onSelect={onSelect} onHover={onHover}/>;})}</group>;
}
export const getHeroPosition=getForumMotion;
