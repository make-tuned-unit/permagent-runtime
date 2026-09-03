// GoalPlaques — the Kanban made physical: each REAL active goal claims a
// station in the working bay and hangs a holo plaque over it. Data comes only
// from agents/goalActivity.ts (the honesty boundary) — an unlit bench means
// the orchestrator really has nothing in flight.
//
// Rendering: drei <Html> cards (the established nameplate pattern from
// AgentCharacterV2) — zero extra draw calls in the GL scene, no new lights
// (bible §8: the bay's amber task lights already exist in WorkstationCluster).

import { useMemo, useState } from 'react';
import { Html } from '@react-three/drei';
import { useActiveGoals, type ActiveGoalLite } from '../agents/goalActivity';
import { ENV } from '../shared/palette';
import { useCommandCenter, navigateToTool } from '../../../lib/store';
import { radius } from '../../../styles/tokens';

import { Tooltip } from '../../common/Tooltip';
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
        return <PlaqueCard key={goal.id} goal={goal} position={slot} />;
      })}
    </>
  );
}

// One holo plaque. Clicking it opens the goal's detail modal through the SAME
// openGoalDetail seam the dashboard "In flight" cards use (GoalDetailModalHost is
// mounted app-wide, so the modal surfaces over the World — no tab switch, the
// diorama stays put). A goal whose lite feed lacks a project_id can't address the
// modal, so it falls back to the dashboard's In-Flight surface rather than
// dead-ending. The <Html> wrapper stays pointerEvents:none so only the small card
// itself is interactive — the surrounding scene keeps its orbit/click behavior.
function PlaqueCard({
  goal,
  position,
}: {
  goal: ActiveGoalLite;
  position: [number, number, number];
}) {
  const [hover, setHover] = useState(false);
  const openGoalDetail = useCommandCenter((s) => s.openGoalDetail);
  const working = goal.state === 'in_progress';
  const accent = working ? ENV.neonAmber : ENV.neonCyan;

  const openDetail = () => {
    if (goal.project_id) openGoalDetail(goal.project_id, goal.id);
    else navigateToTool('dashboard');
  };

  return (
    <Html position={position} center distanceFactor={14} style={{ pointerEvents: 'none' }}>
      <Tooltip content="Open goal detail">
        <div
          onClick={openDetail}
          onPointerEnter={() => setHover(true)}
          onPointerLeave={() => setHover(false)}
          style={{
            padding: '4px 8px',
            borderRadius: radius.xs,
            background: hover ? 'rgba(16, 22, 40, 0.92)' : 'rgba(10, 14, 26, 0.82)',
            border: `1px solid ${accent}${hover ? 'aa' : '55'}`,
            boxShadow: `0 0 ${hover ? 16 : 10}px ${accent}${hover ? '55' : '33'}`,
            color: '#E8E4DD',
            fontFamily: 'JetBrains Mono, monospace',
            fontSize: 10,
            maxWidth: 150,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            textAlign: 'center',
            pointerEvents: 'auto',
            cursor: 'pointer',
            transition: 'border-color 120ms, box-shadow 120ms, background 120ms',
          }}
        >
          <span style={{ color: accent, marginRight: 5 }}>{working ? '⚒' : '◇'}</span>
          {goal.title}
        </div>
      </Tooltip>
    </Html>
  );
}
