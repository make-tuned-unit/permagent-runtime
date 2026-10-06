import { useEffect, useMemo } from 'react';
import { BackSide, Color, Vector3 } from 'three';
import { createCelestialBodies, SKY_RENDER_ORDER } from './ForumSky';

const planetFragment = `
uniform vec3 uLight; varying vec3 vLocal; varying vec3 vWorld;
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);}
float cloud(vec2 p){return noise(p)*.57+noise(p*2.1)*.28+noise(p*4.2)*.15;}
void main(){
  vec3 n=normalize(vLocal);vec2 q=vec2(atan(n.z,n.x),n.y)*vec2(7.,28.);
  float curl=cloud(q*.55);float fine=cloud(q*3.+curl*4.);
  float belt=sin(q.y*1.6+curl*4.5+fine*.7)*.5+.5;
  vec3 cream=vec3(.58,.42,.25),rust=vec3(.18,.12,.087),blue=vec3(.105,.19,.23);
  vec3 base=mix(rust,cream,smoothstep(.12,.8,belt));base=mix(base,blue,smoothstep(.57,.8,cloud(q*.8))*.55);base*=.72+fine*.58;
  float light=max(dot(n,normalize(uLight)),0.);vec3 view=normalize(cameraPosition-vWorld);
  float rim=pow(1.-max(dot(n,view),0.),4.);
  vec3 color=base*(.10+1.15*pow(light,.7))+vec3(.53,.64,.70)*rim*.07+vec3(1.,.57,.24)*rim*light*.45;
  gl_FragColor=vec4(color,1.);
}`;

export function HabitatSky({appearance}:{appearance:'day'|'night'}){
  const bodies=useMemo(()=>{const b=createCelestialBodies();b.gasGiant.position.set(190,285,-1050);b.gasGiant.scale.setScalar(1.75);b.gasGiant.material.fragmentShader=planetFragment;b.gasGiant.material.uniforms.uLight.value=new Vector3(-.8,.35,.42).normalize();b.moon.position.set(-420,430,-1200);return b;},[]);
  useEffect(()=>()=>{Object.values(bodies).forEach(b=>{b.geometry.dispose();b.material.dispose();});},[bodies]);
  const uniforms=useMemo(()=>({uNight:{value:appearance==='night'?1:0},uSun:{value:new Vector3(-.48,.23,-.84).normalize()},uWarm:{value:new Color('#ffd18c')},uBlue:{value:new Color('#081d3b')}}),[appearance]);
  return <>
    <mesh renderOrder={SKY_RENDER_ORDER.dome}><sphereGeometry args={[4500,48,32]}/><shaderMaterial side={BackSide} depthWrite={false} toneMapped={false} uniforms={uniforms}
      vertexShader={'varying vec3 vD;void main(){vD=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}'}
      fragmentShader={`varying vec3 vD;uniform float uNight;uniform vec3 uSun;uniform vec3 uWarm;uniform vec3 uBlue;
float hash(vec3 p){return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453);}
float noise(vec3 p){vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);return mix(mix(mix(hash(i),hash(i+vec3(1,0,0)),f.x),mix(hash(i+vec3(0,1,0)),hash(i+vec3(1,1,0)),f.x),f.y),mix(mix(hash(i+vec3(0,0,1)),hash(i+vec3(1,0,1)),f.x),mix(hash(i+vec3(0,1,1)),hash(i+vec3(1,1,1)),f.x),f.y),f.z);}
void main(){vec3 d=normalize(vD);float h=clamp(d.y,0.,1.);float sun=pow(max(dot(d,uSun),0.),22.);vec3 day=mix(vec3(.034,.075,.12),uBlue,pow(h,.45));day=mix(day,uWarm,sun*.85);day+=uWarm*pow(max(dot(d,uSun),0.),160.)*1.6;
float cloud=noise(vec3(d.x*7.,d.y*19.,d.z*7.))*.65+noise(d*19.)*.24;float bands=smoothstep(.52,.70,cloud)*smoothstep(.015,.12,h)*(1.-smoothstep(.24,.48,h));day=mix(day,mix(vec3(.10,.15,.19),vec3(.53,.48,.36),sqrt(sun)),bands*.4);
vec3 night=mix(vec3(.055,.085,.12),vec3(.005,.013,.035),pow(h,.4));float nebula=pow(noise(d*7.),4.);night+=vec3(.06,.045,.10)*nebula;float stars=step(.997,hash(floor(d*850.)))*pow(max(hash(floor(d*1500.)),.0),3.);night+=vec3(.65,.75,.85)*stars;day+=vec3(.44,.56,.70)*stars*.24*(1.-sun)*smoothstep(.1,.3,h);gl_FragColor=vec4(mix(day,night,uNight),1.);}`}/></mesh>
    <primitive object={bodies.gasGiant} dispose={null}/><primitive object={bodies.moon} dispose={null}/>
  </>;
}
