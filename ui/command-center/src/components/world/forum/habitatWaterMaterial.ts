import { MeshPhysicalMaterial } from 'three';
import { WATER_COURT, impactFoamGlsl } from './habitatWaterFlow';

/**
 * Downstream wave slopes for pool normals. Clearcoat must use the same field at
 * reduced amplitude: Three's MeshPhysicalMaterial seeds clearcoatNormal from
 * nonPerturbedNormal, so an unperturbed coat reads as a flat mirror pane.
 */
function waveSlopeGlsl(timeName: string, amplitude: number) {
  const a = amplitude.toFixed(4);
  return `
        vec2 waveXZ = vWaterWorld.xz;
        float waveT = ${timeName};
        float a = waveXZ.x * 2.05 + waveXZ.y * 5.15 - waveT * 1.05;
        float b = waveXZ.x * -5.7 + waveXZ.y * 3.35 - waveT * 0.68;
        float c = waveXZ.x * 9.4 + waveXZ.y * 8.1 - waveT * 1.35;
        float d = waveXZ.x * 1.3 - waveXZ.y * 11.2 - waveT * 0.42;
        vec2 slope = vec2(
          cos(a) * 0.016 + cos(b) * 0.011 + cos(c) * 0.006 + cos(d) * 0.004,
          cos(a) * 0.026 + cos(b) * 0.010 + cos(c) * 0.007 + cos(d) * 0.009
        ) * ${a};
`;
}

/** Shared pool dielectric: motion from normals/reflections, not pulsing paint. */
export function createHabitatWaterMaterial(time: { value: number }) {
  const material = new MeshPhysicalMaterial({
    color: '#163f3b',
    roughness: 0.4,
    metalness: 0,
    ior: 1.333,
    transparent: true,
    opacity: 0.84,
    depthWrite: true,
    // Thin wet sheen only — prior 0.28 coat + flat clearcoatNormal made overview
    // pools read as creamy white/gold painted panels under warm daylight.
    clearcoat: 0.08,
    clearcoatRoughness: 0.68,
    specularIntensity: 0.48,
    envMapIntensity: 0.42,
  });
  material.onBeforeCompile = shader => {
    shader.uniforms.uWaterTime = time;
    shader.vertexShader = 'varying vec3 vWaterWorld;\n' + shader.vertexShader;
    shader.vertexShader = shader.vertexShader.replace('#include <worldpos_vertex>', `
      #include <worldpos_vertex>
      vWaterWorld = (modelMatrix * vec4(transformed, 1.0)).xyz;
    `);
    shader.fragmentShader = 'uniform float uWaterTime; varying vec3 vWaterWorld;\n' + shader.fragmentShader;
    // Local blocks keep chunk variables out of the shared main() scope.
    shader.fragmentShader = shader.fragmentShader.replace('#include <normal_fragment_maps>', `
      #include <normal_fragment_maps>
      {
        // Downstream (+Z) advection with a few nonaligned layers — no checker grid.
        ${waveSlopeGlsl('uWaterTime', 1)}
        normal = normalize(mat3(viewMatrix) * vec3(-slope.x, 1.0, -slope.y));
      }
    `);
    // clearcoatNormal is assigned after normal_fragment_maps; sync it here so the
    // coat cannot stay as a geometric-flat environment lobe.
    shader.fragmentShader = shader.fragmentShader.replace('#include <clearcoat_normal_fragment_maps>', `
      #include <clearcoat_normal_fragment_maps>
      #ifdef USE_CLEARCOAT
      {
        ${waveSlopeGlsl('uWaterTime', 0.55)}
        clearcoatNormal = normalize(mat3(viewMatrix) * vec3(-slope.x, 1.0, -slope.y));
      }
      #endif
    `);
    shader.fragmentShader = shader.fragmentShader.replace('#include <color_fragment>', `
      #include <color_fragment>
      {
        vec2 depthXZ = vWaterWorld.xz;
        float depthT = uWaterTime;
        float upperDx = abs(depthXZ.x - 4.0) / 1.52;
        float upperDz = abs(depthXZ.y + 8.05) / 1.61;
        float lowerDx = abs(depthXZ.x - 4.0) / 1.88;
        float lowerDz = abs(depthXZ.y + 2.0) / 2.97;
        float rillDx = abs(depthXZ.x - 4.0) / 1.39;
        float rillDz = abs(depthXZ.y + 5.51) / 0.55;
        float upperEdge = max(upperDx, upperDz);
        float lowerEdge = max(lowerDx, lowerDz);
        float rillEdge = max(rillDx, rillDz);
        float basin = min(upperEdge, min(lowerEdge, rillEdge));
        float depth = clamp(1.0 - basin, 0.0, 1.0);
        vec3 shallow = vec3(0.28, 0.48, 0.44);
        vec3 deep = vec3(0.05, 0.15, 0.14);
        diffuseColor.rgb = mix(shallow, deep, 0.38 + depth * 0.55);

        // Foam: production contract from habitatWaterFlow.impactFoamGlsl / impactFoamWeight.
        ${impactFoamGlsl('depthT')}
        diffuseColor.rgb = mix(diffuseColor.rgb, vec3(0.78, 0.86, 0.84), foam * ${WATER_COURT.foam.mix.toFixed(4)});

        // Tiny albedo transport along flow — secondary to normals. Physical
        // dielectric Fresnel remains MeshPhysicalMaterial's responsibility.
        float driftPhase = depthXZ.y * 1.35 + (depthXZ.x - 4.0) * 0.18 - depthT * 1.15;
        float drift = sin(driftPhase * 0.85 + depthXZ.x * 2.4) * 0.03;
        diffuseColor.rgb *= 0.97 + drift;
      }
    `);
  };
  material.customProgramCacheKey = () => 'habitat-water-flow-v4';
  return material;
}
