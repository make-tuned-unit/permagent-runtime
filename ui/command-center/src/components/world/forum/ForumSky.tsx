import { Suspense, useEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { useGLTF } from '@react-three/drei';
import * as THREE from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import { birdFlight, meteorAt, skyHash } from './skyMotion';
import { extendForumLoader } from './forumGltf';

function NightSky() {
  const stars = useMemo(() => {
    const count = 14000, positions = new Float32Array(count * 3), phases = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const y = skyHash(i * 3) * 2 - 1, a = skyHash(i * 3 + 1) * Math.PI * 2, r = 4500;
      positions.set([Math.sqrt(1-y*y)*Math.cos(a)*r,y*r,Math.sqrt(1-y*y)*Math.sin(a)*r],i*3);
      phases[i] = skyHash(i * 3 + 2);
    }
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position',new THREE.BufferAttribute(positions,3));
    geometry.setAttribute('phase',new THREE.BufferAttribute(phases,1));
    const material = new THREE.ShaderMaterial({
      uniforms: { time: { value: 0 } }, transparent: true, depthWrite: false,
      vertexShader: `attribute float phase; varying float p; void main(){p=phase; gl_PointSize=1.2+pow(phase,8.)*2.4; gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`,
      fragmentShader: `uniform float time; varying float p; void main(){float d=length(gl_PointCoord-.5); float a=(1.-smoothstep(.1,.5,d))*(.72+.28*sin(time*(.7+p)+p*93.)); gl_FragColor=vec4(mix(vec3(.72,.83,1.),vec3(1.,.9,.74),p),a);}`,
    });
    return new THREE.Points(geometry,material);
  },[]);
  const meteor = useMemo(() => {
    const g = new THREE.BufferGeometry();g.setAttribute('position',new THREE.BufferAttribute(new Float32Array(6),3));
    return new THREE.Line(g,new THREE.LineBasicMaterial({color:'#DAE8FF',transparent:true,depthWrite:false,toneMapped:false}));
  },[]);
  useEffect(()=>()=>{stars.geometry.dispose();stars.material.dispose();meteor.geometry.dispose();meteor.material.dispose();},[stars,meteor]);
  useFrame(({clock}) => {
    const reduced = getReduceMotion(), time = clock.elapsedTime;
    stars.material.uniforms.time.value = reduced ? 0 : time;
    const shot = meteorAt(time,reduced);meteor.visible=!!shot;
    if (!shot) return;
    const a=shot.azimuth, p=shot.progress, tail=Math.max(0,p-.19);
    const points=meteor.geometry.attributes.position;
    for(const [i,t] of [tail,p].entries()) {
      const x=-55+t*105,z=205;
      points.setXYZ(i,x*Math.cos(a)-z*Math.sin(a),shot.height-t*28,x*Math.sin(a)+z*Math.cos(a));
    }
    points.needsUpdate=true;meteor.geometry.computeBoundingSphere();meteor.material.opacity=shot.opacity;
  });
  return <><GalacticSky/><primitive object={stars}/><primitive object={meteor}/></>;
}
function GalacticSky() {
  return <mesh>
    <sphereGeometry args={[4800,48,32]}/>
    <shaderMaterial side={THREE.BackSide} depthWrite={false} toneMapped={false}
      vertexShader={`varying vec3 direction;void main(){direction=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`}
      fragmentShader={`varying vec3 direction;
float noise(vec3 p){return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453);}
vec3 galaxy(vec3 d,vec3 center,float size,float tilt){
 center=normalize(center);float facing=dot(d,center);if(facing<.75)return vec3(0.);
 vec3 right=normalize(cross(center,vec3(0.,1.,0.))),up=cross(right,center);
 vec2 p=vec2(dot(d,right),dot(d,up))/size;float c=cos(tilt),s=sin(tilt);p=mat2(c,-s,s,c)*p;p.y*=2.3;
 float r=length(p),a=atan(p.y,p.x);float arms=pow(.5+.5*cos(a*2.-log(r+.09)*4.5),5.);
 float disk=exp(-r*2.8)*(0.18+arms*.82)*smoothstep(.025,.14,r);
 float core=exp(-r*r*95.);float dust=.7+.3*noise(d*1700.);
 return vec3(.18,.27,.46)*disk*dust+vec3(.8,.63,.39)*core;
}
void main(){vec3 d=normalize(direction);float plane=abs(dot(d,normalize(vec3(.18,.82,-.54))));
 float band=exp(-plane*plane*150.)*(.65+.35*noise(d*450.));
 vec3 color=vec3(.001,.003,.009)+vec3(.023,.03,.058)*band;
 color+=galaxy(d,vec3(.35,.48,-.8),.13,.45);
 color+=galaxy(d,vec3(-.85,.37,.25),.08,-.7);
 color+=galaxy(d,vec3(.7,.32,.63),.095,1.2);
 gl_FragColor=vec4(color,1.);}`}/>
  </mesh>;
}
function Birds() {
  const { scene }=useGLTF(`${import.meta.env.BASE_URL}world/forum-bird.glb`, false, false, extendForumLoader);
  const birds=useMemo(()=>Array.from({length:6},()=>scene.clone(true)),[scene]);
  useFrame(({clock})=>birds.forEach((bird,i)=>{
    const motion=birdFlight(getReduceMotion()?0:clock.elapsedTime,i);
    bird.position.set(motion.x,motion.y,motion.z);bird.rotation.set(0,motion.yaw,motion.bank);
    const left=bird.getObjectByName('WingL'),right=bird.getObjectByName('WingR');
    if(left)left.rotation.z=motion.flap;if(right)right.rotation.z=-motion.flap;
  }));
  return <group>{birds.map((bird,i)=><primitive key={i} object={bird} dispose={null}/>)}</group>;
}
function Clouds() {
  const material=useRef<THREE.ShaderMaterial>(null);
  const uniforms=useMemo(()=>({time:{value:0}}),[]);
  useFrame(({clock})=>{if(material.current)material.current.uniforms.time.value=getReduceMotion()?0:clock.elapsedTime;});
  return <mesh position={[0,105,0]} rotation-x={-Math.PI/2}>
    <planeGeometry args={[650,650]}/>
    <shaderMaterial ref={material} uniforms={uniforms} transparent depthWrite={false} side={THREE.DoubleSide}
      vertexShader={`varying vec2 vUv;void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`}
      fragmentShader={`uniform float time;varying vec2 vUv;
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);}
void main(){vec2 p=vUv*8.+vec2(time*.002,0);float n=0.,a=.5;for(int i=0;i<5;i++){n+=noise(p)*a;p=p*2.03+1.7;a*=.5;}float edge=smoothstep(0.,.12,vUv.x)*smoothstep(0.,.12,vUv.y)*smoothstep(0.,.12,1.-vUv.x)*smoothstep(0.,.12,1.-vUv.y);float alpha=smoothstep(.47,.7,n)*edge*.78;gl_FragColor=vec4(mix(vec3(.69,.76,.82),vec3(1.),n),alpha);}`}/>
  </mesh>;
}
// Celestial neighbours (north-star direction, job 12 P3/browser-side): a banded
// gas giant low in the north-west beyond the orbital rings, and a smaller pale
// moon. Both are single spheres with their own shader — two draw calls between
// them, no textures, no lights. They render in day and night alike, so the
// system-appearance switch keeps its existing behaviour and night simply shows
// them against the galactic sky.
const SUNRISE_DIR = new THREE.Vector3(-0.95, 0.27, -0.16).normalize();
const GAS_GIANT_POS = new THREE.Vector3(-1, 0.35, -1).normalize().multiplyScalar(900);
const MOON_POS = new THREE.Vector3(0.6, 0.55, -1).normalize().multiplyScalar(1080);

