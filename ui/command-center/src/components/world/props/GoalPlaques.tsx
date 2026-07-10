// GoalPlaques — the Kanban made physical: each REAL active goal claims a
// station in the working bay and hangs a holo plaque over it. Data comes only
// from agents/goalActivity.ts (the honesty boundary) — an unlit bench means
// the orchestrator really has nothing in flight.
//
// Rendering: drei <Html> cards (the established nameplate pattern from
// AgentCharacterV2) — zero extra draw calls in the GL scene, no new lights
// (bible §8: the bay's amber task lights already exist in WorkstationCluster).

import { useMemo } from 'react';
import { Html } from '@react-three/drei';
import { useActiveGoals } from '../agents/goalActivity';
import { ENV } from '../shared/palette';

const ROW_Z = [0, -3.0];
const STATION_X = [-2.4, 0, 2.4];
const PLAQUE_Y = 2.35;

interface GoalPlaquesProps {
  /** World position of the WorkstationCluster this bay annotates. */
  origin: [number, number, number];
  /** Cluster rotationY — plaque slots must match the rotated station grid. */
  rotationY?: number;
}

export function GoalPlaques({ origin, rotationY = 0 }: GoalPlaquesProps) {
  const { goals } = useActiveGoals();

  const slots = useMemo(() => {
    const cos = Math.cos(rotationY);
    const sin = Math.sin(rotationY);
    const out: [number, number, number][] = [];
    for (const rz of ROW_Z) {
      for (const sx of STATION_X) {
        out.push([
          origin[0] + sx * cos + rz * sin,
          origin[1] + PLAQUE_Y,
          origin[2] - sx * sin + rz * cos,
        ]);
      }
    }
    return out;
  }, [origin, rotationY]);

  return (
    <>
      {goals.map((goal, i) => {
        const slot = slots[i];
        if (!slot) return null;
        const working = goal.state === 'in_progress';
        const accent = working ? ENV.neonAmber : ENV.neonCyan;
        return (
          <Html
            key={goal.id}
            position={slot}
            center
            distanceFactor={14}
            style={{ pointerEvents: 'none' }}
          >
            <div
              style={{
                padding: '4px 8px',
                borderRadius: 4,
                background: 'rgba(10, 14, 26, 0.82)',
                border: `1px solid ${accent}55`,
                boxShadow: `0 0 10px ${accent}33`,
                color: '#E8E4DD',
                fontFamily: 'JetBrains Mono, monospace',
                fontSize: 9,
                maxWidth: 150,
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                textAlign: 'center',
              }}
            >
              <span style={{ color: accent, marginRight: 5 }}>
                {working ? '⚒' : '◇'}
              </span>
              {goal.title}
            </div>
          </Html>
        );
      })}
    </>
  );
}
