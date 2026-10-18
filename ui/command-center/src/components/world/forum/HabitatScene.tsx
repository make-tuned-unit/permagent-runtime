import { useForumBuild } from './useForumBuild';
import { Suspense, lazy, useEffect, useMemo, useRef } from 'react';
import { OrbitControls } from '@react-three/drei';
import { useFrame, useThree, type ThreeEvent } from '@react-three/fiber';
import { Mesh, MeshStandardMaterial, Object3D, PerspectiveCamera, Vector3 } from 'three';
import { loadForumModule, measureForumStartup } from './forumStartup';
const HabitatArchitecture=lazy(()=>loadForumModule('architecture module',async()=>({default:(await import('./HabitatArchitecture')).HabitatArchitecture})));
const HabitatPlanting=lazy(()=>loadForumModule('planting module',async()=>({default:(await import('./HabitatPlanting')).HabitatPlanting})));
import { HABITAT_ARRIVAL, habitatOverviewCamera, TABLE_CONVERSATION_CAMERA, habitatCollisionSource, habitatElevation, terrainGeometry, type Point3 } from './habitatLayout';
import { prepareForumCollision } from './walkCollision';
import { getForumMotion } from './ForumInhabitants';
import { commandHabitatAgent } from './habitatMotion';
import { HabitatSky } from './HabitatSky';
import { HabitatWater } from './HabitatWater';
import type { ForumDistrict } from './districts';
import { useForumReducedMotion } from './useForumReducedMotion';
import { useForumActivity } from './ForumActivity';
import { HABITAT_SUN_DIR } from './habitatVista';

