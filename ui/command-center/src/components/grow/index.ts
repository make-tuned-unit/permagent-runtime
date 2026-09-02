/**
 * Grow's public surface.
 *
 * R9 split GrowView.tsx (4,684 lines) into modules by concern. This barrel is
 * what keeps that an internal rearrangement: everything outside this directory
 * imports from `components/grow`, and the file a component happens to live in
 * is ours to move again. `GrowView.tsx` still exports `GrowView` itself, so the
 * pre-split deep import keeps working too.
 */

export { GrowView } from './GrowView';
export { GrowActions } from './GrowActions';
export { GrowResults } from './GrowResults';
export { readStrategy } from './growStrategy';
export type { SavedPillar } from './growStrategy';
