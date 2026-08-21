// Agent roster — identity config for the real inhabitants: Henry the
// orchestrator, the Librarian, the Reader (local OCR/ingest), the Watcher
// (proactive nudges), the Steward (git hygiene), the Guard, and the Financier.
// WORLD_VIEW_BIBLE.md §2, §4. Identity (trim color, crown) is fixed here; state
// NEVER repaints identity trim. The decorative sim agents (Aria/Felix/Nova)
// were removed — only agents that map to a real backend worker live here, so
// the AgentPicker, camera-follow, and HUDs all key off the same set.

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
    // `id` is a stable KEY, not a label. The display name is overwritten
    // from `/api/agent/identity` by stateSources on the first poll; this is
    // only what shows before that lands, so it must not assert a name the
    // user did not choose.
    name: 'Agent',
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
  {
    // The Watcher (Echo, #672) — the daemon's proactive worker: watches the
    // Brain + project news and surfaces at most one nudge a day. It has no live
    // status endpoint yet, so its presence is sim-ambient (the §4 clamp holds it
    // to idle/available — it can never fake work). Its REAL moment is the
    // proactive_nudge event: the vigil beacon flares and it walks the nudge to
    // Henry (behavior.ts + props/WatcherBeacon). Home is the tower's base.
    id: 'watcher',
    name: 'The Watcher',
    role: 'agent',
    trimColor: AGENT_TRIM.watcher,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 7.9, y: 0, z: -7.2 },
    weathering: 0.25,
  },
  {
    // The Steward — git repo hygiene (crate::steward + the scheduled
    // steward.yaml recipe). Read/propose work runs autonomously; destructive
    // git ops are guarded in code and surfaced as approval cards. No live
    // status endpoint yet, so presence is sim-ambient like the Reader/Watcher
    // (the §4 clamp holds it honest). Home mirrors the Watcher across the
    // rotunda on the -x side.
    id: 'steward',
    name: 'The Steward',
    role: 'agent',
    trimColor: AGENT_TRIM.steward,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -7.5, y: 0, z: -6.0 },
    weathering: 0.35,
  },
  {
    // The Guard — the security agent, born of the Strix engine (crate::strix +
    // the daemon sweep loop; the id/config keys keep the `strix` spelling). It
    // probes the user's OWN projects and reports; it never remediates, and
    // anything intrusive is proposed rather than performed. Unlike the
    // Reader/Watcher/Steward it HAS a live wire: the sweep emits
    // agent_state_changed, so its working pose is real, not sim-ambient.
    // Home sits opposite the Steward, on the far +x/+z quadrant.
    id: 'strix',
    name: 'The Guard',
    role: 'agent',
    trimColor: AGENT_TRIM.strix,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 8.6, y: 0, z: 5.4 },
    weathering: 0.3,
  },
  {
    // The Financier — market research and the Finance tab ledger. Reports
    // numbers; never sizes a position and cannot place an order. Tools announce
    // on the `financier` id, so working pose is a real wire, not sim-ambient.
    // Home sits opposite the Guard, on the far -x/+z quadrant.
    id: 'financier',
    name: 'The Financier',
    role: 'agent',
    trimColor: AGENT_TRIM.financier,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -8.2, y: 0, z: 5.0 },
    weathering: 0.2,
  },
];

export function getIdentity(id: string): AgentIdentity | undefined {
  return ROSTER.find((a) => a.id === id);
}
