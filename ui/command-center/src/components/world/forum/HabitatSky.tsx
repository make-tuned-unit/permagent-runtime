import { Suspense, useEffect, useMemo } from 'react';
import { useTexture } from '@react-three/drei';
import { BackSide, Color, SRGBColorSpace, Vector3 } from 'three';
import { createCelestialBodies, SKY_RENDER_ORDER } from './ForumSky';
import { GardenAssetBoundary } from './GardenAssetBoundary';

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

/** A texture failure leaves a quiet sky and the habitat fully usable. */
function GaiaSurveyFallback() {
  return <mesh renderOrder={SKY_RENDER_ORDER.dome}>
    <sphereGeometry args={[4480,32,16]}/>
    <meshBasicMaterial color="#010208" side={BackSide} depthWrite={false} toneMapped={false} fog={false}/>
  </mesh>;
}

/** One owned texture clone; never dispose Drei's cached source. */
function GaiaSurveyBackdrop({appearance}:{appearance:'day'|'night'}) {
  const source=useTexture(`${import.meta.env.BASE_URL}world/astronomy/gaia-edr3-milky-way.webp`);
  const map=useMemo(()=>{const t=source.clone();t.colorSpace=SRGBColorSpace;t.needsUpdate=true;return t;},[source]);
  useEffect(()=>()=>map.dispose(),[map]);
  // Local garden lighting does not create an atmosphere around the station.
  const strength=appearance==='night'?.13:.09;
  const tint=useMemo(()=>new Color(strength,strength,strength),[strength]);
  return <mesh renderOrder={SKY_RENDER_ORDER.dome} rotation={[.22,1.98,.18]}>
    <sphereGeometry args={[4480,64,32]}/>
    <meshBasicMaterial map={map} color={tint} onBeforeCompile={shader=>{
      // A mild mip bias resolves the dense survey stars at display scale.
      // It preserves the source image and avoids a coarse, scintillating grain field.
      shader.fragmentShader=shader.fragmentShader.replace('#include <map_fragment>', `
        #ifdef USE_MAP
          vec4 sampledDiffuseColor = texture2D(map, vMapUv, 1.8);
          diffuseColor *= sampledDiffuseColor;
        #endif
      `);
    }} customProgramCacheKey={()=>'gaia-survey-filter-v1'} side={BackSide} depthWrite={false} toneMapped={false} fog={false}/>
  </mesh>;
}

export function HabitatSky({appearance}:{appearance:'day'|'night'}) {
  const bodies=useMemo(()=>{
    const b=createCelestialBodies();
    b.gasGiant.position.set(190,285,-1050);b.gasGiant.scale.setScalar(1.75);
    b.gasGiant.material.fragmentShader=planetFragment;
    b.gasGiant.material.uniforms.uLight.value=new Vector3(-.92,.27,.18).normalize();
    b.moon.position.set(-420,430,-1200);b.moon.material.fragmentShader=moonFragment;
    b.moon.material.uniforms.uLight.value=new Vector3(-.8,.3,.25).normalize();return b;
  },[]);
  useEffect(()=>()=>Object.values(bodies).forEach(b=>{b.geometry.dispose();b.material.dispose();}),[bodies]);
  const fallback=<GaiaSurveyFallback/>;
  return <>
    <GardenAssetBoundary onFailure={()=>{}} fallback={fallback}>
      <Suspense fallback={fallback}><GaiaSurveyBackdrop appearance={appearance}/></Suspense>
    </GardenAssetBoundary>
    <primitive object={bodies.gasGiant} dispose={null}/><primitive object={bodies.moon} dispose={null}/>
  </>;
}
