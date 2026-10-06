import { Suspense, useEffect, useMemo, useRef } from 'react';
import { OrbitControls } from '@react-three/drei';
import { useFrame, useThree, type ThreeEvent } from '@react-three/fiber';
import { Mesh, MeshStandardMaterial, PerspectiveCamera, Vector3 } from 'three';
import { HabitatArchitecture } from './HabitatArchitecture';
import { HabitatPlanting } from './HabitatPlanting';
import { HABITAT_ARRIVAL, TABLE_CONVERSATION_CAMERA, habitatCollisionSource, habitatElevation, terrainGeometry, type Point3 } from './habitatLayout';
import { registerForumCollision } from './walkCollision';
import { getForumMotion } from './ForumInhabitants';
import { commandHabitatAgent } from './habitatMotion';
import { HabitatSky } from './HabitatSky';
import { HabitatWater } from './HabitatWater';
import type { ForumDistrict } from './districts';
import { getReduceMotion } from '../../../styles/tokens';

export function HabitatScene({directing,onDirected}:{directing:string|null;onDirected:(point:Point3,ok:boolean)=>void}){
  const ground=useMemo(()=>terrainGeometry(),[]);
  useEffect(()=>()=>ground.dispose(),[ground]);
  useEffect(()=>{const source=habitatCollisionSource(),cleanup=registerForumCollision(source),materials=new Set<MeshStandardMaterial>();source.traverse(o=>{if(o instanceof Mesh){o.geometry.dispose();for(const m of Array.isArray(o.material)?o.material:[o.material])materials.add(m as MeshStandardMaterial);}});materials.forEach(m=>m.dispose());return cleanup;},[]);
  const direct=(e:ThreeEvent<MouseEvent>)=>{if(!directing)return;e.stopPropagation();const p:Point3=[e.point.x,habitatElevation(e.point.x,e.point.z),e.point.z];onDirected(p,commandHabitatAgent(directing,p));};
  return <group onClick={direct}>
    <mesh geometry={ground} position={[0,-.06,0]} receiveShadow><meshStandardMaterial color="#ac9f87" roughness={1}/></mesh>
    <Suspense fallback={null}><HabitatArchitecture/></Suspense><HabitatPlanting/><HabitatWater/>
  </group>;
}
export function HabitatLighting({appearance}:{appearance:'day'|'night'}){const day=appearance==='day';return <>
  <HabitatSky appearance={appearance}/><color attach="background" args={['#1b3944']}/>
  <hemisphereLight args={[day?'#adc9d8':'#89a5c3','#6f6144',day?.85:.55]}/>
  <directionalLight position={[-12,16,-16]} color={day?'#ffd197':'#d6deed'} intensity={day?3.6:1.3} castShadow shadow-mapSize={[2048,2048]} shadow-camera-left={-24} shadow-camera-right={24} shadow-camera-top={32} shadow-camera-bottom={-32} shadow-normalBias={.04}/>
  <directionalLight position={[5,8,14]} color="#ccdeed" intensity={day?.75:.6}/>
  <pointLight position={[-3,2.7,3]} color="#ffc16a" intensity={day?6:26} distance={10} decay={2}/>
  <pointLight position={[-10,5.6,-19]} color="#ffca86" intensity={day?5:28} distance={11} decay={2}/>
  <pointLight position={[10,5.7,-19]} color="#ffd4a1" intensity={day?3:20} distance={10} decay={2}/>
</>;}
export function HabitatNavigation({district,focusAgent,overview,follow,conversation=false}:{district:ForumDistrict;focusAgent:{id:string;revision:number}|null;overview:boolean;follow:string|null;conversation?:boolean}){
  const camera=useThree(s=>s.camera),controls=useRef<React.ComponentRef<typeof OrbitControls>>(null),last=useRef<Vector3|null>(null);
  const initialized=useRef(false);
  const transition=useRef<{elapsed:number;from:Vector3;to:Vector3;lookFrom:Vector3;lookTo:Vector3;fovFrom:number;fovTo:number}|null>(null);
  useEffect(()=>{
    const p=focusAgent&&getForumMotion(focusAgent.id),at=p?[p.x,p.y,p.z]:district.position,arrival=district.id==='commons'&&!p;
    const to=new Vector3(...(conversation?TABLE_CONVERSATION_CAMERA.position:overview?[29,28,31]:arrival?HABITAT_ARRIVAL.position:[at[0]+2.9,at[1]+2.05,at[2]+4.1]) as Point3);
    const lookTo=new Vector3(...(conversation?TABLE_CONVERSATION_CAMERA.target:overview?[0,0,-7]:arrival?HABITAT_ARRIVAL.target:[at[0],at[1]+1.0,at[2]]) as Point3);
    const fovTo=conversation?34:overview?48:58;
    if(initialized.current&&!getReduceMotion()){
      transition.current={elapsed:0,from:camera.position.clone(),to,lookFrom:controls.current?.target.clone()??lookTo.clone(),lookTo,fovFrom:camera instanceof PerspectiveCamera?camera.fov:fovTo,fovTo};
    }else{
      camera.position.copy(to);controls.current?.target.copy(lookTo);
      if(camera instanceof PerspectiveCamera){camera.fov=fovTo;camera.updateProjectionMatrix();}controls.current?.update();
    }
    initialized.current=true;last.current=null;
  },[camera,district,focusAgent,overview,conversation]);
  useEffect(()=>{last.current=null;},[follow]);
  useFrame((_,dt)=>{
    const travel=transition.current;
    if(travel){
      travel.elapsed+=Math.min(dt,.05);const t=Math.min(1,travel.elapsed/.85),ease=t*t*(3-2*t);
      camera.position.lerpVectors(travel.from,travel.to,ease);controls.current?.target.lerpVectors(travel.lookFrom,travel.lookTo,ease);
      if(camera instanceof PerspectiveCamera){camera.fov=travel.fovFrom+(travel.fovTo-travel.fovFrom)*ease;camera.updateProjectionMatrix();}
      controls.current?.update();if(t===1)transition.current=null;return;
    }
    if(conversation||!follow||!controls.current)return;const p=getForumMotion(follow);if(!p)return;const next=new Vector3(p.x,p.y+1,p.z);if(last.current){const delta=next.clone().sub(last.current);camera.position.add(delta);controls.current.target.add(delta);}last.current=next;controls.current.update();
  });
  return <OrbitControls ref={controls} makeDefault enabled={!conversation} onStart={()=>{transition.current=null;}} minDistance={1} maxDistance={65} maxPolarAngle={Math.PI*.51}/>;
}
