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

// Cave §4 — the construction kit (scaffold-state, "the most important visual state").
export { Scaffold, IdleCrane, StonePile, SurveyLine } from './ConstructionKit';
// Cave §3 — per-stratum prop families (primal living-cave end of the biography gradient).
export { CrudeToolRack, LibrariansLamp, MineralVein, RootMass } from './StratumProps';
// Cave §6 — the Cornerstone Gallery template kit (FORM only; content is future wiring).
export {
  SwitchbackAscent,
  ascentTop,
  GoalReliefWall,
  StatuaryPlinth,
  CrackedStone,
  CornerstoneInscription,
  CORNERSTONE_TEXT,
  LandingTerrace,
} from './CornerstoneGallery';
// Cave §4 — tending-work props (what W3's tending agents hold/use).
export { HaulingSledge, StoneSettingStation } from './TendingProps';

export * from './geometries';
export * from './materials';
export { useRegisterAnchors, placeAnchor } from './propUtils';
