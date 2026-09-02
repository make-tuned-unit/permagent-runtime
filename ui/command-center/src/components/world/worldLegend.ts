/**
 * What the World's key says.
 *
 * Kept apart from the HUD that draws it so the words can be tested as words,
 * and so the one line that is genuinely load-bearing — which agents are
 * reporting and which are only ambience — is read off the roster instead of
 * being retyped and left to rot. That is the mistake this file exists after:
 * the Forecaster's own source comment claimed a live wire it did not have for
 * weeks, because the claim lived in a second place.
 *
 * Every gesture below was read off the handler that implements it:
 *   drag / scroll / right-drag / arrow keys → `camera/WorldCamera.tsx`'s
 *     <OrbitControls> (rotate, wheel-zoom 8–50m, `enablePan`, `keyEvents` +
 *     `keyPanSpeed`),
 *   station click → `WorldView.tsx handleClickStation` + `pedestalNav.ts`
 *     (glide, then a tab for the three in STATION_TOOL; the Lab says
 *     "coming soon" in its own tooltip and stays glide-only),
 *   agent click → `WorldView.tsx handleSelectAgent` (opens the HUD *and*
 *     switches the camera to third-person),
 *   WASD / arrows / Esc / right-click → `WorldCamera.tsx`'s third-person key
 *     handlers.
 *
 * The walking keys are named HERE, in the key that is on screen in orbit mode,
 * because the only place they were said before was a badge that appears after
 * the camera has already switched — teaching the gesture to someone who has
 * already been surprised by it.
 */

import type { LegendRow } from '../common/CanvasLegend';
import type { CameraMode } from './types';
import { ROSTER } from './agents/roster';

/** "the Reader and the Watcher" — a plain-English list, in the roster's order. */
export function ambientAgentNames(): string {
  const names = ROSTER.filter(a => a.wire === 'sim').map(a => a.name);
  if (names.length === 0) return '';
  if (names.length === 1) return names[0];
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/**
 * Walking mode is a different control scheme, not a different camera angle:
 * `WorldCamera` unmounts <OrbitControls> entirely, so drag and scroll do
 * nothing there. The key follows the mode rather than listing every gesture
 * the hall has ever had — offering a control that is currently dead is the
 * same lie as offering one that never existed.
 */
export function worldGestures(mode: CameraMode): LegendRow[] {
  return mode === 'third-person' ? WALKING_GESTURES : ORBIT_GESTURES;
}

export const WALKING_GESTURES: LegendRow[] = [
  { term: 'WASD or the arrows', meaning: 'walks the agent you opened; the camera follows' },
  { term: 'Esc or right-click', meaning: 'takes you back to the view from above' },
];

export const ORBIT_GESTURES: LegendRow[] = [
  { term: 'Drag', meaning: 'turns the hall around you' },
  { term: 'Scroll', meaning: 'moves you closer in or further back' },
  { term: 'Right-drag or arrow keys', meaning: 'slides the view sideways' },
  {
    term: 'Click a station',
    meaning: 'glides you to it — Build, Brain and Automate then open their tab',
  },
  {
    term: 'Click an agent',
    meaning: 'opens its panel and walks you over: WASD or the arrows to walk, Esc to come back',
  },
];

export function worldVocabulary(): LegendRow[] {
  const ambient = ambientAgentNames();
  return [
    {
      term: 'A working pose',
      meaning: ambient
        ? `a real state for the agents that report in. ${ambient} have nothing reporting them yet, so their coming and going is ambience, not a claim.`
        : 'a real state — every agent in the hall reports in.',
    },
    {
      term: 'Rain, river, spring',
      meaning: 'the Brain, not decoration: rain falls as a memory is saved, the river brightens on recall, the spring rises once there is a first memory.',
    },
    {
      term: 'Marble, columns, sky',
      meaning: 'set dressing. Nothing they do means anything.',
    },
  ];
}
