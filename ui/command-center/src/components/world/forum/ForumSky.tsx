import { Suspense, useEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { useGLTF } from '@react-three/drei';
import * as THREE from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import { birdFlight, meteorAt, skyHash } from './skyMotion';
import { extendForumLoader } from './forumGltf';

// Sky draw order. three sorts by `renderOrder` BEFORE distance, and the sky
// dome, the starfield and the celestial bodies all sit far outside the world,
// so distance sorting alone decided which of them painted last: the radius-4800
// dome is drawn after the radius-900 gas giant by a front-to-back opaque sort,
// and because the dome writes no depth it simply overpainted both bodies
// (job 18, bug 3). Pinning the three layers in order makes that deterministic —
// dome, then stars, then the bodies, then the world itself at the default 0,
// whose opaque geometry writes depth over them where the ridge or a tower
// really does stand in front.
export const SKY_RENDER_ORDER = { dome: -1000, stars: -900, bodies: -100 } as const;

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
    const points=new THREE.Points(geometry,material);
    points.renderOrder=SKY_RENDER_ORDER.stars;
    return points;
  },[]);
  const meteor = useMemo(() => {
    const g = new THREE.BufferGeometry();g.setAttribute('position',new THREE.BufferAttribute(new Float32Array(6),3));
    const line=new THREE.Line(g,new THREE.LineBasicMaterial({color:'#DAE8FF',transparent:true,depthWrite:false,toneMapped:false}));
    line.renderOrder=SKY_RENDER_ORDER.stars;
    return line;
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
  return <mesh renderOrder={SKY_RENDER_ORDER.dome}>
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
// A thinner, higher-contrast cloud deck than the one this shipped with.
// The plane sits at y = 105 over a 650 m square, so from a camera at terrace
// height it covers every sight line above about 17 degrees of elevation —
// which is where both sky bodies hang. At the old `smoothstep(.47,.7)` and
// 0.78 alpha it was an overcast lid, and it was a large part of why the gas
// giant measured *darker* than the day sky (job 19, bug 1): the disc was being
// painted over by cloud. Sparser and more transparent, it reads as weather
// rather than as a ceiling, and the sky gradient and the bodies come through.
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
void main(){vec2 p=vUv*8.+vec2(time*.002,0);float n=0.,a=.5;for(int i=0;i<5;i++){n+=noise(p)*a;p=p*2.03+1.7;a*=.5;}float edge=smoothstep(0.,.12,vUv.x)*smoothstep(0.,.12,vUv.y)*smoothstep(0.,.12,1.-vUv.x)*smoothstep(0.,.12,1.-vUv.y);float alpha=smoothstep(.58,.86,n)*edge*.46;gl_FragColor=vec4(mix(vec3(.69,.76,.82),vec3(1.),n),alpha);}`}/>
  </mesh>;
}
// Celestial neighbours (north-star direction, job 12 P3/browser-side): a banded
// gas giant well clear of the ridge over the harbour side, and a smaller pale
// moon. Both are single spheres with their own shader — two draw calls between
// them, no textures, no lights. They render in day and night alike, so the
// system-appearance switch keeps its existing behaviour and night simply shows
// them against the galactic sky.
//
// Job 18 (bug 3) measured both as *invisible*: darker than the sky they hang
// in. Three causes, all fixed here.
//  - Draw order. Both bodies wrote no depth, and neither did the radius-4800
//    dome, so which shader survived was decided by three's opaque sort rather
//    than by distance. They now sit at `SKY_RENDER_ORDER.bodies`, after the
//    dome and the starfield, and they *do* write depth — which is also what
//    stops the 14,000-point starfield (transparent, therefore drawn last)
//    from speckling straight through the moon's disc.
//  - Lighting. Both were lit from `SUNRISE_DIR`, a bearing they do not share,
//    so the face turned to the viewer was the unlit one. They are self-lit
//    now: `uLight` is a fixed bearing derived from each body's own position,
//    so the visible face is always the modelled one, in day and in night
//    alike, and the limb glow no longer depends on where the sun is.
//  - Elevation. The gas giant sat ~14 degrees up on the ridge's bearing
//    (Blender polar 100-215 degrees, crest 55-75 m, i.e. up to ~23 degrees as
//    seen from the commons), so the ridge closed it off entirely.
const SUNRISE_DIR = new THREE.Vector3(-0.95, 0.27, -0.16).normalize();

/** A sky bearing as a unit vector: `elevation` in degrees above the horizon,
 *  `x`/`z` the horizontal direction (three.js axes, so -z is the Blender +y
 *  north the landform script is authored in). */
function bearing(x: number, z: number, elevationDeg: number) {
  const horizontal = new THREE.Vector2(x, z).normalize().multiplyScalar(Math.cos(elevationDeg * Math.PI / 180));
  return new THREE.Vector3(horizontal.x, Math.sin(elevationDeg * Math.PI / 180), horizontal.y);
}
/** A body's own lighting bearing: back towards the forum (so the face the
 *  camera sees is the modelled one) and lifted, for a terminator across the
 *  lower limb. Deliberately independent of the sun. */
function selfLight(position: THREE.Vector3) {
  return position.clone().negate().normalize().add(new THREE.Vector3(0, .55, 0)).normalize();
}

/** The daylight sky.
 *
 * Day used to be a flat `<color attach="background">` of `ENV.marble` behind
 * an exponential haze — one cream tone from the horizon to the zenith, luma
 * ~233 everywhere. That is what made job 19 bug 1 unfixable by brightening:
 * the frame is 8-bit, so a disc can never be more than 255/233 = 1.09 times a
 * 233 sky, whatever the shader does. A sky with a real vertical gradient — the
 * deep blue overhead and the warm sunrise band at the horizon of
 * `docs/design/solar-forum/north-star.png` — puts the bodies' own elevations
 * at a luma the disc can clear, and is closer to the reference besides.
 *
 * The mix runs in three's linear working space and the exponent is tuned so
 * the band at 24-30 degrees of elevation — where the gas giant and the moon
 * hang — lands near half the horizon's luminance, while the first few degrees
 * above the horizon stay the warm tone the fog and the low key are matched to.
 */
function DaySky() {
  const uniforms=useMemo(()=>({
    uHorizon:{value:new THREE.Color('#FFE3BE')},
    uZenith:{value:new THREE.Color('#0B2454')},
    uSunGlow:{value:new THREE.Color('#FFD9A0')},
    uSun:{value:SUNRISE_DIR.clone()},
  }),[]);
  return <mesh renderOrder={SKY_RENDER_ORDER.dome}>
    <sphereGeometry args={[4800,48,32]}/>
    <shaderMaterial side={THREE.BackSide} depthWrite={false} toneMapped={false} uniforms={uniforms}
      vertexShader={`varying vec3 direction;void main(){direction=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`}
      fragmentShader={`uniform vec3 uHorizon;uniform vec3 uZenith;uniform vec3 uSunGlow;uniform vec3 uSun;varying vec3 direction;
void main(){vec3 d=normalize(direction);
 float height=pow(clamp(d.y,0.,1.),.30);
 vec3 color=mix(uHorizon,uZenith,height);
 // A soft warm bloom around the low sun, so the sunrise still reads as a
 // direction rather than as an even blue bowl.
 float sun=pow(max(dot(d,normalize(uSun)),0.),6.);
 color+=uSunGlow*sun*.55;
 gl_FragColor=vec4(color,1.);}`}/>
  </mesh>;
}
// Radii are load-bearing beyond the look: the in-app harnesses identify the
// two bodies by sphere radius alone (`verify-world-in-app-v2.mjs`
// `__celestialProbe`), so day legibility is bought with the shading floor
// below, never by resizing them.
export const GAS_GIANT_RADIUS = 140;
export const MOON_RADIUS = 46;
// 26.5 degrees up, 19.6 degrees east of due north — over the lagoon, inside
// the first orbital ring's hoop, and inside the default vista's frame
// (`vistaCamera.ts`).
//
// It used to hang on a *south-westerly* bearing, which puts it squarely behind
// the viewer of the new default vantage. North is the only side of the world
// that reads like `docs/design/solar-forum/north-star.png` — ridge, rings,
// floating gardens — and the ridge mesh spans Blender polar 100-215 degrees,
// so a bearing short of 100 (this one is 70.4) is north of the forum and still
// has no landform anywhere near it.
//
// The exact bearing is composition, not astronomy: due north put the disc
// squarely behind the "The Solar Forum" title card, and this is the bearing
// that clears it while staying left of the moon, which keeps its own
// north-easterly bearing at the top right of the frame.
export const GAS_GIANT_POS = bearing(0.336, -0.942, 26.5).multiplyScalar(900);
// Unchanged: the moon was never occluded, only overpainted and unlit.
export const MOON_POS = new THREE.Vector3(0.6, 0.55, -1).normalize().multiplyScalar(1080);

const BODY_VERT = `
varying vec3 vLocal; varying vec3 vWorld;
void main(){ vLocal=normalize(position); vec4 w=modelMatrix*vec4(position,1.); vWorld=w.xyz;
  gl_Position=projectionMatrix*viewMatrix*w; }`;

const GAS_GIANT_FRAG = `
uniform vec3 uLight; uniform vec3 uWarm; uniform vec3 uCool; uniform vec3 uRim;
varying vec3 vLocal; varying vec3 vWorld;
void main(){
  float lat=vLocal.y;
  // Latitude banding: a broad belt pattern warped by a slower wave so the belts
  // are not perfectly regular, plus a finer set of cloud lanes.
  float belts=sin(lat*14.0+sin(lat*3.1)*1.6)*.5+.5;
  float lanes=sin(lat*41.0+sin(lat*7.3)*2.2)*.5+.5;
  vec3 base=mix(uCool,uWarm,clamp(belts*.68+lanes*.32,0.,1.));
  base*=1.-smoothstep(.52,1.,abs(lat))*.34;                 // darker poles
  // Self-lit. uLight is the body's own bearing, not the scene's sun, so the
  // face turned to the forum is always the modelled one and the floor under
  // the shading keeps the belts legible against both skies.
  float shade=clamp(dot(vLocal,uLight)*.5+.5,0.,1.);
  // The floor is a *day* number. Against the night sky anything reads; against
  // a lit sky the disc has to out-run it, and 8-bit output caps the ratio at
  // 255/sky, so the unlit side cannot be allowed to fall away (job 20).
  vec3 body=base*(1.06+.56*shade);
  vec3 view=normalize(cameraPosition-vWorld);
  float rim=pow(1.-max(dot(vLocal,view),0.),3.0);           // limb glow, all round
  gl_FragColor=vec4(body+uRim*rim*1.15,1.);
}`;

const MOON_FRAG = `
uniform vec3 uLight; uniform vec3 uHigh; uniform vec3 uLow; uniform vec3 uGlow;
varying vec3 vLocal; varying vec3 vWorld;
float mhash(vec3 p){return fract(sin(dot(p,vec3(41.7,289.3,183.1)))*43758.5453);}
void main(){
  float mare=smoothstep(.45,.75,mhash(floor(vLocal*7.))*.6+mhash(floor(vLocal*19.))*.4);
  vec3 base=mix(uHigh,uLow,mare*.42);
  float shade=clamp(dot(vLocal,uLight)*.5+.5,0.,1.);
  vec3 view=normalize(cameraPosition-vWorld);
  float rim=pow(1.-max(dot(vLocal,view),0.),2.4);           // its own faint halo
  gl_FragColor=vec4(base*(.95+.34*shade)+uGlow*rim*.55,1.);
}`;

function body(name: string, position: THREE.Vector3, radius: number, segments: [number,number], fragmentShader: string, uniforms: Record<string, { value: THREE.Color | THREE.Vector3 }>) {
  const mesh = new THREE.Mesh(
    new THREE.SphereGeometry(radius, segments[0], segments[1]),
    new THREE.ShaderMaterial({
      uniforms: { uLight: { value: selfLight(position) }, ...uniforms },
      vertexShader: BODY_VERT, fragmentShader,
      // Opaque and depth-writing: the body is the nearest thing on its own
      // line of sight until the world's own geometry gets in the way.
      depthWrite: true, depthTest: true, toneMapped: false,
    }),
  );
  mesh.name = name;
  // The in-app harness finds meshes either by sphere radius (`__celestialProbe`)
  // or by material name (`__namedMeshProbe`); naming both makes the bodies
  // findable from the browser without another geometry heuristic.
  mesh.material.name = name;
  mesh.position.copy(position);
  mesh.renderOrder = SKY_RENDER_ORDER.bodies;
  mesh.visible = true;
  return mesh;
}

/** The two bodies as plain three objects — built outside React so the draw
 *  order, the positions and the self-lit materials can be asserted directly. */
export const CELESTIAL_NAMES = { gasGiant: 'Forum gas giant', moon: 'Forum moon' } as const;

export function createCelestialBodies() {
  const gasGiant = body(CELESTIAL_NAMES.gasGiant, GAS_GIANT_POS, GAS_GIANT_RADIUS, [48,32], GAS_GIANT_FRAG, {
    // Raised together with the shading floor: the dark belts were what pulled
    // the disc mean down to the day sky's own luminance (job 19, bug 1). The
    // warm/cool hue split that makes it read as a banded planet is kept.
    uWarm: { value: new THREE.Color('#F8D6A4') },
    uCool: { value: new THREE.Color('#B295C6') },
    uRim: { value: new THREE.Color('#FFDDB4') },
  });
  const moon = body(CELESTIAL_NAMES.moon, MOON_POS, MOON_RADIUS, [32,24], MOON_FRAG, {
    uHigh: { value: new THREE.Color('#E4E0D8') },
    uLow: { value: new THREE.Color('#A6A29C') },
    uGlow: { value: new THREE.Color('#C8D6F0') },
  });
  return { gasGiant, moon };
}

function CelestialBodies() {
  const bodies = useMemo(createCelestialBodies, []);
  useEffect(() => () => {
    for (const mesh of [bodies.gasGiant, bodies.moon]) { mesh.geometry.dispose(); mesh.material.dispose(); }
  }, [bodies]);
  return <>
    <primitive object={bodies.gasGiant} name={CELESTIAL_NAMES.gasGiant} dispose={null}/>
    <primitive object={bodies.moon} name={CELESTIAL_NAMES.moon} dispose={null}/>
  </>;
}

export function ForumSky({ appearance }: { appearance: 'day' | 'night' }) {
  return <>
    <CelestialBodies/>
    {appearance === 'night' ? <NightSky/> : <>
      <DaySky/><Clouds/><Suspense fallback={null}><Birds/></Suspense>
      {/* The low sunrise sun, on the same bearing as ForumLighting's key. */}
      <mesh position={SUNRISE_DIR.clone().multiplyScalar(760)}><sphereGeometry args={[5.2,24,16]}/><meshBasicMaterial color="#FFF0CE" fog={false} toneMapped={false}/></mesh>
    </>}
  </>;
}
