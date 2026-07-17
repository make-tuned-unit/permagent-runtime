// World View palette — single source of truth per WORLD_VIEW_BIBLE.md §2.
// FROZEN after Phase 0. Lanes import from here; never inline hex in world/**.

/** HUD COLOR LAW: gray idle, amber working, cyan available, red error. No green. */
export const STATE = {
  idle: '#8A94A6',
  working: '#FFB347',
  available: '#00D9FF',
  error: '#FF5D5D',
} as const;

export const ENV = {
  deepVoid: '#0A0E1A',
  marble: '#E8E4DD',
  marbleVein: '#8B7E6F',
  darkStone: '#2A2A3E',
  gunmetal: '#555B6E',
  bronze: '#A08555',
  neonCyan: '#00D9FF',
  /** Memory accent — owns the Brain area and memory-themed props. */
  violet: '#A855CC',
  neonAmber: '#FFB347',
  /** Stargate / Mesh family only. */
  horizonBlue: '#5599FF',
  floorGridGlow: '#1A4D5C',
} as const;

/** Identity trim per agent. State is never expressed by repainting identity trim. */
export const AGENT_TRIM = {
  henry: '#F0E6D0', // warm white-gold — resolves issue #87
  aria: '#FFB347',
  felix: '#FF6B9D',
  nova: '#A78BFA',
  librarian: '#8B7E6F',
  reader: '#4FD1C5', // cool teal — the OCR/document ingest pipeline (#336/#342)
  // Additive identity entry (same precedent as `reader`, added when its worker
  // shipped): the Watcher / Echo (#672), the daemon's proactive nudge worker.
  // Pale vigil steel-blue — watchful, quiet, distinct from every STATE color.
  watcher: '#9FB8D8',
} as const;

export type AgentHudState = keyof typeof STATE;
