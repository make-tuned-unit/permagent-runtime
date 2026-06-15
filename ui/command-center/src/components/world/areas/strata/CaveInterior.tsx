// CaveInterior — the lazy descent chunk (THE_CAVE_vision_bible.md §9 W1).
// Everything beneath the crown floor: the strata cavern walls, the throat abyss,
// the glowing survey-line footprints, and the gray scaffold-mount stand-ins.
// Default export for React.lazy in CaveSystem.tsx.

import { StrataChambers } from './StrataChambers';
import { Throat } from './Throat';
import { SurveyLines } from './SurveyLines';
import { ScaffoldMounts } from './ScaffoldMounts';

export default function CaveInterior() {
  return (
    <group>
      <StrataChambers />
      <Throat />
      <SurveyLines />
      <ScaffoldMounts />
    </group>
  );
}
