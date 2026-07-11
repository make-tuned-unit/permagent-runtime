// GlyphField — the sovereign swarm as a galaxy of glowing point-glyphs.
//
// ONE THREE.Points renders the ENTIRE crowd (0 → AGORA_VISIBLE_CAP) in a single
// draw call. Per-glyph colour comes from an instance attribute (the chiti-ID hue
// when a real occupant list exists, else stable per-slot variety over the truthful
// headcount — see agoraPeers.ts). Positions stream from the O(n) constellation sim
// (agoraSim.ts). Communion between clustered minds is drawn as light PULSES that
// travel the cluster edges — the same "flowing light" language as the private
// Brain graph's SimEdge pulses (BrainScene.ts), at mesh scale.
//
// Dormant today: population 0 ⇒ draw range 0 ⇒ nothing renders, no per-frame cost.
// Honest per §4 — never fabricates peers.

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../../shared/palette';
import { getReduceMotion } from '../../../../styles/tokens';
import { AGORA_VISIBLE_CAP, chitiHue, slotHash, type AgoraPopulation } from './agoraPeers';
import { ensureLayout, stepSim, getPositions, getEdges } from './agoraSim';

const CAP = AGORA_VISIBLE_CAP;

// Glow point-sprite (from BrainScene's PULSE shader family — soft disc + bright core).
const GLYPH_VERT = `
attribute float size;
varying vec3 vCol;
uniform float uPx;
void main() {
  vCol = color;
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  gl_PointSize = size * uPx * (300.0 / -mv.z);
  gl_Position = projectionMatrix * mv;
}`;
const GLYPH_FRAG = `
varying vec3 vCol;
void main() {
  vec2 c = gl_PointCoord - 0.5;
  float d = length(c);
  float a = smoothstep(0.5, 0.0, d);
  float core = smoothstep(0.22, 0.0, d);
  gl_FragColor = vec4(vCol + core * vec3(0.7), a);
}`;

const cA = new THREE.Color(ENV.horizonBlue);
const cB = new THREE.Color(ENV.neonCyan);
const cC = new THREE.Color(ENV.violet);
const tmpCol = new THREE.Color();

/** Cool "mind" hue kept in the palette family (horizon-blue↔cyan, some violet). */
function glyphColor(i: number, pop: AgoraPopulation, out: THREE.Color): THREE.Color {
  const occ = pop.occupants[i];
  const h = occ ? chitiHue(occ.id) : slotHash(i * 5 + 2);
  out.copy(cA).lerp(cB, h);
  const v = slotHash(i * 9 + 4);
  if (v > 0.72) out.lerp(cC, (v - 0.72) * 1.5);
  return out;
}

export function GlyphField({ pop, pixelRatio }: { pop: AgoraPopulation; pixelRatio: number }) {
  const glyphRef = useRef<THREE.Points>(null);
  const pulseRef = useRef<THREE.Points>(null);
  const reduceMotion = useMemo(() => getReduceMotion(), []);

  // Preallocated buffers (CAP-sized, mutated in place — zero per-frame alloc).
  const glyphGeo = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(CAP * 3), 3));
    g.setAttribute('color', new THREE.BufferAttribute(new Float32Array(CAP * 3), 3));
    g.setAttribute('size', new THREE.BufferAttribute(new Float32Array(CAP), 1));
    return g;
  }, []);
  const pulseGeo = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(CAP * 3), 3));
    g.setAttribute('color', new THREE.BufferAttribute(new Float32Array(CAP * 3), 3));
    g.setAttribute('size', new THREE.BufferAttribute(new Float32Array(CAP), 1));
    return g;
  }, []);

  const glyphMat = useMemo(
    () => new THREE.ShaderMaterial({
      vertexShader: GLYPH_VERT,
      fragmentShader: GLYPH_FRAG,
      uniforms: { uPx: { value: pixelRatio } },
      vertexColors: true,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    }),
    [pixelRatio],
  );
  const pulseMat = useMemo(
    () => new THREE.ShaderMaterial({
      vertexShader: GLYPH_VERT,
      fragmentShader: GLYPH_FRAG,
      uniforms: { uPx: { value: pixelRatio } },
      vertexColors: true,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    }),
    [pixelRatio],
  );

  // Colours + sizes are static per population; write once on change and set the
  // draw range from the live count. ensureLayout seeds the swarm positions.
  useMemo(() => {
    ensureLayout(pop.visible);
    const col = glyphGeo.getAttribute('color') as THREE.BufferAttribute;
    const siz = glyphGeo.getAttribute('size') as THREE.BufferAttribute;
    for (let i = 0; i < pop.visible; i++) {
      glyphColor(i, pop, tmpCol);
      col.setXYZ(i, tmpCol.r, tmpCol.g, tmpCol.b);
      (siz.array as Float32Array)[i] = 0.5 + slotHash(i * 11 + 6) * 0.5;
    }
    col.needsUpdate = true;
    siz.needsUpdate = true;
    glyphGeo.setDrawRange(0, pop.visible);
    // Seed glyph positions from the freshly-built layout so a static (reduceMotion)
    // swarm is already in place.
    const gp = glyphGeo.getAttribute('position') as THREE.BufferAttribute;
    const src = getPositions();
    (gp.array as Float32Array).set(src.subarray(0, pop.visible * 3));
    gp.needsUpdate = true;
    return pop.visible;
  }, [pop, glyphGeo]);

  useFrame(() => {
    const n = pop.visible;
    if (n <= 0) return;
    if (!reduceMotion) stepSim(1 / 60, n);

    // Stream sim positions into the glyph buffer.
    const gp = glyphGeo.getAttribute('position') as THREE.BufferAttribute;
    const src = getPositions();
    (gp.array as Float32Array).set(src.subarray(0, n * 3));
    gp.needsUpdate = true;

    // Pulses of communion travel each cluster edge (hub ↔ member).
    const { a, b, count } = getEdges();
    if (count > 0) {
      const pp = pulseGeo.getAttribute('position') as THREE.BufferAttribute;
      const pc = pulseGeo.getAttribute('color') as THREE.BufferAttribute;
      const ps = pulseGeo.getAttribute('size') as THREE.BufferAttribute;
      const t = performance.now() * 0.001;
      const pa = pp.array as Float32Array;
      const sa = ps.array as Float32Array;
      for (let e = 0; e < count; e++) {
        const ia = a[e] * 3;
        const ib = b[e] * 3;
        // Each edge's pulse phase offset by a stable hash — staggered exchange.
        const frac = (t * 0.5 + slotHash(e * 7 + 3)) % 1;
        pa[e * 3] = src[ia] + (src[ib] - src[ia]) * frac;
        pa[e * 3 + 1] = src[ia + 1] + (src[ib + 1] - src[ia + 1]) * frac;
        pa[e * 3 + 2] = src[ia + 2] + (src[ib + 2] - src[ia + 2]) * frac;
        glyphColor(a[e], pop, tmpCol);
        pc.setXYZ(e, tmpCol.r, tmpCol.g, tmpCol.b);
        sa[e] = 0.35 * (0.5 + 0.5 * Math.sin(frac * Math.PI));
      }
      pp.needsUpdate = true;
      pc.needsUpdate = true;
      ps.needsUpdate = true;
      pulseGeo.setDrawRange(0, count);
    }
  });

  return (
    <group>
      <points ref={glyphRef} geometry={glyphGeo} material={glyphMat} />
      <points ref={pulseRef} geometry={pulseGeo} material={pulseMat} />
    </group>
  );
}
