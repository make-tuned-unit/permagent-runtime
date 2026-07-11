// Zones — the rotunda's threshold ring (WORLD_VIEW_BIBLE.md §3).
//
// History: this once mounted five satellite zone ROOMS (Build/Brain/Lab/Automate/
// Mesh interiors) — dark blockout boxes hanging off the rotunda that read as junk.
// Those were dropped. Then a "Thresholds" ring of framed portal openings + floating
// zone labels remained, but with the rooms gone the labels pointed at nothing ("why
// does it say Build here?"). Per Jesse: remove ALL of it — frames, floor seams, and
// labels. The colonnade is now a clean unbroken ring (HallStructure punches only the
// one opening below) and the single thing kept is the relocated Mesh Stargate.

import { ZONES } from './zones';
import { StargatePortal } from './antechamber/Stargate';
import { ForumPlaque } from './antechamber/ForumPlaque';

const MESH = ZONES.find((z) => z.id === 'antechamber')!;

export function Zones() {
  return (
    <group>
      {/* The relocated Stargate stands in the single colonnade opening (§3 A5). */}
      <group
        position={[Math.cos(MESH.angle) * 14.6, 0, Math.sin(MESH.angle) * 14.6]}
        rotation-y={Math.PI / 2 - MESH.angle}
      >
        <StargatePortal />
        <ForumPlaque />
      </group>
    </group>
  );
}
