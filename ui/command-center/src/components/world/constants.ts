import type { Vector3Tuple } from 'three';
import type { StationConfig } from './types';
import { NEON_ACCENT } from '../../styles/tokens';

// Color palette
export const COLORS = {
  primaryMarble: '#E8E4DD',
  marbleVeining: '#8B7E6F',
  neonCyan: NEON_ACCENT,
  neonAmber: '#FFB347',
  deepVoid: '#0A0E1A',
  floorGridGlow: '#1A4D5C',
} as const;

// Station pedestals: cardinal points at radius ~10 — threshold markers for the
// zones beyond (bible §3). Display labels follow the naming law: product tab
// names EXACTLY (Build, Brain, Automate, Mesh); the Lab keeps "Lab".
// Ids are unchanged (W2 re-themes pedestals + icons in the detail pass).
export const STATIONS: StationConfig[] = [
  {
    id: 'workbench',
    name: 'Lab',
    position: [0, 0, -10] as Vector3Tuple,
    iconType: 'planets',
    tooltip: 'Lab',
  },
  {
    id: 'library',
    name: 'Build',
    position: [10, 0, 0] as Vector3Tuple,
    iconType: 'gear',
    tooltip: 'Build',
  },
  {
    id: 'observatory',
    name: 'Brain',
    position: [0, 0, 10] as Vector3Tuple,
    iconType: 'scroll',
    tooltip: 'Brain',
  },
  {
    // Was id 'forum-portal' with a portal icon — a leftover from before the
    // Stargate was relocated to the NW colonnade opening. That id is special-
    // cased to launch the Agora arc, so clicking the AUTOMATE pedestal dove
    // you into the mesh. The pedestal is now honestly Automate (glide-to like
    // every other station, horologium rings icon); 'forum-portal' remains the
    // Stargate group's own click id (areas/WorldZones.tsx).
    id: 'automate',
    name: 'Automate',
    position: [-10, 0, 0] as Vector3Tuple,
    iconType: 'rings',
    tooltip: 'Automate',
  },
];

export const COLUMN_COUNT = 8;
export const ROTUNDA_RADIUS = 15;
export const DOME_HEIGHT = 18;
export const PLATFORM_RADIUS = 20;
