// AutomateSteles — the Automate Hall's scheduler gallery (bible §3 A4), bound
// to the real scheduler (agent-QA D19).
//
// One stele per registered job, engraved with a timetable grid whose colour is
// that job's actual last outcome, labelled with the job's own name, and pulsing
// only while a run is genuinely in flight. The room used to be four stone
// masses with a deterministic ~20% of cells lit amber on a hardcoded 4-second
// clock — identical whether the scheduler had twelve healthy automations, one
// failing one, or none at all. The pulse was the loudest thing in the room and
// it meant nothing.
//
// The materials this now uses were written for exactly this and left unused:
// materials.ts documents `lightErrorTick` as "a schedule that errored/missed"
// and `lightIdleTick` as "a real-but-dormant unit (an idle or paused
// schedule)". The vocabulary was already there; nothing consumed it.
//
// Draw calls: 5 instanced (steles · bronze bases · healthy cells · failed
// cells · dormant cells) + at most one amber tick group + one table + one plate
// per stele.
// Anchors: one lean anchor per rendered stele + 2 stand anchors at the table.
// Motion: the amber tick, and ONLY while `jobs` reports something running.

import { useMemo } from 'react';
import { useFrame } from '@react-three/fiber';
import { getReduceMotion } from '../../../styles/tokens';
import type { AreaId } from '../shared/anchors';
import type { AgentAnchor } from '../shared/anchors';
import { InstancedProp, type InstanceTransform } from '../shared/instancing';
import { unitBox } from './geometries';
import {
  stoneDark, stoneMarble, metalBronze,
  lightCyan, lightAmberTick, lightErrorTick, lightIdleTick,
} from './materials';
import { placeAnchor, useRegisterAnchors } from './propUtils';
import { TextPlate } from '../areas/TextPlate';
import { COLORS } from '../constants';
import type { HallJob } from '../areas/automate/scheduleActivity';

const STELE_SPACING = 1.7;
const STELE_H = 3.2;
const GRID_ROWS = 6;
const GRID_COLS = 4;

/** Centre-out x for the i-th of n steles. */
function steleX(i: number, n: number): number {
  return (i - (n - 1) / 2) * STELE_SPACING;
}

/**
 * The pulse. It exists only while a run is in flight — `active` false leaves
 * the shared amber singleton parked at its resting value rather than breathing
 * on a fabricated clock.
 */
function TickPulse({ active }: { active: boolean }) {
  const reduceMotion = useMemo(() => getReduceMotion(), []);
  useFrame(({ clock }) => {
    if (reduceMotion) return;
    const target = active
      ? ((clock.elapsedTime % 4) / 4 < 0.5 ? 1.9 : 1.1)
      : 1.5;
    lightAmberTick.emissiveIntensity += (target - lightAmberTick.emissiveIntensity) * 0.08;
  });
  return null;
}

interface AutomateStelesProps {
  /** The real registered jobs, from `areas/automate/scheduleActivity`. An
   *  empty list renders an empty gallery — the room never invents a stele. */
  jobs: HallJob[];
  position?: [number, number, number];
  rotationY?: number;
  areaId?: AreaId;
  idPrefix?: string;
}

/**
 * The layout, pure and pinned: which cells light, in which material, for which
 * job. Every claim the gallery makes is decided here — a cell in the `running`
 * bucket is the only thing that will pulse, so nothing may land in it that the
 * daemon did not report as in flight.
 */
export function buildSteleLayout(jobs: HallJob[]) {
  const steles: InstanceTransform[] = [];
  const bases: InstanceTransform[] = [];
  // Cells by what they are reporting — one instanced group per material, so
  // a stele's colour IS its outcome and no cell can be lit by accident.
  const healthy: InstanceTransform[] = [];
  const failed: InstanceTransform[] = [];
  const dormant: InstanceTransform[] = [];
  const running: InstanceTransform[] = [];
  const marbleWork: InstanceTransform[] = [];
  const channels: InstanceTransform[] = [];

  jobs.forEach((job, s) => {
    const x = steleX(s, jobs.length);
    steles.push({ position: [x, STELE_H / 2 + 0.18, -2.5], scale: [1.25, STELE_H, 0.3] });
    bases.push({ position: [x, 0.09, -2.5], scale: [1.45, 0.18, 0.5] });

    const bucket =
      job.outcome === 'failed' || job.outcome === 'missed' ? failed
        : job.outcome === 'never' || job.outcome === 'off' ? dormant
          : healthy;

    for (let row = 0; row < GRID_ROWS; row++) {
      for (let col = 0; col < GRID_COLS; col++) {
        const cell: InstanceTransform = {
          position: [x - 0.42 + col * 0.28, 0.95 + row * 0.42, -2.34],
          scale: [0.2, 0.07, 0.02],
        };
        // The top row of a running job's grid is its in-flight row: the one
        // thing in this room permitted to pulse, and only while the daemon
        // says `currently_running`.
        if (job.running && row === GRID_ROWS - 1) running.push(cell);
        else bucket.push(cell);
      }
    }
  });

  if (jobs.length > 0) {
    // Planning table: long dark stone top on marble slabs.
    marbleWork.push({ position: [-2.2, 0.42, 1.2], scale: [0.2, 0.84, 0.9] });
    marbleWork.push({ position: [2.2, 0.42, 1.2], scale: [0.2, 0.84, 0.9] });
    // Floor channel linking the steles to the table.
    channels.push({ position: [0, 0.012, -1.0], scale: [9.4, 0.02, 0.08] });
  }

  return { steles, bases, healthy, failed, dormant, running, marbleWork, channels };
}

