// CaveInterior — the lazy descent chunk (THE_CAVE_vision_bible.md §9 W1).
// Everything beneath the crown floor: the strata cavern walls, the throat abyss,
// and the construction story off ./anchors.
//
// W1 → W2 reconnect: the gray scaffold/crane/banked/survey blockout stand-ins are
// now superseded by W2's real construction kit (CaveConstruction), driven off the
// same SCAFFOLD_MOUNTS table. Only the finished 'built' wings remain the W1
// blockout mass (BuiltWings) — W2 has no finished-wing prop yet.

import { StrataChambers } from './StrataChambers';
import { CaveFloors } from './CaveFloors';
import { Throat } from './Throat';
import { CaveConstruction } from './CaveConstruction';
import { BuiltWings } from './ScaffoldMounts';

export default function CaveInterior() {
  return (
    <group>
      {/* Walkable surface — chamber floors + connecting ramps (rebuild §5). */}
      <CaveFloors />
      <StrataChambers />
      <Throat />
      <CaveConstruction />
      <BuiltWings />
    </group>
  );
}
