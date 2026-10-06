import { useRef, useState } from 'react';
import { Html } from '@react-three/drei';
import { useFrame } from '@react-three/fiber';
import type { Group } from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import { HABITAT_STATIONS } from './habitatLayout';

const globeFragment = `
  varying vec3 vLocal; varying vec3 vWorld;
  float hash(vec3 p){return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453);}
  float noise(vec3 p){
    vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);
    return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),
      mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);
  }
  void main(){
    vec3 n=normalize(vLocal);
    float field=noise(n*5.)*.65+noise(n*13.)*.25+noise(n*31.)*.10;
    float land=smoothstep(.49,.56,field);
    float lon=atan(n.z,n.x),lat=asin(n.y);
    float grid=max(pow(abs(cos(lon*12.)),80.),pow(abs(cos(lat*12.)),80.))*.4;
    vec3 view=normalize(cameraPosition-vWorld);
    float rim=pow(1.-abs(dot(normalize(vWorld-vec3(-3.,1.51,3.)),view)),3.);
    vec3 color=mix(vec3(.015,.075,.16),vec3(.07,.35,.36),land);
    color+=vec3(.08,.40,.55)*grid+vec3(.08,.44,.61)*rim;
    gl_FragColor=vec4(color,.56+land*.23+rim*.20);
    #include <tonemapping_fragment>
    #include <colorspace_fragment>
  }
`;

/** A team emblem and an in-world overview control, not a simulated job feed. */
export function HabitatOrrery({ conversation, onOverview }: { conversation: boolean; onOverview: () => void }) {
  const globe = useRef<Group>(null);
  const [hovered, setHovered] = useState(false);
  useFrame((_, dt) => {
    if (globe.current && !conversation && !getReduceMotion()) globe.current.rotation.y += Math.min(dt,.05)*.09;
  });
  return <group visible={!conversation} name="Commons team globe" position={[-3,1.51,3]}
    onClick={e=>{e.stopPropagation();setHovered(false);onOverview();}}
    onPointerOver={e=>{e.stopPropagation();setHovered(true);}}
    onPointerOut={()=>setHovered(false)}>
    <group ref={globe} rotation-z={.18}>
      <mesh>
        <sphereGeometry args={[.37,48,32]}/>
        <shaderMaterial transparent depthWrite={false}
          vertexShader="varying vec3 vLocal;varying vec3 vWorld;void main(){vLocal=position;vWorld=(modelMatrix*vec4(position,1.)).xyz;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}"
          fragmentShader={globeFragment}/>
      </mesh>
      {HABITAT_STATIONS.map((s,i)=>{
        const y=1-2*(i+.5)/HABITAT_STATIONS.length,r=Math.sqrt(1-y*y),a=i*2.39996;
        return <mesh key={s.id} position={[Math.cos(a)*r*.386,y*.386,Math.sin(a)*r*.386]}>
          <sphereGeometry args={[.014,8,6]}/><meshBasicMaterial color={s.color} toneMapped={false}/>
        </mesh>;
      })}
    </group>
    <mesh rotation-x={Math.PI/2} position={[0,-.45,0]}>
      <torusGeometry args={[.48,.008,6,64]}/><meshBasicMaterial color="#71d7cf" transparent opacity={.72}/>
    </mesh>
    <mesh rotation-x={Math.PI/2} position={[0,-.43,0]}>
      <torusGeometry args={[.32,.006,6,48]}/><meshBasicMaterial color="#7caac2" transparent opacity={.55}/>
    </mesh>
    <mesh position={[0,-.40,0]}>
      <cylinderGeometry args={[.22,.39,.09,48,1,true]}/><meshBasicMaterial color="#5dc7ca" transparent opacity={.055} depthWrite={false}/>
    </mesh>
    {hovered&&!conversation&&<Html position={[0,.60,0]} center distanceFactor={6} zIndexRange={[20,0]}>
      <div className="habitat-nameplate"><b>Explore the garden</b><span>Open the team overview</span></div>
    </Html>}
  </group>;
}