export function AutomateSteles({
  jobs,
  position = [0, 0, 0],
  rotationY = 0,
  areaId = 'automate',
  idPrefix = 'automate',
}: AutomateStelesProps) {
  const layout = useMemo(() => buildSteleLayout(jobs), [jobs]);

  const anchors = useMemo<AgentAnchor[]>(() => {
    const list: AgentAnchor[] = jobs.map((_, i) => ({
      id: `${idPrefix}.stele${i + 1}.lean`,
      areaId,
      kind: 'lean' as const,
      ...placeAnchor([steleX(i, jobs.length), 0, -1.7], Math.PI, position, rotationY),
    }));
    list.push({
      id: `${idPrefix}.table.standW`,
      areaId,
      kind: 'stand',
      ...placeAnchor([-1.2, 0, 2.1], Math.PI, position, rotationY),
    });
    list.push({
      id: `${idPrefix}.table.standE`,
      areaId,
      kind: 'stand',
      ...placeAnchor([1.2, 0, 2.1], Math.PI, position, rotationY),
    });
    return list;
  }, [areaId, idPrefix, position, rotationY, jobs]);
  useRegisterAnchors(anchors);

  return (
    <group position={position} rotation-y={rotationY}>
      <TickPulse active={jobs.some((j) => j.running)} />
      {layout.steles.length > 0 && (
        <>
          <InstancedProp name={`${idPrefix}.stele`} geometry={unitBox} material={stoneDark} transforms={layout.steles} castShadow />
          <InstancedProp name={`${idPrefix}.base`} geometry={unitBox} material={metalBronze} transforms={layout.bases} />
        </>
      )}
      {layout.healthy.length > 0 && (
        <InstancedProp name={`${idPrefix}.cell.ok`} geometry={unitBox} material={lightCyan} transforms={layout.healthy} />
      )}
      {layout.failed.length > 0 && (
        <InstancedProp name={`${idPrefix}.cell.failed`} geometry={unitBox} material={lightErrorTick} transforms={layout.failed} />
      )}
      {layout.dormant.length > 0 && (
        <InstancedProp name={`${idPrefix}.cell.dormant`} geometry={unitBox} material={lightIdleTick} transforms={layout.dormant} />
      )}
      {layout.running.length > 0 && (
        <InstancedProp name={`${idPrefix}.cell.running`} geometry={unitBox} material={lightAmberTick} transforms={layout.running} />
      )}
      {layout.marbleWork.length > 0 && (
        <>
          <InstancedProp name={`${idPrefix}.marbleWork`} geometry={unitBox} material={stoneMarble} transforms={layout.marbleWork} />
          <InstancedProp name={`${idPrefix}.floorChannel`} geometry={unitBox} material={lightCyan} transforms={layout.channels} />
          <mesh position={[0, 0.88, 1.2]} scale={[5.2, 0.1, 1.1]} geometry={unitBox} material={stoneDark} castShadow />
        </>
      )}
      {/* A stele that says which automation it is. Without the name the room
          could show state and still not tell you whose state it was. */}
      {jobs.map((job, i) => (
        <TextPlate
          key={job.id}
          text={plateText(job)}
          height={0.26}
          color={job.running ? COLORS.neonAmber : COLORS.primaryMarble}
          opacity={0.85}
          position={[steleX(i, jobs.length), 0.62, -2.32]}
        />
      ))}
    </group>
  );
}

/** Short enough to engrave; the HUD-free room has no room for a sentence. */
export function plateText(job: HallJob): string {
  const name = job.label.length > 18 ? `${job.label.slice(0, 17)}…` : job.label;
  return job.running ? `${name} · RUNNING` : name;
}
