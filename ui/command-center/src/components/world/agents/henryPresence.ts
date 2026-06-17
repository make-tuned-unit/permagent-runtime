// Henry's presence — THE_CAVE_vision_bible.md §4: "Henry never lifts a stone — he walks
// the site, inspects, presides; light gathers slightly where he stands." Henry's character
// publishes his live world position here every frame (from his own useFrame); W4's
// atmosphere reads it to gather light at his feet. This is the W3 → W4 coordinate the brief
// calls for: "publish a Henry-position read from your module if W4 needs it."
//
// A plain mutable record (no React, no allocation) so W4 can sample it in a useFrame
// without a re-render. Henry's own AgentCharacterV2 light gather (the small pool at his
// feet) reads the same record so the two never disagree.

import type { AgentHudState } from '../shared/palette';

export interface HenryPresence {
  x: number;
  y: number;
  z: number;
  /** Henry's current HUD state — lets W4 tint the gathered light to his crown gems. */
  hudState: AgentHudState;
  /** True once Henry's character has reported at least one frame. */
  live: boolean;
}

const presence: HenryPresence = { x: 0, y: 0, z: 0, hudState: 'available', live: false };

/** Called once per frame from Henry's AgentCharacterV2 useFrame. No allocation. */
export function publishHenryPosition(
  x: number,
  y: number,
  z: number,
  hudState: AgentHudState,
): void {
  presence.x = x;
  presence.y = y;
  presence.z = z;
  presence.hudState = hudState;
  presence.live = true;
}

/** W4 (and Henry's own gather light) read this — never mutate the returned object. */
export function getHenryPresence(): HenryPresence {
  return presence;
}
