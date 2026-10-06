import { useEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { BufferGeometry, DoubleSide, Float32BufferAttribute, MeshPhysicalMaterial, ShaderMaterial } from 'three';
import { getReduceMotion } from '../../../styles/tokens';

/** Small shared ripple shader: the existing sky environment supplies reflections. */
function waterMaterial(time: { value: number }) {
  const material = new MeshPhysicalMaterial({
    color: '#315c52', roughness: .19, metalness: .22,
    clearcoat: 1, clearcoatRoughness: .12, envMapIntensity: 1.3,
  });
  material.onBeforeCompile = shader => {
    shader.uniforms.uWaterTime = time;
    shader.vertexShader = 'varying vec3 vWaterWorld;\n' + shader.vertexShader;
    shader.vertexShader = shader.vertexShader.replace('#include <worldpos_vertex>', `
      #include <worldpos_vertex>
      vWaterWorld = (modelMatrix * vec4(transformed, 1.0)).xyz;
    `);
    shader.fragmentShader = 'uniform float uWaterTime; varying vec3 vWaterWorld;\n' + shader.fragmentShader;
    shader.fragmentShader = shader.fragmentShader.replace('#include <normal_fragment_maps>', `
      #include <normal_fragment_maps>
      vec2 p = vWaterWorld.xz;
      float a = p.x * 7.2 + p.y * 3.7 + uWaterTime * .75;
      float b = p.x * -4.3 + p.y * 9.1 - uWaterTime * .56;
      vec2 slope = vec2(cos(a) * .034 - cos(b) * .016, cos(a) * .018 + cos(b) * .029);
      normal = normalize(mat3(viewMatrix) * vec3(-slope.x, 1., -slope.y));
    `);
    shader.fragmentShader = shader.fragmentShader.replace('#include <color_fragment>', `
      #include <color_fragment>
      float ripple = sin(vWaterWorld.x * 11. + sin(vWaterWorld.z * 7. + uWaterTime * .35))
                   * sin(vWaterWorld.z * 13. - uWaterTime * .42);
      diffuseColor.rgb *= .90 + .10 * ripple;
    `);
  };
  material.customProgramCacheKey = () => 'habitat-water-ripples-v1';
  return material;
}

function cascadeGeometry() {
  const positions: number[] = [], uvs: number[] = [], indices: number[] = [];
  for (let row = 0; row <= 20; row++) for (let col = 0; col <= 24; col++) {
    const v = row / 20, u = col / 24;
    // Water clears the lip, curves outward, then meets the receiving rill.
    positions.push(4 + (u - .5) * 2.76, 1.255 - v * 1.17, -6.43 + Math.sqrt(v) * .39);
    uvs.push(u, 1 - v);
    if (row < 20 && col < 24) {
      const i = row * 25 + col;
      indices.push(i, i + 25, i + 1, i + 1, i + 25, i + 26);
    }
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute('position', new Float32BufferAttribute(positions, 3));
  geometry.setAttribute('uv', new Float32BufferAttribute(uvs, 2));
  geometry.setIndex(indices); geometry.computeVertexNormals();
  return geometry;
}

export function HabitatWater() {
  const time = useRef({ value: 0 });
  const built = useMemo(() => ({
    water: waterMaterial(time.current), cascade: cascadeGeometry(),
    flow: new ShaderMaterial({
      side: DoubleSide, transparent: true, depthWrite: false,
      uniforms: { uTime: time.current },
      vertexShader: 'varying vec2 vUv;void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}',
      fragmentShader: `varying vec2 vUv;uniform float uTime;
        void main(){
          float strands=sin(vUv.x*83.+sin(vUv.y*13.-uTime*1.8)*.7)*.5+.5;
          float fine=sin(vUv.x*217.+vUv.y*7.-uTime*.5)*.5+.5;
          float drop=sin(vUv.y*41.+uTime*4.5+vUv.x*19.)*.5+.5;
          float edge=smoothstep(0.,.045,vUv.x)*smoothstep(0.,.045,1.-vUv.x);
          float foam=pow(1.-vUv.y,9.)*.35;
          vec3 color=mix(vec3(.08,.20,.17),vec3(.47,.65,.57),strands*.45+fine*.15+drop*.1+foam);
          gl_FragColor=vec4(color,edge*(.43+strands*.28+foam));
          #include <tonemapping_fragment>
          #include <colorspace_fragment>
        }`,
    }),
  }), []);
  useFrame((_, dt) => { if (!getReduceMotion()) time.current.value += Math.min(dt, .05); });
  useEffect(() => () => { built.water.dispose(); built.flow.dispose(); built.cascade.dispose(); }, [built]);
  return <group name="Continuous garden cascade">
    <mesh position={[4,.085,-2]} rotation-x={-Math.PI/2} material={built.water}>
      <planeGeometry args={[3.76,5.94]}/>
    </mesh>
    <mesh position={[4,1.255,-8.05]} rotation-x={-Math.PI/2} material={built.water}>
      <planeGeometry args={[3.04,3.22]}/>
    </mesh>
    <mesh position={[4,.085,-5.51]} rotation-x={-Math.PI/2} material={built.water}>
      <planeGeometry args={[2.78,1.10]}/>
    </mesh>
    <mesh geometry={built.cascade} material={built.flow}/>
  </group>;
}
