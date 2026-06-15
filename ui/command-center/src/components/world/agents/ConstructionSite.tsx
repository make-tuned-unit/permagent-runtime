// Construction site visuals — THE_CAVE_vision_bible.md §4 (the world the agents build).
// Scaffold lattice + stone courses that RISE as real banked material is spent
// (construction.ts). Glow is real-event-driven: a freshly-set stone flashes warm only on
// a stage advance, then settles. Empty bank ⇒ nothing rises (idle, not tending).
//
// Perf (bible §8): static families (scaffold poles, survey markers) use the shared
// InstancedProp (one draw call each); the rising stones use one InstancedMesh across BOTH
// sites ⇒ the whole construction system is ~3 draw calls, well inside budget. Zero
// per-frame allocations — a single module-level tmp Object3D is reused; stone matrices are
// rewritten in a useFrame only while courses exist.

import { useLayoutEffect, useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import { ENV } from '../shared/palette';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { TENDING_COLOR } from './poses';
import { SITES, STAGES_PER_SITE, tickConstruction } from './construction';
import { getReduceMotion } from '../../../styles/tokens';

const STAGE_H = 0.55; // height of one set-stone course
const STONE_W = 1.0;
const STONE_D = 1.0;
const POLE_H = STAGES_PER_SITE * STAGE_H + 0.6;
const STONE_RISE_MS = 1400; // a newly-set stone eases up into place
const FLASH_DECAY_PER_S = 0.7; // warm set-flash fade rate

const tmp = new THREE.Object3D();

interface RisingStone {
  siteIndex: number;
  course: number;
  startedAt: number;
}

export function ConstructionSite() {
  const reduceMotion = getReduceMotion();

  // Scaffold poles: 4 corner poles per site (static instanced).
  const poleTransforms = useMemo<InstanceTransform[]>(() => {
    const out: InstanceTransform[] = [];
    for (const s of SITES) {
      const [ox, , oz] = s.position;
      for (const sx of [-1, 1])
        for (const sz of [-1, 1])
          out.push({ position: [ox + sx * (STONE_W / 2 + 0.1), POLE_H / 2, oz + sz * (STONE_D / 2 + 0.1)] });
    }
    return out;
  }, []);

  // Survey-line markers on the bare footprint (the promise of a wing, bible §4).
  const surveyTransforms = useMemo<InstanceTransform[]>(
    () =>
      SITES.map((s) => ({
        position: [s.position[0], 0.012, s.position[2]],
        rotation: [-Math.PI / 2, 0, 0],
        scale: [STONE_W + 0.5, STONE_D + 0.5, 1],
      })),
    [],
  );

  const poleGeo = useMemo(() => new THREE.CylinderGeometry(0.05, 0.05, POLE_H, 6), []);
  const poleMat = useMemo(
    () => new THREE.MeshStandardMaterial({ color: ENV.bronze, roughness: 0.5, metalness: 0.7 }),
    [],
  );
  const stoneGeo = useMemo(() => new THREE.BoxGeometry(STONE_W, STAGE_H * 0.92, STONE_D), []);
  const stoneMat = useMemo(
    () =>
      new THREE.MeshStandardMaterial({
        color: ENV.marble,
        roughness: 0.6,
        metalness: 0.02,
        // Emissive carries the per-set warm flash; base 0 so settled stones stay matte.
        emissive: new THREE.Color(TENDING_COLOR),
        emissiveIntensity: 0,
      }),
    [],
  );
  const surveyGeo = useMemo(() => new THREE.RingGeometry(0.74, 0.82, 4), []);
  const surveyMat = useMemo(
    () => new THREE.MeshBasicMaterial({ color: TENDING_COLOR, transparent: true, opacity: 0.4 }),
    [],
  );

  const stonesRef = useRef<THREE.InstancedMesh>(null);
  const rising = useRef<RisingStone | null>(null);
  const flash = useRef<number[]>(SITES.map(() => 0));
  const lastCourse = useRef<number[]>(SITES.map(() => 0));
  const tickAt = useRef(0);

  // Start with no stones drawn (progress 0) — avoids a 1-frame flash of all stones
  // stacked at the origin before the first useFrame writes the real count.
  useLayoutEffect(() => {
    if (stonesRef.current) stonesRef.current.count = 0;
  }, []);

  useFrame(({ clock }, dt) => {
    const nowMs = Date.now();
    const t = clock.elapsedTime;

    // Slow discrete construction tick (not every frame) — spends the bank.
    if (t - tickAt.current >= 0.5) {
      tickAt.current = t;
      tickConstruction(nowMs);
    }

    const mesh = stonesRef.current;
    if (!mesh) return;

    let instance = 0;
    let anyRising = false;
    let maxFlash = 0;

    for (let si = 0; si < SITES.length; si++) {
      const site = SITES[si];
      const [ox, , oz] = site.position;
      const full = Math.floor(site.progress);

      // Detect a newly set course → start its rise + warm flash.
      if (full > lastCourse.current[si]) {
        rising.current = { siteIndex: si, course: full - 1, startedAt: nowMs };
        flash.current[si] = 1;
        lastCourse.current[si] = full;
      }

      for (let c = 0; c < full && c < STAGES_PER_SITE; c++) {
        let y = STAGE_H / 2 + c * STAGE_H;
        if (
          !reduceMotion &&
          rising.current &&
          rising.current.siteIndex === si &&
          rising.current.course === c
        ) {
          const k = Math.min(1, (nowMs - rising.current.startedAt) / STONE_RISE_MS);
          y = STAGE_H / 2 + c * STAGE_H + (1 - k) * 0.8;
          if (k < 1) anyRising = true;
        }
        tmp.position.set(ox, y, oz);
        tmp.rotation.set(0, 0, 0);
        tmp.scale.setScalar(1);
        tmp.updateMatrix();
        mesh.setMatrixAt(instance++, tmp.matrix);
      }

      // Decay the warm set-flash (frame-rate independent).
      if (flash.current[si] > 0) {
        flash.current[si] = Math.max(0, flash.current[si] - FLASH_DECAY_PER_S * dt);
      }
      if (flash.current[si] > maxFlash) maxFlash = flash.current[si];
    }

    mesh.count = instance;
    mesh.instanceMatrix.needsUpdate = true;
    stoneMat.emissiveIntensity = reduceMotion ? 0 : maxFlash * 0.9;

    if (rising.current && !anyRising) rising.current = null;
  });

  const maxStones = SITES.length * STAGES_PER_SITE;

  return (
    <group>
      <InstancedProp name="construction.pole" geometry={poleGeo} material={poleMat} transforms={poleTransforms} castShadow />
      <InstancedProp name="construction.survey" geometry={surveyGeo} material={surveyMat} transforms={surveyTransforms} />
      <instancedMesh ref={stonesRef} args={[stoneGeo, stoneMat, maxStones]} castShadow />
    </group>
  );
}