const BODY_VERT = `
varying vec3 vLocal; varying vec3 vWorld;
void main(){ vLocal=normalize(position); vec4 w=modelMatrix*vec4(position,1.); vWorld=w.xyz;
  gl_Position=projectionMatrix*viewMatrix*w; }`;

const GAS_GIANT_FRAG = `
uniform vec3 uSun; uniform vec3 uWarm; uniform vec3 uCool; uniform vec3 uRim;
varying vec3 vLocal; varying vec3 vWorld;
void main(){
  float lat=vLocal.y;
  // Latitude banding: a broad belt pattern warped by a slower wave so the belts
  // are not perfectly regular, plus a finer set of cloud lanes.
  float belts=sin(lat*14.0+sin(lat*3.1)*1.6)*.5+.5;
  float lanes=sin(lat*41.0+sin(lat*7.3)*2.2)*.5+.5;
  vec3 base=mix(uCool,uWarm,clamp(belts*.68+lanes*.32,0.,1.));
  base*=1.-smoothstep(.52,1.,abs(lat))*.42;                 // darker poles
  // The planet shares the sunrise's bearing, so its face is turned away from
  // the sun: a physically dark disc. Keep the terminator but hold a floor under
  // it, the way a real gas giant is lifted by its own scattered light, or the
  // belts are invisible and it reads as a hole in the sky.
  float lit=clamp(dot(vLocal,uSun)*.55+.62,.34,1.);
  vec3 view=normalize(cameraPosition-vWorld);
  float rim=pow(1.-max(dot(vLocal,view),0.),3.2);           // limb glow
  // The limb glow is strongest on the sun's side, which is what sells a body
  // this size as lit from behind rather than as flat paint.
  float sunSide=clamp(dot(vLocal,uSun)*.5+.5,0.,1.);
  gl_FragColor=vec4(base*lit+uRim*rim*(.35+sunSide*.85),1.);
}`;

