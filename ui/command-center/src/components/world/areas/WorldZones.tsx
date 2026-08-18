// Zones — the rotunda's threshold ring (WORLD_VIEW_BIBLE.md §3).
//
// History: this once mounted five satellite zone ROOMS (Build/Brain/Lab/Automate/
// Mesh interiors) — dark blockout boxes hanging off the rotunda that read as junk.
// Those were dropped. Then a "Thresholds" ring of framed portal openings + floating
// zone labels remained, but with the rooms gone the labels pointed at nothing ("why
// does it say Build here?"). Ruling: remove ALL of it — frames, floor seams, and
// labels. The colonnade is now a clean unbroken ring (HallStructure punches only the
// one opening below) and the single thing kept is the relocated Mesh Stargate.
//
// The Stargate is now the membrane to the Agora (#306): clicking it plays the
// body⇄code arc — the camera dives through into the collective mind beyond.

import { ZONES } from './zones';
import { StargatePortal } from './antechamber/Stargate';
import { ForumPlaque } from './antechamber/ForumPlaque';
import { PortalGodRays } from './antechamber/PortalGodRays';

const MESH = ZONES.find((z) => z.id === 'antechamber')!;

export function Zones({ onClickStation }: { onClickStation?: (id: string) => void }) {
  return (
    <group>
      {/* The relocated Stargate stands in the single colonnade opening (§3 A5).
          Clicking it (or its plinth) triggers the Agora arc via 'forum-portal'. */}
      <group
        position={[Math.cos(MESH.angle) * 14.6, 0, Math.sin(MESH.angle) * 14.6]}
        rotation-y={Math.PI / 2 - MESH.angle}
      >
        <group
          onClick={(e) => {
            e.stopPropagation();
            onClickStation?.('forum-portal');
          }}
          onPointerOver={(e) => {
            e.stopPropagation();
            document.body.style.cursor = 'pointer';
          }}
          onPointerOut={(e) => {
            e.stopPropagation();
            document.body.style.cursor = 'auto';
          }}
        >
          <StargatePortal />
        </group>
        <ForumPlaque />
        {/* Volumetric-feel god-rays fanning from the event horizon (§8: additive
            sprite cones, not real volumetrics). */}
        <PortalGodRays />
      </group>
    </group>
  );
}
