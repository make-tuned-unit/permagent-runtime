import { useEffect, useMemo } from 'react';
import { BackSide, Color, Vector3 } from 'three';
import { createCelestialBodies, SKY_RENDER_ORDER } from './ForumSky';

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
  // A darker unlit hemisphere and a thin illuminated limb make the body
  // read as a sphere, without adding a glowing outline around its night side.
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

export function HabitatSky({appearance}:{appearance:'day'|'night'}){
  const bodies=useMemo(()=>{const b=createCelestialBodies();b.gasGiant.position.set(190,285,-1050);b.gasGiant.scale.setScalar(1.75);b.gasGiant.material.fragmentShader=planetFragment;b.gasGiant.material.uniforms.uLight.value=new Vector3(-.92,.27,.18).normalize();b.moon.position.set(-420,430,-1200);b.moon.material.fragmentShader=moonFragment;b.moon.material.uniforms.uLight.value=new Vector3(-.8,.3,.25).normalize();return b;},[]);
  useEffect(()=>()=>{Object.values(bodies).forEach(b=>{b.geometry.dispose();b.material.dispose();});},[bodies]);
  // Three retains the compiled shader's uniform object. Keep its identity and
  // update the value so the visible sky follows the World lighting selector.
  const uniforms=useMemo(()=>({uNight:{value:0},uSun:{value:new Vector3(-.48,.23,-.84).normalize()},uWarm:{value:new Color('#ffd18c')},uBlue:{value:new Color('#081d3b')}}),[]);
  useEffect(()=>{uniforms.uNight.value=appearance==='night'?1:0;},[appearance,uniforms]);
  return <>
    <mesh renderOrder={SKY_RENDER_ORDER.dome}><sphereGeometry args={[4500,48,32]}/><shaderMaterial side={BackSide} depthWrite={false} toneMapped={false} uniforms={uniforms}
      vertexShader={'varying vec3 vD;void main(){vD=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}'}
      fragmentShader={`varying vec3 vD;uniform float uNight;uniform vec3 uSun;uniform vec3 uWarm;uniform vec3 uBlue;
float hash(vec3 p){return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453);}
float noise(vec3 p){vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);}
void main(){vec3 d=normalize(vD);float h=clamp(d.y,0.,1.);float toward=max(dot(d,uSun),0.);float sun=pow(toward,32.);vec3 day=mix(vec3(.042,.091,.14),uBlue,pow(h,.45));
// Separate the small bright solar disc from its amber halo. The previous
// broad high-energy lobe became a flat grey patch under the scene's grade.
day+=uWarm*(pow(toward,36.)*.055+pow(toward,320.)*.65+smoothstep(.99975,.99995,toward)*5.);
float cloud=noise(vec3(d.x*11.,d.y*42.,d.z*11.))*.65+noise(d*31.)*.24;float bands=smoothstep(.46,.67,cloud)*smoothstep(.015,.10,h)*(1.-smoothstep(.22,.40,h));day=mix(day,mix(vec3(.07,.12,.17),vec3(.5,.29,.12),sun),bands*.28);
vec3 night=mix(vec3(.055,.085,.12),vec3(.005,.013,.035),pow(h,.4));float nebula=pow(noise(d*7.),4.);night+=vec3(.06,.045,.10)*nebula;float stars=step(.997,hash(floor(d*850.)))*pow(max(hash(floor(d*1500.)),.0),3.);night+=vec3(.65,.75,.85)*stars;day+=vec3(.44,.56,.70)*stars*.24*(1.-sun)*smoothstep(.1,.3,h);vec3 skyColor=mix(day,night,uNight);skyColor+=(hash(vec3(gl_FragCoord.xy,17.))-.5)/255.;gl_FragColor=vec4(skyColor,1.);}`}/></mesh>
    <primitive object={bodies.gasGiant} dispose={null}/><primitive object={bodies.moon} dispose={null}/>
  </>;
}
