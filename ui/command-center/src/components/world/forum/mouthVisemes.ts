/**
 * The avatar's mouth: a handful of flat shapes, no lips (decision D6).
 *
 * Each word the server says is turned into a short run of shapes from its
 * letters (vowels open the mouth, bilabials close it, round vowels purse it).
 * The run is laid over the word's own time span from the server's per-segment
 * `word_timings`. This is a coarse letter heuristic, not phonetics — the goal
 * is a believable cadence: distinct shapes, closing between words, never
 * flickering. Audio energy (agentSpeech.ts) only scales how far each shape
 * opens.
 */

export type MouthShape = 'closed' | 'small' | 'mid' | 'round' | 'wide';

/** Ellipsoid half-extents (world units, head space) for each shape. */
export const MOUTH_SHAPES: Record<MouthShape, { w: number; h: number }> = {
  closed: { w: 0.05, h: 0.004 },
  small: { w: 0.04, h: 0.012 },
  mid: { w: 0.056, h: 0.022 },
  round: { w: 0.027, h: 0.027 },
  wide: { w: 0.054, h: 0.04 },
};

/** A shape must be held at least this long to read as a shape and not a flicker. */
export const MIN_SHAPE_HOLD_MS = 75;
/** Closed beat at the end of a word, so words read as separate. */
const WORD_CLOSE_MS = 70;

export interface MouthSpan {
  startMs: number;
  endMs: number;
  shape: MouthShape;
}

interface Weighted {
  shape: MouthShape;
  weight: number;
}

const VOWEL_SHAPE: Record<string, MouthShape> = {
  a: 'wide',
  e: 'mid',
  i: 'mid',
  o: 'round',
  u: 'round',
  y: 'mid',
};

function letterShape(ch: string): Weighted {
  const vowel = VOWEL_SHAPE[ch];
  if (vowel) return { shape: vowel, weight: 2 };
  if (ch === 'm' || ch === 'b' || ch === 'p') return { shape: 'closed', weight: 0.8 };
  if (ch === 'w') return { shape: 'round', weight: 1 };
  return { shape: 'small', weight: 1 };
}

/** Coarse viseme run for one word, adjacent equal shapes merged. */
export function wordShapes(word: string): Weighted[] {
  let letters = word.toLowerCase().replace(/[^a-z]/g, '');
  // Silent final "e" after a consonant ("make", "voice") never opens the mouth.
  if (letters.length > 2 && letters.endsWith('e') && !VOWEL_SHAPE[letters[letters.length - 2]]) {
    letters = letters.slice(0, -1);
  }
  if (!letters) return [{ shape: 'mid', weight: 1 }];
  const out: Weighted[] = [];
  for (const ch of letters) {
    const next = letterShape(ch);
    const last = out[out.length - 1];
    if (last && last.shape === next.shape) last.weight += next.weight;
    else out.push({ ...next });
  }
  return out;
}

/** Merge the lightest span into its heavier neighbour until at most `max` remain. */
function fitToCount(spans: Weighted[], max: number): Weighted[] {
  const list = spans.map(s => ({ ...s }));
  while (list.length > Math.max(1, max)) {
    let lightest = 0;
    for (let i = 1; i < list.length; i++) if (list[i].weight < list[lightest].weight) lightest = i;
    const left = list[lightest - 1];
    const right = list[lightest + 1];
    const into = !left ? right : !right ? left : left.weight >= right.weight ? left : right;
    into.weight += list[lightest].weight;
    list.splice(lightest, 1);
    // Re-merge equal neighbours the removal created.
    for (let i = list.length - 1; i > 0; i--) {
      if (list[i].shape === list[i - 1].shape) {
        list[i - 1].weight += list[i].weight;
        list.splice(i, 1);
      }
    }
  }
  return list;
}

/** Lay a viseme run over every word's time span, with a closed beat after each word. */
export function buildMouthTrack(words: ReadonlyArray<{ word: string; startMs: number; endMs: number }>): MouthSpan[] {
  const track: MouthSpan[] = [];
  for (const w of words) {
    const dur = w.endMs - w.startMs;
    if (dur <= 0) continue;
    const close = dur >= MIN_SHAPE_HOLD_MS * 2.5 ? Math.min(WORD_CLOSE_MS, dur * 0.2) : 0;
    const speakMs = dur - close;
    const shapes = fitToCount(wordShapes(w.word), Math.floor(speakMs / MIN_SHAPE_HOLD_MS));
    const total = shapes.reduce((n, s) => n + s.weight, 0);
    let t = w.startMs;
    for (const s of shapes) {
      const span = (speakMs * s.weight) / total;
      track.push({ startMs: t, endMs: t + span, shape: s.shape });
      t += span;
    }
    if (close > 0) track.push({ startMs: t, endMs: w.endMs, shape: 'closed' });
  }
  return track;
}

/** Shape at `tMs` into the segment; `closed` outside any word. */
export function shapeAt(track: ReadonlyArray<MouthSpan>, tMs: number): MouthShape {
  let lo = 0;
  let hi = track.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const s = track[mid];
    if (tMs < s.startMs) hi = mid - 1;
    else if (tMs >= s.endMs) lo = mid + 1;
    else return s.shape;
  }
  return 'closed';
}
