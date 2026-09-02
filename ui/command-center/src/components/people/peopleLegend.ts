/**
 * What the People graph's key says.
 *
 * This graph has a premise — you at the centre, everyone else grouped by the
 * projects you share — and it had never once said so. A ring of faces with
 * floating labels is not self-explanatory, and the two things the layout
 * actually encodes were both silent: that a line between two people means a
 * shared project (`peopleGraph.ts` builds `kind: 'project'` edges from exactly
 * that), and that the larger, tinted face is someone on more than one project
 * (`isBridge`) — the person who connects two groups, which is the most useful
 * thing this view knows and the hardest to guess.
 *
 * Gestures, read off `PeopleGraphCanvas.tsx`: <OrbitControls> with its
 * defaults (drag rotates, wheel zooms, right-drag pans), `onPointerOver` and
 * `PersonFace`'s focus handling (hover or Tab lights a face and shows the
 * name), and the face's own `onClick` (opens the profile). You sit at the
 * origin and are deliberately not clickable, which the key says rather than
 * leaving someone to click themselves and wonder.
 *
 * Nothing here animates on its own — `PersonFace` moves only under hover,
 * focus or selection — so the key says that too. On a canvas where the World's
 * rain means something, "nothing here is a live reading" is information.
 */

import type { LegendRow } from '../common/CanvasLegend';
import { QUIET_AFTER_DAYS } from './contactAge';

export const PEOPLE_GESTURES: LegendRow[] = [
  { term: 'Drag', meaning: 'turns the graph around you' },
  { term: 'Scroll', meaning: 'moves you closer in or further back' },
  { term: 'Right-drag', meaning: 'slides the view sideways' },
  { term: 'Hover or Tab to a face', meaning: 'lights them up and shows their name' },
  { term: 'Click a face', meaning: 'opens their profile' },
];

export const PEOPLE_VOCABULARY: LegendRow[] = [
  {
    term: 'The middle',
    meaning: 'is you. Everyone else is grouped by a project you share — the label above each group names it.',
  },
  {
    term: 'Lines',
    meaning: 'from the middle are you to them; between two people, a project they both work on.',
  },
  {
    term: 'A bigger, brighter face',
    meaning: 'is on more than one project — they bridge the groups they sit between.',
  },
  {
    term: 'A faded face',
    meaning: `is quiet: no contact in the last ${QUIET_AFTER_DAYS} days.`,
  },
  {
    term: 'Nothing moves on its own',
    meaning: 'here — this is a picture of your directory, not a live feed.',
  },
];
