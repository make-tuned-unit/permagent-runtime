import { Suspense, useEffect, useMemo, useRef } from 'react';
import { Html, useGLTF } from '@react-three/drei';
import { useFrame } from '@react-three/fiber';
import { BufferGeometry, CylinderGeometry, Group, Material, Mesh, MeshBasicMaterial, MeshPhysicalMaterial, MeshStandardMaterial, Object3D, SphereGeometry } from 'three';
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
import { getReduceMotion } from '../../../styles/tokens';
import { createAgentWardrobe } from './agentWardrobe';
import { heroNode, replaceFixedHeroMouth } from './heroRig';
import { speechOpening, smoothSpeech } from './agentSpeech';
import { DetailBuilder, material, finishDetails } from './habitatGeometry';
import { advanceHabitatAgent, canHabitatWorkReturn, commandHabitatAgent, getHabitatTravel, initializeHabitatAgent } from './habitatMotion';
import { advanceHabitatSocial, prepareHabitatSocial, resetHabitatSocial } from './habitatSocial';

const KIT_URL = `${import.meta.env.BASE_URL}world/garden-hero/hero-kit-v3b.glb?v=e114f28bedaa`;
function cloneAsset(scene:Object3D,name:string){const original=scene.getObjectByName(name);if(!original)throw new Error(`Hero kit missing ${name}`);const copy=original.clone(true);copy.position.set(0,0,0);copy.traverse(o=>{if(o instanceof Mesh){o.castShadow=true;o.receiveShadow=true;}});return copy;}
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
  const {scene}=useGLTF(KIT_URL,false,false,extendForumLoader);
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
  const {scene}=useGLTF(KIT_URL,false,false,extendForumLoader),root=useRef<Group>(null),station=stationFor(id);
  const owned=useMemo(()=>{
    const object=cloneAsset(scene,'robot'),materials:Material[]=[],geometries:BufferGeometry[]=[],wardrobe=createAgentWardrobe(id);
    object.traverse(o=>{if(o instanceof Mesh){const sourceMaterials=Array.isArray(o.material)?o.material:[o.material];o.castShadow=sourceMaterials.some(m=>m.name.startsWith('Ceramic'));const copy=(m:MeshStandardMaterial)=>{const c=new MeshPhysicalMaterial({name:m.name,color:m.color,map:m.map,normalMap:m.normalMap,roughnessMap:m.roughnessMap,metalnessMap:m.metalnessMap,roughness:m.roughness,metalness:m.metalness,transparent:m.transparent,opacity:m.opacity,alphaTest:m.alphaTest,side:m.side,depthWrite:m.depthWrite,depthTest:m.depthTest});if(c.name.startsWith('Ceramic')){c.color.set(id==='strix'?'#a6b0ac':id==='forecaster'?'#d3dfe0':id==='picker'?'#e2d4b5':'#eee5d0');c.roughness=.28;c.clearcoat=.34;c.clearcoatRoughness=.22;c.ior=1.46;}else if(c.name.startsWith('Faceplate')){c.color.set('#07141a');c.roughness=.24;c.clearcoat=.18;c.clearcoatRoughness=.3;}c.needsUpdate=true;materials.push(c);return c;};o.material=Array.isArray(o.material)?o.material.map(m=>copy(m as MeshStandardMaterial)):copy(o.material as MeshStandardMaterial);}});
    const head=heroNode(object,'Head');object.add(wardrobe.body);head?.add(wardrobe.headwear);
    if(head){const mouthGeometry=replaceFixedHeroMouth(head);if(mouthGeometry)geometries.push(mouthGeometry);}
    const mouth=new Mesh(new SphereGeometry(1,24,12),new MeshBasicMaterial({color:'#b3e4d5'}));mouth.name='Voice-synchronized mouth';mouth.position.set(0,-.085,.307);mouth.scale.set(.055,.004,.013);head?.add(mouth);
    const seatBuilder=new DetailBuilder(),seatWood=material('#73543b'),seatCeramic=material('#e5decc',.4),seatJoint=material('#354345',.4,.3);
    for(const x of [-.125,.125]){seatBuilder.tube([[x,.68,0],[x,.50,.31]],.075,seatCeramic);seatBuilder.sphere([x,.50,.31],[.08,.075,.08],seatJoint);seatBuilder.tube([[x,.50,.31],[x,.12,.37]],.068,seatCeramic);seatBuilder.box([x,.065,.43],[.18,.10,.28],seatCeramic);}
    // Bent elbows and palms resting on the table make the close conversation
    // pose distinct from the straight, shoulder-driven walking rig.
    for(const side of [-1,1]){
      seatBuilder.sphere([side*.29,1.065,0],[.10,.10,.10],seatJoint);
      seatBuilder.tube([[side*.29,1.06,0],[side*.37,.85,.20]],.072,seatCeramic);
      seatBuilder.sphere([side*.37,.85,.20],[.082,.078,.082],seatJoint);
      seatBuilder.tube([[side*.37,.85,.20],[side*.30,1.055,.71]],.071,seatCeramic);
      seatBuilder.sphere([side*.30,1.055,.71],[.072,.045,.055],seatJoint);
      seatBuilder.sphere([side*.30,1.067,.78],[.103,.041,.10],seatCeramic);
      for(let finger=0;finger<4;finger++)seatBuilder.sphere([side*.30+(finger-1.5)*.045,1.06,.867],[.019,.027,.064],seatCeramic);
      seatBuilder.sphere([side*.411,1.058,.78],[.034,.03,.065],seatCeramic);
    }
    seatBuilder.cylinder([0,.61,-.08],.34,.09,seatWood);seatBuilder.box([0,.92,-.30],[.61,.53,.065],seatWood);for(const x of [-.24,.24])for(const z of [-.24,.15])seatBuilder.cylinder([x,.29,z],.028,.58,seatJoint);
    const seatedBody=seatBuilder.finish('Seated pose and chair');seatedBody.visible=false;object.add(seatedBody);
    const limbs=['Arm.L','Arm.R','Leg.L','Leg.R'].map(n=>heroNode(object,n));
    const eyes:Mesh[]=[];head?.traverse(o=>{if(o instanceof Mesh&&(o.material as MeshStandardMaterial).name?.startsWith('Eyes'))eyes.push(o);});
    return {object,materials,geometries,wardrobe,head,limbs,eyes,mouth,seatedBody,seatMaterials:[seatWood,seatCeramic,seatJoint]};
  },[scene,id]);
  const pose=useMemo(()=>initializeHabitatAgent(id),[id]);
  const speech=useRef({opening:0,bins:new Uint8Array(32)});
  const workReturnAt=useRef(0);
  useEffect(()=>()=>{owned.materials.forEach(m=>m.dispose());owned.geometries.forEach(g=>g.dispose());owned.wardrobe.materials.forEach(m=>m.dispose());finishDetails(owned.wardrobe.body);finishDetails(owned.wardrobe.headwear);finishDetails(owned.seatedBody);owned.seatMaterials.forEach(m=>m.dispose());owned.mouth.geometry.dispose();(owned.mouth.material as MeshBasicMaterial).dispose();},[owned]);
  useEffect(()=>{owned.materials.forEach(m=>{if(m instanceof MeshStandardMaterial&&m.name.startsWith('Eyes')){const eyeColor=id==='henry'?'#a0dbb9':station.color;m.color.set(eyeColor);m.emissive.set(eyeColor);m.emissiveIntensity=live&&state==='working'?.8:.6;}});
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
    const engine=id==='henry'?useCommandCenter.getState().voiceEngine:null,analyser=engine?.state==='playing'?engine.getAnalyser():null;
    if(analyser){if(speech.current.bins.length!==analyser.frequencyBinCount)speech.current.bins=new Uint8Array(analyser.frequencyBinCount);analyser.getByteFrequencyData(speech.current.bins);}
    const opening=speechOpening(engine?.state??'idle',analyser?speech.current.bins:null);speech.current.opening=smoothSpeech(speech.current.opening,opening,dt);
    owned.mouth.scale.set(.055+speech.current.opening*.005,.004+speech.current.opening*.045,.013);
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
    if(owned.head){let targetYaw=0,targetPitch=working?.14:0;if((selected||hovered)&&!pose.walking){targetYaw=Math.max(-.6,Math.min(.6,Math.atan2(camera.position.x-pose.x,camera.position.z-pose.z)-pose.heading));const horizontal=Math.hypot(camera.position.x-pose.x,camera.position.z-pose.z);targetPitch=Math.max(-.28,Math.min(.24,Math.atan2(camera.position.y-(pose.y+1.55),horizontal)));}else if(!reduced&&!working)targetYaw=Math.sin(t*.3+index)*.10;owned.head.rotation.y+=(targetYaw-owned.head.rotation.y)*(1-Math.exp(-dt*4));owned.head.rotation.x+=(targetPitch-owned.head.rotation.x)*(1-Math.exp(-dt*4));}
    if(reduced)owned.eyes.forEach(eye=>eye.scale.y=1);
    if(!reduced){if(pose.walking)host.position.y+=Math.abs(wave)*.022;else host.position.y+=Math.sin(t*1.0+index)*.006;const blink=Math.sin(t*.72+index*1.37)>.999?.12:1;owned.eyes.forEach(eye=>eye.scale.y=blink);}
  });
  const proportions=id==='strix'?[1.1,1.04,1.05]:id==='reader'?[.94,1.02,.96]:id==='librarian'?[1,1.07,1]:id==='picker'?[1,.95,1]:[1,1,1];
  return <group ref={root} onClick={e=>{e.stopPropagation();onSelect(id);}} onPointerOver={e=>{e.stopPropagation();onHover(id);}} onPointerOut={()=>onHover(null)}>
    <group scale={proportions as [number,number,number]}><primitive object={owned.object} dispose={null}/></group>
    {selected&&<mesh rotation-x={-Math.PI/2} position={[0,.028,0]}><ringGeometry args={[.46,.49,48]}/><meshBasicMaterial color={station.color} transparent opacity={.7} depthWrite={false}/></mesh>}
    {!conversation&&(selected||hovered)&&<Html position={[0,2.05,0]} center distanceFactor={8} zIndexRange={[20,0]}><div className="habitat-nameplate"><b>{name}</b><span>{live?state:'Ambient presence'}</span></div></Html>}
  </group>;
}
export function HeroGardenAgents({onSelect,onHover,selected='',hovered='',orchestratorName='Your orchestrator',conversation=false,following=null,directing=null,socialActive=true}:{onSelect:(id:string)=>void;onHover:(id:string|null)=>void;selected?:string;hovered?:string|null;orchestratorName?:string;conversation?:boolean;following?:string|null;directing?:string|null;socialActive?:boolean}){
  const states=useAgentRuntimeStates(),authError=useCommandCenter(s=>s.daemonAuthError);
  const bodies=useMemo(()=>{const geometry=new CylinderGeometry(.33,.33,1.85,16),material=new MeshStandardMaterial();return {geometry,material,meshes:ROSTER.map(a=>{const body=new Mesh(geometry,material);body.name=`${a.id} moving body`;return body;})};},[]);
  useEffect(()=>{
    resetHabitatSocial();
    prepareHabitatSocial();
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