const MOON_FRAG = `
uniform vec3 uSun; uniform vec3 uHigh; uniform vec3 uLow;
varying vec3 vLocal; varying vec3 vWorld;
float mhash(vec3 p){return fract(sin(dot(p,vec3(41.7,289.3,183.1)))*43758.5453);}
void main(){
  float mare=smoothstep(.45,.75,mhash(floor(vLocal*7.))*.6+mhash(floor(vLocal*19.))*.4);
  vec3 base=mix(uHigh,uLow,mare*.55);
  float lit=clamp(dot(vLocal,uSun)*.85+.34,.12,1.);
  gl_FragColor=vec4(base*lit,1.);
}`;

function CelestialBodies() {
  const gasGiant=useMemo(()=>({
    uSun:{value:SUNRISE_DIR.clone()},
    uWarm:{value:new THREE.Color('#D9A46A')},
    uCool:{value:new THREE.Color('#6E5B7A')},
    uRim:{value:new THREE.Color('#FFCE9B')},
  }),[]);
  const moon=useMemo(()=>({
    uSun:{value:SUNRISE_DIR.clone()},
    uHigh:{value:new THREE.Color('#D8D4CC')},
    uLow:{value:new THREE.Color('#8E8B87')},
  }),[]);
  return <>
    <mesh position={GAS_GIANT_POS}>
      <sphereGeometry args={[140,48,32]}/>
      <shaderMaterial uniforms={gasGiant} vertexShader={BODY_VERT} fragmentShader={GAS_GIANT_FRAG} depthWrite={false} toneMapped={false}/>
    </mesh>
    <mesh position={MOON_POS}>
      <sphereGeometry args={[46,32,24]}/>
      <shaderMaterial uniforms={moon} vertexShader={BODY_VERT} fragmentShader={MOON_FRAG} depthWrite={false} toneMapped={false}/>
    </mesh>
  </>;
}

export function ForumSky({ appearance }: { appearance: 'day' | 'night' }) {
  return <>
    <CelestialBodies/>
    {appearance === 'night' ? <NightSky/> : <>
      <Clouds/><Suspense fallback={null}><Birds/></Suspense>
      {/* The low sunrise sun, on the same bearing as ForumLighting's key. */}
      <mesh position={SUNRISE_DIR.clone().multiplyScalar(760)}><sphereGeometry args={[5.2,24,16]}/><meshBasicMaterial color="#FFF0CE" fog={false} toneMapped={false}/></mesh>
    </>}
  </>;
}
