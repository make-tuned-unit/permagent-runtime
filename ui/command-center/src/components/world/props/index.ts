// W2 — instanced prop library (WORLD_VIEW_BIBLE.md §5, props/ lane).
// Families register agent anchors via shared/anchors on mount. W1 mounts
// these inside the zone skeleton; legacy WorldFurniture.tsx migration follows
// after the W1 rebase.

export { WorkstationCluster } from './WorkstationCluster';
export { MezzanineBookWall, MEZZ_WALL_HEIGHT_Y, MEZZ_WALL_INNER_R } from './MezzanineBookWall';
export { BrainShelves } from './BrainShelves';
export { LabProps } from './LabProps';
export { AutomateSteles } from './AutomateSteles';
export { AmbientProps } from './AmbientProps';

export * from './geometries';
export * from './materials';
export { useRegisterAnchors, placeAnchor } from './propUtils';
