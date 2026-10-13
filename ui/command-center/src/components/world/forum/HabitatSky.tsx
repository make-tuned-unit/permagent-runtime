import { Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { useForumTexture as useTexture } from './useForumTexture';
import { useFrame, useThree } from '@react-three/fiber';
import { BackSide, Color, DoubleSide, Group, Vector3 } from 'three';
import { createCelestialBodies, SKY_RENDER_ORDER } from './ForumSky';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { getReduceMotion } from '../../../styles/tokens';
import {
  HABITAT_DAY_PANORAMA_URLS,
  HABITAT_HORIZON_CARDS,
  HABITAT_PANORAMA_ROTATION_Y,
  HABITAT_SUN_DIR,
  HABITAT_VISTA_ASSETS,
  habitatVistaAssetUrl,
  type HabitatHorizonCard,
} from './habitatVista';

const celestialNoise = `
float hash(vec3 p){return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453);}
float noise(vec3 p){vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);}
float cloud(vec3 p){return noise(p)*.57+noise(p*2.03+7.1)*.28+noise(p*4.11+13.2)*.15;}
`;
const planetFragment = `
uniform vec3 uLight; varying vec3 vLocal; varying vec3 vWorld;
${celestialNoise}
void main(){
  vec3 n=normalize(vLocal),q=n*vec3(7.,13.,7.);
  float curl=cloud(n*6.);float fine=cloud(q*3.+curl*3.);
  float belt=sin(n.y*48.+curl*8.+fine*2.)*.5+.5;
  float weather=cloud(q+vec3(curl*2.,fine,0.));
  vec3 base=mix(vec3(.14,.075,.045),vec3(.48,.31,.19),smoothstep(.18,.82,weather*.62+belt*.38));
  base=mix(base,vec3(.11,.17,.21),smoothstep(.59,.78,weather)*.5);
  base*=.78+fine*.4;
  float light=max(dot(n,normalize(uLight)),0.);vec3 view=normalize(cameraPosition-vWorld);
  float facing=max(dot(n,view),0.),rim=pow(1.-facing,5.);
  vec3 color=base*(.025+1.10*pow(light,.8))+vec3(.008,.015,.026)*(1.-light);
  color+=vec3(.7,.39,.16)*rim*pow(light,.45)*.35;
  gl_FragColor=vec4(color,1.);
}`;

const moonFragment = `
uniform vec3 uLight; varying vec3 vLocal; varying vec3 vWorld;
${celestialNoise}
void main(){
  vec3 n=normalize(vLocal);float maria=cloud(n*12.);
  vec3 base=mix(vec3(.12,.105,.095),vec3(.32,.29,.25),smoothstep(.24,.73,maria));
  float light=max(dot(n,normalize(uLight)),0.);
  gl_FragColor=vec4(base*(.04+.95*sqrt(light))+vec3(.004,.009,.016),1.);
}`;

const proceduralDayUniforms = {
  uHorizon: { value: new Color('#FFE3BE') },
  uZenith: { value: new Color('#1A3D6B') },
  uSunGlow: { value: new Color('#FFD9A0') },
  uSun: { value: HABITAT_SUN_DIR.clone() },
  uLandWarm: { value: new Color('#8B7355') },
  uWater: { value: new Color('#5A9AAA') },
  uHaze: { value: new Color('#C8D8E4') },
};

const proceduralDayFragment = `
uniform vec3 uHorizon;uniform vec3 uZenith;uniform vec3 uSunGlow;uniform vec3 uSun;
uniform vec3 uLandWarm;uniform vec3 uWater;uniform vec3 uHaze;
varying vec3 direction;
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);
 return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);}
void main(){
 vec3 d=normalize(direction);
 float height=pow(clamp(d.y,0.,1.),.28);
 vec3 color=mix(uHorizon,uZenith,height);
 float sun=pow(max(dot(d,normalize(uSun)),0.),6.);
 color+=uSunGlow*sun*.58;
 if(d.y<.12){
   float below=smoothstep(.12,-.08,d.y);
   float ridge=noise(vec2(atan(d.x,d.z)*2.2,d.y*18.))*.55+noise(vec2(d.x*8.,d.z*8.))*.45;
   float land=smoothstep(.35,.78,ridge);
   float waterBand=smoothstep(.18,.42,abs(d.z+.35))*smoothstep(.08,.22,-d.y);
   vec3 terrain=mix(uWater,uLandWarm,land);
   color=mix(color,terrain,below*.82);
   color=mix(color,uHaze,below*waterBand*.45);
 }
 gl_FragColor=vec4(color,1.);
}`;

function ProceduralDaySky() {
  const uniforms = useMemo(() => ({
    uHorizon: { value: proceduralDayUniforms.uHorizon.value.clone() },
    uZenith: { value: proceduralDayUniforms.uZenith.value.clone() },
    uSunGlow: { value: proceduralDayUniforms.uSunGlow.value.clone() },
    uSun: { value: HABITAT_SUN_DIR.clone() },
    uLandWarm: { value: proceduralDayUniforms.uLandWarm.value.clone() },
    uWater: { value: proceduralDayUniforms.uWater.value.clone() },
    uHaze: { value: proceduralDayUniforms.uHaze.value.clone() },
  }), []);
  return <mesh renderOrder={SKY_RENDER_ORDER.dome}>
    <sphereGeometry args={[4480, 48, 32]}/>
    <shaderMaterial side={BackSide} depthWrite={false} toneMapped={false} uniforms={uniforms}
      vertexShader={`varying vec3 direction;void main(){direction=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`}
      fragmentShader={proceduralDayFragment}/>
  </mesh>;
}

function GaiaSurveyFallback() {
  return <mesh renderOrder={SKY_RENDER_ORDER.dome}>
    <sphereGeometry args={[4480, 32, 16]}/>
    <meshBasicMaterial color="#010208" side={BackSide} depthWrite={false} toneMapped={false} fog={false}/>
  </mesh>;
}

function GaiaSurveyBackdrop({ appearance }: { appearance: 'day' | 'night' }) {
  const map = useTexture(`${import.meta.env.BASE_URL}world/astronomy/gaia-edr3-milky-way.webp`);
  const strength = appearance === 'night' ? .13 : .09;
  const tint = useMemo(() => new Color(strength, strength, strength), [strength]);
  return <mesh renderOrder={SKY_RENDER_ORDER.dome} rotation={[.22, 1.98, .18]}>
    <sphereGeometry args={[4480, 64, 32]}/>
    <meshBasicMaterial map={map} color={tint} onBeforeCompile={shader => {
      shader.fragmentShader = shader.fragmentShader.replace('#include <map_fragment>', `
        #ifdef USE_MAP
          vec4 sampledDiffuseColor = texture2D(map, vMapUv, 1.8);
          diffuseColor *= sampledDiffuseColor;
        #endif
      `);
    }} customProgramCacheKey={() => 'gaia-survey-filter-v1'} side={BackSide} depthWrite={false} toneMapped={false} fog={false}/>
  </mesh>;
}

function PanoramaDome({ url }: { url: string }) {
  const map = useTexture(url, 'panorama');
  return <mesh renderOrder={SKY_RENDER_ORDER.dome} rotation={[0, HABITAT_PANORAMA_ROTATION_Y, 0]}>
    <sphereGeometry args={[4480, 64, 32]}/>
    <meshBasicMaterial map={map} side={BackSide} depthWrite={false} toneMapped={false} fog={false}/>
  </mesh>;
}

function PanoramaWithFallback() {
  const procedural = <><ProceduralDaySky/><HabitatCelestialBodies/>      <mesh position={HABITAT_SUN_DIR.clone().multiplyScalar(760)}>
        <sphereGeometry args={[5.2, 24, 16]}/>
        <meshBasicMaterial color="#FFF0CE" fog={false} toneMapped={false}/>
      </mesh></>;
  return <GardenAssetBoundary onFailure={() => {}} fallback={
    <GardenAssetBoundary onFailure={() => {}} fallback={procedural}>
      <Suspense fallback={procedural}><PanoramaDome url={HABITAT_DAY_PANORAMA_URLS[1]}/></Suspense>
    </GardenAssetBoundary>
  }>
    <Suspense fallback={
      <GardenAssetBoundary onFailure={() => {}} fallback={procedural}>
        <Suspense fallback={procedural}><PanoramaDome url={HABITAT_DAY_PANORAMA_URLS[1]}/></Suspense>
      </GardenAssetBoundary>
    }>
      <PanoramaDome url={HABITAT_DAY_PANORAMA_URLS[0]}/>
    </Suspense>
  </GardenAssetBoundary>;
}

function TexturedHorizonCard({ card }: { card: HabitatHorizonCard }) {
  const url = habitatVistaAssetUrl(HABITAT_VISTA_ASSETS.cards[card.kind]);
  const map = useTexture(url);
  return <CardMesh card={card} map={map}/>;
}

function CardMesh({ card, map }: { card: HabitatHorizonCard; map: import('three').Texture }) {
  const group = useRef<Group>(null);
  const origin = useMemo(() => new Vector3(...card.position), [card.position]);
  const { camera } = useThree();
  useFrame(() => {
    if (!group.current) return;
    if (getReduceMotion()) {
      group.current.position.set(0, 0, 0);
      return;
    }
    group.current.position.set(
      (camera.position.x - origin.x) * card.parallax,
      0,
      (camera.position.z - origin.z) * card.parallax,
    );
  });
  return <group position={origin}>
    <group ref={group}>
      <mesh rotation={[0, card.rotationY, 0]} renderOrder={SKY_RENDER_ORDER.bodies - 10}>
        <planeGeometry args={[card.width, card.height]}/>
        <meshBasicMaterial map={map} transparent alphaTest={0.08} depthWrite={false} toneMapped={false} side={DoubleSide}/>
      </mesh>
    </group>
  </group>;
}

function HorizonCard({ card }: { card: HabitatHorizonCard }) {
  // Missing optional cutouts should leave the distant panorama unobstructed.
  // Flat procedural stand-ins read as enormous rectangular walls in overview.
  const fallback = null;
  return <GardenAssetBoundary onFailure={() => {}} fallback={fallback}>
    <Suspense fallback={fallback}><TexturedHorizonCard card={card}/></Suspense>
  </GardenAssetBoundary>;
}

function HabitatCelestialBodies() {
  const bodies = useMemo(() => {
    const b = createCelestialBodies();
    b.gasGiant.position.set(190, 285, -1050);
    b.gasGiant.scale.setScalar(1.75);
    b.gasGiant.material.fragmentShader = planetFragment;
    b.gasGiant.material.uniforms.uLight.value = HABITAT_SUN_DIR.clone();
    b.moon.position.set(-420, 430, -1200);
    b.moon.material.fragmentShader = moonFragment;
    b.moon.material.uniforms.uLight.value = new Vector3(-.8, .3, .25).normalize();
    return b;
  }, []);
  useEffect(() => () => Object.values(bodies).forEach(b => { b.geometry.dispose(); b.material.dispose(); }), [bodies]);
  return <>
    <primitive object={bodies.gasGiant} dispose={null}/>
    <primitive object={bodies.moon} dispose={null}/>
  </>;
}

export function HabitatSky({ appearance }: { appearance: 'day' | 'night' }) {
  const nightFallback = <GaiaSurveyFallback/>;
  const [seen, setSeen] = useState({ day: appearance === 'day', night: appearance === 'night' });
  useEffect(() => { setSeen(previous => previous[appearance] ? previous : { ...previous, [appearance]: true }); }, [appearance]);
  return <>
    {(seen.day || appearance === 'day') && <group visible={appearance === 'day'}>
      <PanoramaWithFallback/>
      {HABITAT_HORIZON_CARDS.map(card => <HorizonCard key={card.id} card={card}/>)}
    </group>}
    {(seen.night || appearance === 'night') && <group visible={appearance === 'night'}><GardenAssetBoundary onFailure={() => {}} fallback={nightFallback}>
      <Suspense fallback={nightFallback}><GaiaSurveyBackdrop appearance="night"/></Suspense>
    </GardenAssetBoundary><HabitatCelestialBodies/></group>}
  </>;
}
