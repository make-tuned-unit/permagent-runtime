// Zones — the rotunda's threshold ring (WORLD_VIEW_BIBLE.md §3).
//
// The five satellite zone ROOMS (Build/Brain/Lab/Automate/Mesh interiors) were
// blockout "ZoneShell" boxes — dark floor/wall/ceiling slabs hanging off the rotunda
// that read as junk blocks floating outside it. They are no longer mounted; the
// direction is to GROW THE ROTUNDA itself, not satellite rooms. What stays is the
// Thresholds ring: the framed portal openings in the colonnade + their labels + the
// relocated Mesh Stargate (Thresholds owns the Stargate, so the Mesh portal is kept).

import { Thresholds } from './Thresholds';

export function Zones() {
  return (
    <group>
      <Thresholds />
    </group>
  );
}
