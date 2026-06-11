import { useRef, useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';

// Material palette — gunmetal-dominant body, bronze accent, emissive trim
const BODY_COLOR = '#555B6E';       // gunmetal — primary surface
const JOINT_COLOR = '#33363F';      // dark metal — joints, inner segments
const ACCENT_COLOR = '#A08555';     // worn bronze — shoulder caps, knee plates
const VISOR_GLOW_FALLBACK = '#00D9FF';

interface CyborgProps {
  trimColor?: string;
  weathering?: number; // 0-1, increases roughness
  showCrown?: boolean;
}

function CyborgCrown({ trimColor }: { trimColor: string }) {
  const ref = useRef<THREE.Group>(null);

  useFrame(() => {
    if (ref.current) {
      ref.current.position.y = 0.1 * Math.sin(performance.now() * 0.003);
    }
  });

  return (
    <group ref={ref} position-y={2.35}>
      {/* Crown base ring — metallic gold */}
      <mesh>
        <torusGeometry args={[0.25, 0.04, 8, 5]} />
        <meshStandardMaterial color="#FFD700" roughness={0.2} metalness={0.8} emissive="#FFD700" emissiveIntensity={0.3} />
      </mesh>
      {/* Crown points */}
      {Array.from({ length: 5 }, (_, i) => {
        const angle = (i / 5) * Math.PI * 2;
        return (
          <mesh key={i} position={[Math.cos(angle) * 0.25, 0.15, Math.sin(angle) * 0.25]}>
            <coneGeometry args={[0.06, 0.2, 4]} />
            <meshStandardMaterial color="#FFD700" roughness={0.2} metalness={0.8} emissive="#FFD700" emissiveIntensity={0.3} />
          </mesh>
        );
      })}
      {/* Emissive gem accents in trim color */}
      <mesh position={[0.25, 0.05, 0]}>
        <sphereGeometry args={[0.04, 8, 8]} />
        <meshBasicMaterial color={trimColor} transparent opacity={0.9} />
      </mesh>
      <mesh position={[-0.25, 0.05, 0]}>
        <sphereGeometry args={[0.04, 8, 8]} />
        <meshBasicMaterial color={trimColor} transparent opacity={0.9} />
      </mesh>
      <pointLight color={trimColor} intensity={0.4} distance={3} />
    </group>
  );
}

export function CyborgCharacterModel({ trimColor = VISOR_GLOW_FALLBACK, weathering = 0, showCrown = false }: CyborgProps) {
  const groupRef = useRef<THREE.Group>(null);

  // Subtle idle sway
  useFrame(() => {
    if (groupRef.current) {
      groupRef.current.rotation.z = Math.sin(performance.now() * 0.002) * 0.015;
    }
  });

  const baseRoughness = 0.2 + weathering * 0.2;

  // Shared materials
  const bodyMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: BODY_COLOR, roughness: baseRoughness + 0.2, metalness: 0.4, emissive: BODY_COLOR, emissiveIntensity: 0.08 }),
    [baseRoughness]
  );
  const jointMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: JOINT_COLOR, roughness: 0.3, metalness: 0.5, emissive: trimColor, emissiveIntensity: 0.05 }),
    [trimColor]
  );
  const accentMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: ACCENT_COLOR, roughness: 0.3 + weathering * 0.15, metalness: 0.6 }),
    [weathering]
  );
  const trimMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 2.0, roughness: 0.2, metalness: 0.1 }),
    [trimColor]
  );
  const underGlowMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 1.2, roughness: 0.2, metalness: 0.1, transparent: true, opacity: 0.8 }),
    [trimColor]
  );
  const visorMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 3.5, roughness: 0.05, metalness: 0.1 }),
    [trimColor]
  );
  const jointGlowMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 1.0, roughness: 0.2, metalness: 0.1 }),
    [trimColor]
  );
  // Cape material — semi-transparent dark fabric with emissive trim edge
  const capeMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: '#1A1D25', roughness: 0.7, metalness: 0.1, transparent: true, opacity: 0.4, side: THREE.DoubleSide, vertexColors: true }),
    []
  );
  const capeEdgeMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 1.5, roughness: 0.2, metalness: 0.1 }),
    [trimColor]
  );

  // Torso — LatheGeometry with chest-to-waist S-curve (Sonny-like taper)
  const torsoGeo = useMemo(() => {
    const points: THREE.Vector2[] = [];
    // Profile: Y goes bottom-to-top, X is radius
    // Waist (bottom) → chest (top) with a flowing curve
    const profile = [
      [0.18, 0.0],   // hip base
      [0.17, 0.05],  // low hip
      [0.14, 0.18],  // waist (narrowest)
      [0.16, 0.30],  // lower ribcage
      [0.23, 0.55],  // mid chest (widest)
      [0.25, 0.68],  // upper chest
      [0.24, 0.78],  // shoulder line
      [0.20, 0.88],  // shoulder top taper
      [0.14, 0.98],  // upper shoulder / collar
      [0.10, 1.05],  // neck base
    ];
    for (const [x, y] of profile) {
      points.push(new THREE.Vector2(x, y));
    }
    return new THREE.LatheGeometry(points, 16);
  }, []);

  // Shoulder cap — curved capsule shape instead of box
  const shoulderGeo = useMemo(() => {
    return new THREE.CapsuleGeometry(0.07, 0.08, 4, 8);
  }, []);

  // === CAPE DRAPE SYSTEM ===
  // Single source of truth for cape vertex positions.
  // The cape attaches at the neck, follows the torso's back surface,
  // then hangs free below the waist where the body narrows.

  // Body back z at a given absolute y (from the torso LatheGeometry profile)
  const bodyBackZ = (absY: number): number => {
    // Torso profile placed at y=0.68. Profile maps relative y → radius.
    const relY = absY - 0.68;
    // Interpolate the profile (relative y → radius, back surface = -radius)
    const prof: [number, number][] = [
      [0.0, 0.18], [0.05, 0.17], [0.18, 0.14], [0.30, 0.16],
      [0.55, 0.23], [0.68, 0.25], [0.78, 0.24], [0.88, 0.20],
      [0.98, 0.14], [1.05, 0.10],
    ];
    if (relY <= 0) return -prof[0][1];
    if (relY >= prof[prof.length - 1][0]) return -prof[prof.length - 1][1];
    for (let i = 0; i < prof.length - 1; i++) {
      if (relY >= prof[i][0] && relY <= prof[i + 1][0]) {
        const t = (relY - prof[i][0]) / (prof[i + 1][0] - prof[i][0]);
        return -(prof[i][1] + t * (prof[i + 1][1] - prof[i][1]));
      }
    }
    return -0.18;
  };

  // Cape top = neck (y=1.75), bottom = calf (y=0.30)
  const CAPE_TOP = 1.75;
  const CAPE_BOT = 0.30;
  const CAPE_LEN = CAPE_TOP - CAPE_BOT;

  // Cape position function: u (0..1 across width), v (0..1 top to bottom)
  const capePos = (u: number, v: number): [number, number, number] => {
    const y = CAPE_TOP - v * CAPE_LEN;

    // Width: flares from neck to shoulder width, then gently widens to hem
    const neckW = 0.22;
    const shoulderW = 0.70; // wider than shoulder caps at ±0.30
    const hemW = 0.85;
    let width: number;
    if (v < 0.08) {
      // Neck to shoulder flare
      const t = v / 0.08;
      width = neckW + (shoulderW - neckW) * t * t; // ease-in
    } else {
      width = shoulderW + (hemW - shoulderW) * ((v - 0.08) / 0.92);
    }
    const x = (u - 0.5) * width;

    // Z: follow body back surface at top, hang free below waist
    const bodyZ = bodyBackZ(y);
    const offset = 0.03; // small gap to prevent z-fighting
    const bodyFollowZ = bodyZ - offset; // hugs the back

    // Below the waist (v > ~0.6, y < ~0.86), the body narrows but cape hangs straight
    const freeHangZ = -0.28; // the z where the cape hangs freely
    const waistV = 0.55; // where body is narrowest
    let z: number;
    if (v < waistV) {
      // Follow the body
      z = Math.min(bodyFollowZ, freeHangZ);
    } else {
      // Transition to free hang — cape doesn't follow body inward
      z = freeHangZ;
    }

    // Shoulder wrap: top edges come forward, visible from the front
    const edgeness = Math.abs(u - 0.5) * 2; // 0 at center, 1 at edge
    const topWrap = Math.max(0, 1 - v * 8) * edgeness * 0.12; // only affects top ~12%
    z += topWrap;

    return [x, y, z];
  };

  // Build cape geometry
  const capeGeo = useMemo(() => {
    const widthSegs = 14;
    const heightSegs = 20;
    const geo = new THREE.PlaneGeometry(1, 1, widthSegs, heightSegs);
    const pos = geo.attributes.position;
    const colors = new Float32Array(pos.count * 3);

    for (let i = 0; i < pos.count; i++) {
      const u = pos.getX(i) + 0.5;
      const v = pos.getY(i) + 0.5;
      const [cx, cy, cz] = capePos(u, v);
      pos.setXYZ(i, cx, cy, cz);

      const alpha = 1.0 - v * 0.55;
      colors[i * 3] = alpha;
      colors[i * 3 + 1] = alpha;
      colors[i * 3 + 2] = alpha;
    }
    pos.needsUpdate = true;
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    geo.computeVertexNormals();
    return geo;
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Cape hem trim
  const capeHemGeo = useMemo(() => {
    const pts: THREE.Vector3[] = [];
    for (let i = 0; i <= 20; i++) {
      const [x, y, z] = capePos(i / 20, 1.0);
      pts.push(new THREE.Vector3(x, y, z));
    }
    return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 20, 0.012, 4, false);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Cape interior circuit lines
  const capeCircuitGeos = useMemo(() => {
    const normXPositions = [0.2, 0.35, 0.5, 0.65, 0.8]; // u positions
    return normXPositions.map((nu) => {
      const pts: THREE.Vector3[] = [];
      for (let i = 0; i <= 12; i++) {
        const v = i / 12;
        const [x, y, z] = capePos(nu, v);
        pts.push(new THREE.Vector3(x, y, z + 0.005));
      }
      return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 10, 0.005, 3, false);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Cape horizontal panel seams
  const capePanelGeos = useMemo(() => {
    return [0.25, 0.50, 0.75].map((vt) => {
      const pts: THREE.Vector3[] = [];
      for (let i = 0; i <= 16; i++) {
        const [x, y, z] = capePos(i / 16, vt);
        pts.push(new THREE.Vector3(x, y, z + 0.005));
      }
      return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 14, 0.006, 3, false);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Cape side edge trims
  const capeEdgeGeos = useMemo(() => {
    return [0.0, 1.0].map((uVal) => {
      const pts: THREE.Vector3[] = [];
      for (let i = 0; i <= 16; i++) {
        const v = i / 16;
        const [x, y, z] = capePos(uVal, v);
        pts.push(new THREE.Vector3(x, y, z));
      }
      return new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 12, 0.008, 4, false);
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Low-intensity trim for interior cape circuits
  const capeCircuitMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 0.4, roughness: 0.3, metalness: 0.1, transparent: true, opacity: 0.6 }),
    [trimColor]
  );
  const capePanelMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: trimColor, emissive: trimColor, emissiveIntensity: 0.6, roughness: 0.3, metalness: 0.1 }),
    [trimColor]
  );

  return (
    <group ref={groupRef}>
      {/* === HEAD — sleek elongated skull === */}
      <group position-y={1.88}>
        {/* Skull — elongated sphere (taller than wide, slight front-back stretch) */}
        <mesh material={bodyMat} castShadow scale={[0.85, 1.1, 0.95]}>
          <sphereGeometry args={[0.28, 16, 12]} />
        </mesh>

        {/* Cranial ridge — subtle raised line over the top */}
        <mesh position={[0, 0.18, 0]} rotation-x={Math.PI / 2}>
          <cylinderGeometry args={[0.02, 0.02, 0.4, 4]} />
          <primitive object={trimMat} attach="material" />
        </mesh>

        {/* Visor — angular wrap-around band, slightly narrower = machine mask */}
        <mesh position={[0, 0.02, 0.22]} material={visorMat}>
          <boxGeometry args={[0.22, 0.045, 0.06]} />
        </mesh>
        {/* Visor side wraps */}
        {[1, -1].map((side) => (
          <mesh key={`vw-${side}`} position={[side * 0.13, 0.02, 0.18]} rotation-y={side * 0.5} material={visorMat}>
            <boxGeometry args={[0.06, 0.04, 0.04]} />
          </mesh>
        ))}
        {/* Visor light — bright, casts visible pool onto chest */}
        <pointLight position={[0, 0.02, 0.28]} color={trimColor} intensity={1.5} distance={3} decay={2} />

        {/* Face plate seam — vertical center line */}
        <mesh position={[0, -0.06, 0.24]}>
          <boxGeometry args={[0.015, 0.16, 0.02]} />
          <primitive object={trimMat} attach="material" />
        </mesh>

        {/* Jaw line — angular, flush, reads as chin plate not mouth */}
        <mesh position={[0, -0.14, 0.18]} material={jointMat}>
          <boxGeometry args={[0.16, 0.06, 0.12]} />
        </mesh>

        {/* Temple channels — run along the skull, not protruding */}
        {[1, -1].map((side) => (
          <mesh key={`tc-${side}`} position={[side * 0.2, 0.06, 0.08]} rotation-z={side * -0.15} rotation-y={side * 0.4}>
            <cylinderGeometry args={[0.015, 0.015, 0.25, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
        ))}
        {/* Upper skull channels — over the crown */}
        {[1, -1].map((side) => (
          <mesh key={`uc-${side}`} position={[side * 0.1, 0.2, -0.05]} rotation-z={side * -0.1} rotation-x={0.4}>
            <cylinderGeometry args={[0.012, 0.012, 0.22, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
        ))}

        {/* Flush sensor pads (NOT protruding ears) — inset discs on skull */}
        {[1, -1].map((side) => (
          <mesh key={`sp-${side}`} position={[side * 0.24, -0.02, 0]} rotation-y={side * Math.PI / 2}>
            <cylinderGeometry args={[0.04, 0.04, 0.01, 8]} />
            <primitive object={visorMat} attach="material" />
          </mesh>
        ))}
      </group>

      {/* === NECK — tapered joint, sits inside torso collar === */}
      <mesh position-y={1.72} material={jointMat}>
        <cylinderGeometry args={[0.07, 0.09, 0.14, 12]} />
      </mesh>
      <mesh position-y={1.78}>
        <torusGeometry args={[0.08, 0.015, 6, 16]} />
        <primitive object={trimMat} attach="material" />
      </mesh>

      {/* === TORSO — curved lathe body with panel details === */}
      <mesh position-y={0.68} material={bodyMat} castShadow>
        <primitive object={torsoGeo} attach="geometry" />
      </mesh>

      {/* Panel seam rings on the curved torso — reads as segmented armor wrapping the form */}
      {[
        { y: 0.88, r: 0.15 },  // waist
        { y: 1.08, r: 0.19 },  // lower chest
        { y: 1.30, r: 0.24 },  // mid chest
        { y: 1.48, r: 0.22 },  // upper chest
      ].map(({ y, r }, i) => (
        <mesh key={`pr-${i}`} position-y={y}>
          <torusGeometry args={[r, 0.012, 4, 20]} />
          <primitive object={underGlowMat} attach="material" />
        </mesh>
      ))}

      {/* Under-plate inner glow — cylinder inside the torso, visible through ring gaps */}
      <mesh position-y={0.68} material={underGlowMat}>
        <cylinderGeometry args={[0.12, 0.10, 0.85, 12]} />
      </mesh>

      {/* Vertical chest channels — follow the curve, doubled coverage */}
      <mesh position={[0.12, 1.2, 0.18]}>
        <cylinderGeometry args={[0.015, 0.015, 0.6, 4]} />
        <primitive object={trimMat} attach="material" />
      </mesh>
      <mesh position={[-0.12, 1.2, 0.18]}>
        <cylinderGeometry args={[0.015, 0.015, 0.6, 4]} />
        <primitive object={trimMat} attach="material" />
      </mesh>
      {/* Center seam */}
      <mesh position={[0, 1.15, 0.20]}>
        <cylinderGeometry args={[0.012, 0.012, 0.7, 4]} />
        <primitive object={trimMat} attach="material" />
      </mesh>
      {/* Side channels — along the flanks */}
      {[1, -1].map((side) => (
        <mesh key={`sc-${side}`} position={[side * 0.20, 1.15, 0.08]}>
          <cylinderGeometry args={[0.012, 0.012, 0.55, 4]} />
          <primitive object={trimMat} attach="material" />
        </mesh>
      ))}
      {/* Rear spine channel */}
      <mesh position={[0, 1.15, -0.16]}>
        <cylinderGeometry args={[0.015, 0.015, 0.65, 4]} />
        <primitive object={trimMat} attach="material" />
      </mesh>
      {/* Rear diagonal channels */}
      {[1, -1].map((side) => (
        <mesh key={`rd-${side}`} position={[side * 0.10, 1.2, -0.14]} rotation-z={side * 0.15}>
          <cylinderGeometry args={[0.010, 0.010, 0.5, 4]} />
          <primitive object={trimMat} attach="material" />
        </mesh>
      ))}

      {/* Shoulder caps — curved capsules, bronze accent */}
      {[1, -1].map((side) => (
        <group key={`shoulder-${side}`} position={[side * 0.30, 1.50, 0]}>
          <mesh geometry={shoulderGeo} material={accentMat} castShadow rotation-z={side * 0.3} />
          <mesh position-y={-0.06}>
            <torusGeometry args={[0.06, 0.012, 4, 8]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
        </group>
      ))}

      {/* === ARMS — tapered, sleek === */}
      {[1, -1].map((side) => (
        <group key={`arm-${side}`}>
          {/* Upper arm — tapered cylinder, thicker at shoulder */}
          <mesh position={[side * 0.38, 1.28, 0]} rotation-z={side * 0.12} material={bodyMat} castShadow>
            <cylinderGeometry args={[0.055, 0.042, 0.42, 10]} />
          </mesh>
          {/* Upper arm trim channel */}
          <mesh position={[side * 0.38, 1.28, 0.055]} rotation-z={side * 0.12}>
            <cylinderGeometry args={[0.010, 0.010, 0.34, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
          {/* Elbow joint — smooth sphere */}
          <mesh position={[side * 0.42, 1.04, 0]} material={jointMat}>
            <sphereGeometry args={[0.038, 10, 10]} />
          </mesh>
          {/* Elbow glow ring */}
          <mesh position={[side * 0.42, 1.04, 0]} rotation-x={Math.PI / 2}>
            <torusGeometry args={[0.032, 0.006, 4, 12]} />
            <primitive object={jointGlowMat} attach="material" />
          </mesh>
          {/* Forearm — tapered, narrower than upper arm */}
          <mesh position={[side * 0.44, 0.84, 0.02]} rotation-z={side * 0.05} material={bodyMat}>
            <cylinderGeometry args={[0.042, 0.032, 0.38, 10]} />
          </mesh>
          {/* Forearm trim channel */}
          <mesh position={[side * 0.44, 0.84, 0.065]} rotation-z={side * 0.05}>
            <cylinderGeometry args={[0.012, 0.012, 0.3, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
          {/* Wrist joint */}
          <mesh position={[side * 0.45, 0.63, 0.03]} material={jointMat}>
            <sphereGeometry args={[0.028, 8, 8]} />
          </mesh>
          {/* Wrist glow ring */}
          <mesh position={[side * 0.45, 0.63, 0.03]}>
            <torusGeometry args={[0.024, 0.005, 4, 10]} />
            <primitive object={jointGlowMat} attach="material" />
          </mesh>
          {/* Hand — tapered wedge */}
          <mesh position={[side * 0.45, 0.55, 0.04]} material={bodyMat}>
            <cylinderGeometry args={[0.022, 0.035, 0.12, 6]} />
          </mesh>
        </group>
      ))}

      {/* === LEGS — tapered, flowing === */}
      {[1, -1].map((side) => (
        <group key={`leg-${side}`}>
          {/* Upper leg — tapered, thicker at hip */}
          <mesh position={[side * 0.12, 0.42, 0]} material={bodyMat} castShadow>
            <cylinderGeometry args={[0.07, 0.055, 0.5, 10]} />
          </mesh>
          {/* Knee — bronze accent cap wrapping the joint */}
          <mesh position={[side * 0.12, 0.18, 0]} material={accentMat}>
            <sphereGeometry args={[0.05, 10, 10]} />
          </mesh>
          {/* Knee glow ring */}
          <mesh position={[side * 0.12, 0.18, 0]} rotation-x={Math.PI / 2}>
            <torusGeometry args={[0.042, 0.006, 4, 12]} />
            <primitive object={jointGlowMat} attach="material" />
          </mesh>
          {/* Lower leg — tapered, narrower */}
          <mesh position={[side * 0.12, 0.05, 0.01]} material={bodyMat}>
            <cylinderGeometry args={[0.05, 0.04, 0.28, 10]} />
          </mesh>
          {/* Leg trim line — front */}
          <mesh position={[side * 0.12, 0.3, 0.065]}>
            <cylinderGeometry args={[0.012, 0.012, 0.45, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
          {/* Leg trim line — inner side */}
          <mesh position={[side * 0.06, 0.3, 0.02]}>
            <cylinderGeometry args={[0.010, 0.010, 0.4, 4]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
        </group>
      ))}

      {/* === FEET — sleek pointed boots === */}
      {[1, -1].map((side) => (
        <group key={`foot-${side}`} position={[side * 0.12, 0, 0.02]}>
          {/* Boot — tapered cylinder laid on side */}
          <mesh material={bodyMat} position={[0, -0.05, 0.04]} rotation-x={Math.PI / 2}>
            <cylinderGeometry args={[0.03, 0.045, 0.18, 8]} />
          </mesh>
          {/* Heel pad */}
          <mesh position={[0, -0.07, -0.04]} material={jointMat}>
            <boxGeometry args={[0.06, 0.02, 0.06]} />
          </mesh>
          {/* Boot trim ring */}
          <mesh position-y={-0.08} rotation-x={Math.PI / 2}>
            <torusGeometry args={[0.04, 0.008, 4, 8]} />
            <primitive object={trimMat} attach="material" />
          </mesh>
        </group>
      ))}

      {/* === CAPE — static draped fabric, the senator's toga === */}
      <mesh geometry={capeGeo} material={capeMat} />
      {/* Cape hem glow — neon trim at the bottom edge */}
      <mesh geometry={capeHemGeo} material={capeEdgeMat} />
      {/* Cape shoulder/neck trim — glow line along the top attachment */}
      {useMemo(() => {
        const pts: THREE.Vector3[] = [];
        for (let i = 0; i <= 12; i++) {
          const [x, y, z] = capePos(i / 12, 0.0);
          pts.push(new THREE.Vector3(x, y, z));
        }
        const geo = new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts), 12, 0.01, 4, false);
        return <mesh geometry={geo} material={capeEdgeMat} />;
      }, [])} {/* eslint-disable-line react-hooks/exhaustive-deps */}
      {/* Cape interior vertical circuit lines */}
      {capeCircuitGeos.map((geo, i) => (
        <mesh key={`cc-${i}`} geometry={geo} material={capeCircuitMat} />
      ))}
      {/* Cape horizontal panel seams */}
      {capePanelGeos.map((geo, i) => (
        <mesh key={`cp-${i}`} geometry={geo} material={capePanelMat} />
      ))}
      {/* Cape side edge trims */}
      {capeEdgeGeos.map((geo, i) => (
        <mesh key={`ce-${i}`} geometry={geo} material={capeEdgeMat} />
      ))}

      {/* === CROWN (optional) === */}
      {showCrown && <CyborgCrown trimColor={trimColor} />}
    </group>
  );
}