function* prepareHabitat(){
  const ground=measureForumStartup('terrain',terrainGeometry);let cleanup:(()=>void)|undefined,completed=false;
  try {
    yield;
    const source=measureForumStartup('collision source',habitatCollisionSource);
    try {yield;cleanup=yield* prepareForumCollision(source);}
    finally {const materials=new Set<MeshStandardMaterial>();source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();for(const m of Array.isArray(o.material)?o.material:[o.material])materials.add(m as MeshStandardMaterial);}});materials.forEach(m=>m.dispose());}
    completed=true;return{ground,dispose(){ground.dispose();cleanup?.();}};
  }finally{if(!completed){ground.dispose();cleanup?.();}}
}
export function HabitatScene({directing,onDirected,onReady}:{directing:string|null;onDirected:(point:Point3,ok:boolean)=>void;onReady?:()=>void}){
  const built=useForumBuild('HabitatScene terrain / collision',prepareHabitat,[],value=>value.dispose());
  // Stage readiness means the walkable ground and its collision data exist.
  // Architecture and planting are progressive details and report their own
  // asset health; waiting for both here kept the full-screen startup overlay
  // over a usable scene while those optional builders continued.
  useEffect(()=>{if(built)onReady?.();},[built,onReady]);
  const direct=(e:ThreeEvent<MouseEvent>)=>{if(!directing)return;e.stopPropagation();const p:Point3=[e.point.x,habitatElevation(e.point.x,e.point.z),e.point.z];onDirected(p,commandHabitatAgent(directing,p));};
  return <group onClick={direct}>
    {built&&<mesh geometry={built.ground} position={[0,-.06,0]} receiveShadow><meshStandardMaterial color="#ac9f87" roughness={1}/></mesh>}
    <Suspense fallback={null}><HabitatArchitecture/></Suspense><Suspense fallback={null}><HabitatPlanting/></Suspense><HabitatWater/>
  </group>;
}
export function HabitatLighting({appearance,skyAttempt=0}:{appearance:'day'|'night';skyAttempt?:number}){
  const day=appearance==='day';
  const sunTarget=useMemo(()=>{const target=new Object3D();target.position.set(1,0,-20);return target;},[]);
  const sunPosition=useMemo(()=>HABITAT_SUN_DIR.clone().multiplyScalar(52).add(sunTarget.position),[]);
  return <>
  <HabitatSky key={skyAttempt} appearance={appearance}/>
  <color attach="background" args={[day?'#2A1E14':'#010208']}/>
  {day
    ? <fogExp2 attach="fog" args={['#C4D2DC', .0021]}/>
    : <fog attach="fog" args={['#010208', 180, 480]}/>}
  <hemisphereLight args={[day?'#E8C99A':'#89a5c3', day?'#3D5A66':'#62553e', day?1.05:.95]}/>
  <primitive object={sunTarget}/>
  <directionalLight target={sunTarget} position={day?sunPosition.toArray():[-20,32,-10]} color={day?'#FFC882':'#ffdbaa'} intensity={day?3.85:2.1}
    castShadow shadow-mapSize={[2048,2048]} shadow-camera-left={-38} shadow-camera-right={38} shadow-camera-top={42} shadow-camera-bottom={-42} shadow-normalBias={day?.028:.025}/>
  <directionalLight position={[8,10,18]} color={day?'#7AB8C4':'#b9d5e6'} intensity={day?.58:.7}/>
  <pointLight position={[4,7,-43]} color={day?'#A8C8D8':'#c3dce9'} intensity={day?5:11} distance={21} decay={2}/>
  <pointLight position={[-3,2.7,3]} color="#ffd199" intensity={day?5.5:15} distance={10} decay={2}/>
  <pointLight position={[-10,5.6,-19]} color="#ffcf97" intensity={day?4.5:20} distance={11} decay={2}/>
  <pointLight position={[10,5.7,-19]} color="#ffd4a1" intensity={day?2.8:20} distance={10} decay={2}/>
</>;}
export function HabitatNavigation({district,focusAgent,overview,overviewRevision=0,follow,conversation=false}:{district:ForumDistrict;focusAgent:{id:string;revision:number}|null;overview:boolean;overviewRevision?:number;follow:string|null;conversation?:boolean}){
  const camera=useThree(s=>s.camera),controls=useRef<React.ComponentRef<typeof OrbitControls>>(null),last=useRef<Vector3|null>(null);
  const size=useThree(s=>s.size),reducedMotion=useForumReducedMotion();
  const active=useForumActivity();
  const keyboardInput=useRef(false);
  useEffect(()=>{
    if(!active)return;
    const key=()=>{keyboardInput.current=true;},pointer=()=>{keyboardInput.current=false;};
    document.addEventListener('keydown',key,true);document.addEventListener('pointerdown',pointer,true);
    return()=>{document.removeEventListener('keydown',key,true);document.removeEventListener('pointerdown',pointer,true);};
  },[active]);
  const initialized=useRef(false);
  const previousView=useRef<{district:ForumDistrict;focusAgent:typeof focusAgent;overview:boolean;overviewRevision:number;conversation:boolean;aspect:number}|null>(null);
  const manuallyFramed=useRef(false);
  const transition=useRef<{elapsed:number;retargetedAt:number;from:Vector3;to:Vector3;lookFrom:Vector3;lookTo:Vector3;fovFrom:number;fovTo:number}|null>(null);
  useEffect(()=>{
    // Hidden canvases can report 0x0 on workspace leave. Neither that resize nor
    // returning to the workspace is a new camera command. Keep manual framing;
    // only the automatic Overview fit responds to a real aspect change.
    if(!active||size.width<=1||size.height<=1)return;
    const aspect=size.width/size.height,previous=previousView.current;
    const changed=!previous||previous.district!==district||previous.focusAgent!==focusAgent||previous.overview!==overview||previous.overviewRevision!==overviewRevision||previous.conversation!==conversation;
    const finishTravel=reducedMotion&&transition.current!==null;
    if(!changed&&!finishTravel&&(!overview||manuallyFramed.current||previous.aspect===aspect))return;
    previousView.current={district,focusAgent,overview,overviewRevision,conversation,aspect};
    if(changed)manuallyFramed.current=false;
    const p=focusAgent&&getForumMotion(focusAgent.id),at=p?[p.x,p.y,p.z]:district.position,arrival=district.id==='commons'&&!p;
    // Approach the archive through its central bay. The generic side view puts
    // its near column directly between the camera and the furnished room.
    const archive=district.id==='gallery'&&!p;
    const framing=habitatOverviewCamera(aspect);
    const to=new Vector3(...(conversation?TABLE_CONVERSATION_CAMERA.position:overview?framing.position:arrival?HABITAT_ARRIVAL.position:archive?[-10,5.7,-12.5]:[at[0]+2.9,at[1]+2.05,at[2]+4.1]) as Point3);
    const lookTo=new Vector3(...(conversation?TABLE_CONVERSATION_CAMERA.target:overview?framing.target:arrival?HABITAT_ARRIVAL.target:archive?[-10,4.65,-20.7]:[at[0],at[1]+1.0,at[2]]) as Point3);
    const fovTo=conversation?TABLE_CONVERSATION_CAMERA.fov:overview?framing.fov:arrival?HABITAT_ARRIVAL.fov:58;
    if(initialized.current&&!reducedMotion&&!keyboardInput.current){
      // A ResizeObserver update refits the destination of the same command.
      // Start its remaining segment at the current pose, but keep the original
      // deadline: repeated narrow-shell measurements must not restart travel.
      const elapsed=changed?0:transition.current?.elapsed??0;
      transition.current={elapsed,retargetedAt:elapsed,from:camera.position.clone(),to,lookFrom:controls.current?.target.clone()??lookTo.clone(),lookTo,fovFrom:camera instanceof PerspectiveCamera?camera.fov:fovTo,fovTo};
    }else{
      transition.current=null;
      camera.position.copy(to);controls.current?.target.copy(lookTo);
      if(camera instanceof PerspectiveCamera){camera.fov=fovTo;camera.updateProjectionMatrix();}controls.current?.update();
    }
    initialized.current=true;last.current=null;
  },[camera,district,focusAgent,overview,overviewRevision,conversation,size.width,size.height,reducedMotion,active]);
  useEffect(()=>{last.current=null;},[follow]);
  useFrame((_,dt)=>{
    if(!active)return;
    const travel=transition.current;
    if(travel){
      // This is a timed camera command, not a physics integrator. Capping dt
      // stretched the 0.85 s action over many seconds on slow native frames.
      travel.elapsed+=dt;const t=Math.min(1,(travel.elapsed-travel.retargetedAt)/(.85-travel.retargetedAt)),ease=t*t*(3-2*t);
      camera.position.lerpVectors(travel.from,travel.to,ease);controls.current?.target.lerpVectors(travel.lookFrom,travel.lookTo,ease);
      if(camera instanceof PerspectiveCamera){camera.fov=travel.fovFrom+(travel.fovTo-travel.fovFrom)*ease;camera.updateProjectionMatrix();}
      controls.current?.update();if(t===1)transition.current=null;return;
    }
    if(conversation||!follow||!controls.current)return;const p=getForumMotion(follow);if(!p)return;const next=new Vector3(p.x,p.y+1,p.z);if(last.current){const delta=next.clone().sub(last.current);camera.position.add(delta);controls.current.target.add(delta);}last.current=next;controls.current.update();
  });
  // ForumCanvasActivity owns hidden-control suspension and restores the prior
  // enabled value. Keep conversation ownership separate from that lifecycle.
  return <OrbitControls ref={controls} makeDefault enabled={!conversation} onStart={()=>{transition.current=null;manuallyFramed.current=true;}} minDistance={1} maxDistance={Math.max(110,110*1.6/Math.max(.25,size.width/Math.max(1,size.height)))} maxPolarAngle={Math.PI*.51}/>;
}
