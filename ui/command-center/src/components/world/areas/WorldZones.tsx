// Zones — five lazy-loaded rooms off the rotunda (WORLD_VIEW_BIBLE.md §3).
// Each interior is its own chunk (React.lazy), loaded when the camera approaches.
// Thresholds (portal frames + relocated Stargate) are always loaded so every zone
// reads from the hall center. The old distant "imposter" silhouettes were removed —
// they read as dark shapes floating outside the rotunda before a zone loaded.

import { lazy } from 'react';
import { ZONES } from './zones';
import { ZoneMount } from './ZoneMount';
import { Thresholds } from './Thresholds';

const BuildZone = lazy(() => import('./build/BuildZone'));
const BrainZone = lazy(() => import('./brain/BrainZone'));
const LabZone = lazy(() => import('./lab/LabZone'));
const AutomateZone = lazy(() => import('./automate/AutomateZone'));
const AntechamberZone = lazy(() => import('./antechamber/AntechamberZone'));

const byId = Object.fromEntries(ZONES.map((z) => [z.id, z]));

export function Zones() {
  return (
    <group>
      <Thresholds />
      <ZoneMount zone={byId.build} component={BuildZone} imposter={null} />
      <ZoneMount zone={byId.brain} component={BrainZone} imposter={null} />
      <ZoneMount zone={byId.lab} component={LabZone} imposter={null} />
      <ZoneMount zone={byId.automate} component={AutomateZone} imposter={null} />
      <ZoneMount zone={byId.antechamber} component={AntechamberZone} imposter={null} />
    </group>
  );
}
