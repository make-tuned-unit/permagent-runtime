// StratumPlaque — legibility for the descent (Carved Cave correction, C-nav).
// At each descent stop, shows the ACTIVE chamber's plaque. Knob 1: the plaque LEADS
// with the real date range + counts ("memories from [start]–[end] · N memories, M
// described"); the poetic stratum name is secondary atmosphere. Every number/date is
// quoted from the derived strata (deriveStrata data), which came from a read-only count
// of real rows — never fabricated.
//
// Only visible in descent mode. Bound to descentState's active stop index → the rail's
// stop 0 is the verge, stops 1..N map to each chamber; the floor stop shows the deepest
// chamber's plaque (no plaque of its own — it IS the bottom of the deepest band).

import type { CameraMode } from '../types';
import { COLORS } from '../constants';
import { useStrata } from '../areas/strata/strataState';
import { useDescentStop } from '../camera/descentState';
import type { StratumDef } from '../areas/strata/strata';

/** "2026-06-20T…Z" → "Jun 20, 2026". Empty/invalid → "—" (honest unknown). */
function fmtDate(iso: string): string {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

/** Map the descent stop index to the chamber it frames. */
function chamberForStop(strata: StratumDef[], stop: number): StratumDef | null {
  if (strata.length === 0) return null;
  // Stop 0 = the verge view (frames the topmost chamber). Stops 1..N = chamber i-1.
  // The final (floor) stop frames the deepest chamber.
  if (stop <= 0) return strata[0];
  const idx = Math.min(stop - 1, strata.length - 1);
  return strata[idx];
}

export function StratumPlaque({ cameraMode }: { cameraMode: CameraMode }) {
  const strata = useStrata();
  const stop = useDescentStop();

  if (cameraMode !== 'descent') return null;
  const chamber = chamberForStop(strata, stop);
  if (!chamber) return null;

  const range = `${fmtDate(chamber.dateRange.start)} – ${fmtDate(chamber.dateRange.end)}`;
  const counts =
    chamber.describedCount > 0
      ? `${chamber.memoryCount} memories, ${chamber.describedCount} described`
      : `${chamber.memoryCount} memories`;

  return (
    <div
      style={{
        position: 'absolute',
        top: '50%',
        left: 24,
        transform: 'translateY(-50%)',
        maxWidth: 320,
        background: 'rgba(10, 14, 26, 0.86)',
        border: `1px solid ${COLORS.neonCyan}40`,
        borderRadius: 8,
        padding: '14px 18px',
        fontFamily: 'monospace',
        color: COLORS.neonCyan,
        backdropFilter: 'blur(6px)',
        pointerEvents: 'none',
        zIndex: 11,
        lineHeight: 1.5,
      }}
    >
      {/* DATA LEADS (knob 1) — the real date range + counts. */}
      <div style={{ fontSize: 13, color: COLORS.primaryMarble }}>
        memories from <span style={{ color: COLORS.neonCyan }}>{range}</span>
      </div>
      <div style={{ fontSize: 12, opacity: 0.85, marginTop: 2 }}>{counts}</div>
      {/* Poetic name SECONDARY — atmosphere only. */}
      <div
        style={{
          fontSize: 11,
          letterSpacing: 1.5,
          textTransform: 'uppercase',
          color: COLORS.neonAmber,
          opacity: 0.7,
          marginTop: 8,
        }}
      >
        {chamber.label}
      </div>
    </div>
  );
}
