// Task dais bus — the tiny shared seam between behavior (who steps onto the
// dais and when) and the TaskDais visual (the downward work-transmission beam).
// A ground agent whose HUD state flips to `working` walks onto the central
// platform first; on arrival behavior fires `triggerDaisBeam`, the beam plays
// for BEAM_MS, then the agent steps off and engages at its seat. Module-scope
// mutable record read per-frame by the beam — same pattern as watcherNudge.

export const DAIS = {
  x: 0,
  z: 0,
  /** Platform top surface — agents' waypoint Y while standing on it. */
  topY: 0.32,
  radius: 2.3,
} as const;

/** Beam duration — also how long the agent holds still on the platform. */
export const BEAM_MS = 2600;

export interface DaisBeam {
  agentId: string;
  /** Monotonic — consumers compare to detect a fresh firing. */
  seq: number;
  /** performance.now() at trigger. */
  startedAt: number;
}

let beam: DaisBeam = { agentId: '', seq: 0, startedAt: 0 };

export function triggerDaisBeam(agentId: string): void {
  beam = { agentId, seq: beam.seq + 1, startedAt: performance.now() };
}

export function getDaisBeam(): DaisBeam {
  return beam;
}
