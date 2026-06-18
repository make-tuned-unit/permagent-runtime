// Agent roster — identity config for the five inhabitants. WORLD_VIEW_BIBLE.md §2, §4.
// Identity (trim color, crown) is fixed here; state NEVER repaints identity trim.

import { AGENT_TRIM } from '../shared/palette';

export interface AgentIdentity {
  id: string;
  name: string;
  role: 'orchestrator' | 'agent';
  /** Identity toga-trim color — never changes with state (bible §4). */
  trimColor: string;
  isHenry: boolean;
  /** The Librarian is locked to the mezzanine ring. */
  mezzanineLocked: boolean;
  /** Spawn position (world space). */
  home: { x: number; y: number; z: number };
  /** 0-1, increases body roughness reading via darker vertex tint. */
  weathering: number;
}

export const MEZZ_RADIUS = 15.2;
export const MEZZ_Y = 10.15;

export const ROSTER: AgentIdentity[] = [
  {
    id: 'henry',
    name: 'Aria',
    role: 'orchestrator',
    trimColor: AGENT_TRIM.henry,
    isHenry: true,
    mezzanineLocked: false,
    home: { x: 0, y: 0, z: 0 },
    weathering: 0,
  },
  {
    id: 'aria',
    name: 'Aria',
    role: 'agent',
    trimColor: AGENT_TRIM.aria,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 4, y: 0, z: 2 },
    weathering: 0,
  },
  {
    id: 'felix',
    name: 'Felix',
    role: 'agent',
    trimColor: AGENT_TRIM.felix,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -3, y: 0, z: -4 },
    weathering: 0,
  },
  {
    id: 'nova',
    name: 'Nova',
    role: 'agent',
    trimColor: AGENT_TRIM.nova,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -5, y: 0, z: 3 },
    weathering: 0,
  },
  {
    id: 'librarian',
    name: 'The Librarian',
    role: 'agent',
    trimColor: AGENT_TRIM.librarian,
    isHenry: false,
    mezzanineLocked: true,
    home: { x: MEZZ_RADIUS, y: MEZZ_Y, z: 0 },
    weathering: 0.4,
  },
];

export function getIdentity(id: string): AgentIdentity | undefined {
  return ROSTER.find((a) => a.id === id);
}
