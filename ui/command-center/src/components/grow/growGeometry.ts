/**
 * Grow's card geometry, in one place, because the inner radii are derived from
 * it and a second copy would derive them differently.
 *
 * D4: `r_inner = max(0, r_outer - padding)`, anchored to the container. The
 * screen's cards were a 12px radius with 16px padding, which is not a
 * concentric pair — the arithmetic is negative, so every nested box was
 * strictly rounder than its container's remaining curvature. That is the
 * "pinched or flared" failure WWDC25/356 names, and it was on every card in
 * this directory because each one had copied its neighbour.
 *
 * 16 with 12 is the nearest pair on the existing scale that resolves, and it
 * resolves to `radius.xs`: a real, visibly different, DERIVED inner radius.
 * "Uniform corner radius everywhere" is the anti-slop list's flattest tell, and
 * the fix is not a bigger number — it is arithmetic.
 *
 * No second large radius is invented here. `radius.glass` stays what it is: the
 * outermost FLOATING surface, derived from the window's own corner. Grow has no
 * floating surface — every one of its panels is content — so nothing on this
 * screen wears it.
 */

import { concentric, radius, space } from '../../styles/tokens';

/** A card's own corner. */
export const CARD_R = radius.xl;
/** A card's padding — and therefore half of its children's radius arithmetic. */
export const CARD_PAD = space.xl;
/** What a direct child of a card gets. `concentric(16, 12) = 4`. */
export const CARD_INNER_R = concentric(CARD_R, CARD_PAD);

/** A denser row-shaped card (a calendar post, a growth move): shallower corner,
 *  shallower padding, and its own derived inner. `concentric(12, 10) = 2`. */
export const ROW_R = radius.lg;
export const ROW_PAD = space.lg;
export const ROW_INNER_R = concentric(ROW_R, ROW_PAD);
