import { useEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import { BufferGeometry, DoubleSide, Float32BufferAttribute, ShaderMaterial } from 'three';
import { getReduceMotion } from '../../../styles/tokens';
import {
  WATER_COURT,
  advanceWaterTime,
  cascadeVertex,
} from './habitatWaterFlow';
import { createHabitatWaterMaterial } from './habitatWaterMaterial';

function cascadeGeometry() {
  const { rows, cols } = WATER_COURT.cascade;
  const positions: number[] = [];
  const uvs: number[] = [];
  const indices: number[] = [];
  for (let row = 0; row <= rows; row++) {
    for (let col = 0; col <= cols; col++) {
      const u = col / cols;
      const v = row / rows;
      const p = cascadeVertex(u, v);
      positions.push(p.x, p.y, p.z);
      uvs.push(u, 1 - v);
      if (row < rows && col < cols) {
        const i = row * (cols + 1) + col;
        indices.push(i, i + cols + 1, i + 1, i + 1, i + cols + 1, i + cols + 2);
      }
    }
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute('position', new Float32BufferAttribute(positions, 3));
  geometry.setAttribute('uv', new Float32BufferAttribute(uvs, 2));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

function cascadeMaterial(time: { value: number }) {
  return new ShaderMaterial({
    side: DoubleSide,
    transparent: true,
    depthWrite: false,
    uniforms: { uTime: time },
    vertexShader: `
      varying vec2 vUv;
      varying float vFall;
      void main() {
        vUv = uv;
        vFall = 1.0 - uv.y;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: `
      varying vec2 vUv;
      varying float vFall;
      uniform float uTime;
      void main() {
        float flow = vFall * 16.0 - uTime * 3.4;
        float sheet = sin(vUv.x * 19.0 + sin(flow) * 1.1 + vUv.x * 3.7) * 0.5 + 0.5;
        float breakUp = sin(vUv.x * 37.0 - flow * 0.62 + sin(vUv.x * 11.0) * 0.8) * 0.5 + 0.5;
        float fine = sin(vUv.x * 71.0 + flow * 0.35 - vFall * 2.0) * 0.5 + 0.5;
        float edge = smoothstep(0.0, 0.05, vUv.x) * smoothstep(0.0, 0.05, 1.0 - vUv.x);
        float body = mix(0.62, 0.28, smoothstep(0.05, 0.55, vFall));
        float landFoam = pow(smoothstep(0.72, 1.0, vFall), 1.8) * (0.35 + breakUp * 0.25);
        vec3 deep = vec3(0.10, 0.24, 0.22);
        vec3 lit = vec3(0.42, 0.58, 0.52);
        vec3 color = mix(deep, lit, sheet * 0.35 + fine * 0.12 + landFoam);
        float alpha = edge * (body + sheet * 0.18 + landFoam * 0.35);
        gl_FragColor = vec4(color, clamp(alpha, 0.0, 0.78));
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }
    `,
  });
}

export function HabitatWater() {
  const time = useRef({ value: 0 });
  const built = useMemo(() => ({
    water: createHabitatWaterMaterial(time.current),
    cascade: cascadeGeometry(),
    flow: cascadeMaterial(time.current),
  }), []);
  useFrame((_, dt) => {
    time.current.value = advanceWaterTime(time.current.value, dt, getReduceMotion());
  });
  useEffect(() => () => {
    built.water.dispose();
    built.flow.dispose();
    built.cascade.dispose();
  }, [built]);
  const { upper, lower, rill } = WATER_COURT;
  return (
    <group name="Continuous garden cascade">
      <mesh position={[...lower.center]} rotation-x={-Math.PI / 2} material={built.water}>
        <planeGeometry args={[...lower.sizeXZ]} />
      </mesh>
      <mesh position={[...upper.center]} rotation-x={-Math.PI / 2} material={built.water}>
        <planeGeometry args={[...upper.sizeXZ]} />
      </mesh>
      <mesh position={[...rill.center]} rotation-x={-Math.PI / 2} material={built.water}>
        <planeGeometry args={[...rill.sizeXZ]} />
      </mesh>
      <mesh geometry={built.cascade} material={built.flow} />
    </group>
  );
}
