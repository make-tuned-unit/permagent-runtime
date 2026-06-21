// Zones — five lazy-loaded rooms off the rotunda (WORLD_VIEW_BIBLE.md §3).
// Each interior is its own chunk (React.lazy); unloaded zones render only the
// ≤3-mesh imposter silhouette. Thresholds (portal frames + relocated Stargate)
// are always loaded so every zone reads from the hall center.

import { lazy } from 'react';
import { ZONES } from './zones';
import { ZoneMount } from './ZoneMount';
import { Thresholds } from './Thresholds';
import {
  BuildImposter,
  BrainImposter,
  LabImposter,
  AutomateImposter,
  AntechamberImposter,
} from './imposters';

const BuildZone = lazy(() => import('./build/BuildZone'));
const BrainZone = lazy(() => import('./brain/BrainZone'));
const LabZone = lazy(() => import('./lab/LabZone'));
const AutomateZone = lazy(() => import('./automate/AutomateZone'));
const AntechamberZone = lazy(() => import('./antechamber/AntechamberZone'));

const byId = Object.fromEntries(ZONES.map((z) => [z.id, z]));

export function Zones({ onClickZone }: { onClickZone: (id: string) => void }) {
  return (
    <group>
      <Thresholds />
      <ZoneMount zone={byId.build} component={BuildZone} imposter={<BuildImposter />} onClickZone={onClickZone} />
      <ZoneMount zone={byId.brain} component={BrainZone} imposter={<BrainImposter />} onClickZone={onClickZone} />
      <ZoneMount zone={byId.lab} component={LabZone} imposter={<LabImposter />} onClickZone={onClickZone} />
      <ZoneMount zone={byId.automate} component={AutomateZone} imposter={<AutomateImposter />} onClickZone={onClickZone} />
      <ZoneMount zone={byId.antechamber} component={AntechamberZone} imposter={<AntechamberImposter />} onClickZone={onClickZone} />
    </group>
  );
}
