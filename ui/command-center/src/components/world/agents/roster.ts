// Agent roster — identity config for the three real inhabitants: Henry the
// orchestrator, the Reader (local OCR/ingest), and the Librarian. WORLD_VIEW_BIBLE.md
// §2, §4. Identity (trim color, crown) is fixed here; state NEVER repaints identity
// trim. The decorative sim agents (Aria/Felix/Nova) were removed — only agents that
// map to a real backend worker live here, so the AgentPicker, camera-follow, and HUDs
// all key off the same set.

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
    name: 'Henry',
    role: 'orchestrator',
    trimColor: AGENT_TRIM.henry,
    isHenry: true,
    mezzanineLocked: false,
    home: { x: 0, y: 0, z: 0 },
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
  {
    // The Reader — local OCR/document-ingest pipeline (#336/#342). Backend worker
    // gains a ground-floor presence here; renders + click-to-zoom for free via the
    // ROSTER fan-out (WorldAgents + behavior.ensureMotion + the camera follow proxy).
    // No crown (isHenry:false). State is sim-ambient for v1;
    // a real reader-event live wire is a follow-up.
    id: 'reader',
    name: 'The Reader',
    role: 'agent',
    trimColor: AGENT_TRIM.reader,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 5, y: 0, z: -2 },
    weathering: 0,
  },
];

export function getIdentity(id: string): AgentIdentity | undefined {
  return ROSTER.find((a) => a.id === id);
}
