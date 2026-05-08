import type { Vector3Tuple } from 'three';
import type { StationConfig } from './types';

// Color palette
export const COLORS = {
  primaryMarble: '#E8E4DD',
  marbleVeining: '#8B7E6F',
  neonCyan: '#00D9FF',
  neonAmber: '#FFB347',
  deepVoid: '#0A0E1A',
  floorGridGlow: '#1A4D5C',
} as const;

// Station layout: cardinal points, radius ~10 from center
export const STATIONS: StationConfig[] = [
  {
    id: 'workbench',
    name: 'Workbench',
    position: [0, 0, -10] as Vector3Tuple,
    iconType: 'gear',
    tooltip: 'Workbench',
  },
  {
    id: 'library',
    name: 'Library',
    position: [10, 0, 0] as Vector3Tuple,
    iconType: 'scroll',
    tooltip: 'Library (Mezzanine) - The Brain',
  },
  {
    id: 'observatory',
    name: 'Observatory',
    position: [0, 0, 10] as Vector3Tuple,
    iconType: 'planets',
    tooltip: 'Observatory',
  },
  {
    id: 'forum-portal',
    name: 'Forum Portal',
    position: [-10, 0, 0] as Vector3Tuple,
    iconType: 'portal',
    tooltip: 'Mesh Forum (coming soon)',
    disabled: true,
  },
];

export const COLUMN_COUNT = 8;
export const ROTUNDA_RADIUS = 15;
export const DOME_HEIGHT = 18;
export const PLATFORM_RADIUS = 20;
