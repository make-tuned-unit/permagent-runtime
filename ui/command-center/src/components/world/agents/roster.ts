// Agent roster — identity config for the real inhabitants: Henry the
// orchestrator, the Librarian, the Reader (local OCR/ingest), the Watcher
// (proactive nudges), the Steward (git hygiene), the Guard, the Financier, the
// Forecaster, and (J11) the Council, Polybot and the Picker.
// WORLD_VIEW_BIBLE.md §2, §4. Identity (trim color, crown) is fixed here; state
// NEVER repaints identity trim. The decorative sim agents (Aria/Felix/Nova)
// were removed — only agents that map to a real backend worker live here, so
// the AgentPicker, camera-follow, and HUDs all key off the same set.

import { AGENT_TRIM } from '../shared/palette';

/**
 * Where this character's on-screen state comes from. Declared HERE, next to the
 * identity, because the alternative — a hand-maintained exclusion list in
 * `stateSources` — is exactly how the Forecaster spent weeks animating off a
 * fabricated timer while its own comment (correctly) claimed a real wire.
 *
 *   daemon — a real event or poll reports this agent's state.
 *   sim    — nothing reports it; the §4 clamp holds the ambient toggler to
 *            idle/available so it can never fake work.
 *   static — nothing reports it YET. A fixed, honest resting pose: no toggler,
 *            no pulse, and a HUD that says plainly what it is waiting for.
 *            The one-line upgrade when the emitter lands is `static` → `daemon`.
 */
export type StateWire = 'daemon' | 'sim' | 'static';

export interface AgentIdentity {
  id: string;
  name: string;
  role: 'orchestrator' | 'agent';
  /** Identity toga-trim color — never changes with state (bible §4). */
  trimColor: string;
  isHenry: boolean;
  /** The Librarian is locked to the mezzanine ring. */
  mezzanineLocked: boolean;
  /** Spawn position (world space). */
  home: { x: number; y: number; z: number };
  /** 0-1, increases body roughness reading via darker vertex tint. */
  weathering: number;
  /** Where this character's state comes from. See `StateWire`. */
  wire: StateWire;
}

export const MEZZ_RADIUS = 15.2;
export const MEZZ_Y = 10.15;

export const ROSTER: AgentIdentity[] = [
  {
    id: 'henry',
    // Orchestrator: sees every desk (Finance included) and queries specialists
    // — The Financier for money, the Reader for a dropped file. `id` is a
    // stable KEY, not a label. The display name is overwritten from
    // `/api/agent/identity` by stateSources on the first poll; this is only
    // what shows before that lands, so it must not assert a name the user
    // did not choose.
    name: 'Agent',
    role: 'orchestrator',
    trimColor: AGENT_TRIM.henry,
    isHenry: true,
    mezzanineLocked: false,
    home: { x: 0, y: 0, z: 0 },
    weathering: 0,
    wire: 'daemon',
  },
  {
    id: 'librarian',
    name: 'The Librarian',
    role: 'agent',
    trimColor: AGENT_TRIM.librarian,
    isHenry: false,
    mezzanineLocked: true,
    home: { x: MEZZ_RADIUS, y: MEZZ_Y, z: 0 },
    weathering: 0.4,
    wire: 'daemon',
  },
  {
    // The Reader — local OCR/document-ingest pipeline (#336/#342). Backend worker
    // gains a ground-floor presence here; renders + click-to-zoom for free via the
    // ROSTER fan-out (WorldAgents + behavior.ensureMotion + the camera follow proxy).
    // No crown (isHenry:false). State is sim-ambient for v1;
    // a real reader-event live wire is a follow-up.
    id: 'reader',
    name: 'The Reader',
    role: 'agent',
    trimColor: AGENT_TRIM.reader,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 5, y: 0, z: -2 },
    weathering: 0,
    wire: 'sim',
  },
  {
    // The Watcher (Echo, #672) — the daemon's proactive worker: watches the
    // Brain + project news (at most one nudge a day) and, with the Financier,
    // delivers overbought sell signals on open holdings. It has no live
    // status endpoint yet, so its presence is sim-ambient (the §4 clamp holds it
    // to idle/available — it can never fake work). Its REAL moment is the
    // proactive_nudge event: the vigil beacon flares and it walks the nudge to
    // Henry (behavior.ts + props/WatcherBeacon). Home is the tower's base.
    id: 'watcher',
    name: 'The Watcher',
    role: 'agent',
    trimColor: AGENT_TRIM.watcher,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 7.9, y: 0, z: -7.2 },
    weathering: 0.25,
    wire: 'sim',
  },
  {
    // The Steward — git repo hygiene (crate::steward + the scheduled
    // steward.yaml recipe). Read/propose work runs autonomously; destructive
    // git ops are guarded in code and surfaced as approval cards. No live
    // status endpoint yet, so presence is sim-ambient like the Reader/Watcher
    // (the §4 clamp holds it honest). Home mirrors the Watcher across the
    // rotunda on the -x side.
    id: 'steward',
    name: 'The Steward',
    role: 'agent',
    trimColor: AGENT_TRIM.steward,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -7.5, y: 0, z: -6.0 },
    weathering: 0.35,
    wire: 'daemon',
  },
  {
    // The Guard — the security agent, born of the Strix engine (crate::strix +
    // the daemon sweep loop; the id/config keys keep the `strix` spelling). It
    // probes the user's OWN projects and reports; it never remediates, and
    // anything intrusive is proposed rather than performed. Unlike the
    // Reader/Watcher/Steward it HAS a live wire: the sweep emits
    // agent_state_changed, so its working pose is real, not sim-ambient.
    // Home sits opposite the Steward, on the far +x/+z quadrant.
    id: 'strix',
    name: 'The Guard',
    role: 'agent',
    trimColor: AGENT_TRIM.strix,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 8.6, y: 0, z: 5.4 },
    weathering: 0.3,
    wire: 'daemon',
  },
  {
    // The Financier — owns the Finance tab (quotes, ledger, household, scanner,
    // Polybot, tomorrow's pick). Scores overbought open lots; the Watcher
    // delivers those nudges. Opus judges the 15:30 ET close scan.
    // Reports numbers; never sizes a position. Tools announce
    // on the `financier` id, so working pose is a real wire, not sim-ambient.
    // Home sits opposite the Guard, on the far -x/+z quadrant.
    id: 'financier',
    name: 'The Financier',
    role: 'agent',
    trimColor: AGENT_TRIM.financier,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -8.2, y: 0, z: 5.0 },
    weathering: 0.2,
    wire: 'daemon',
  },
  {
    // The Forecaster — where the market around each project is going, from
    // other people's public numbers. Tools announce on the `forecaster` id, so
    // the working pose is a real wire and not sim-ambient, exactly as the
    // Financier's is. Home mirrors the Financier across +x: the two read the
    // same kind of number, one at read time and one over time.
    //
    // That was true of the daemon and false of the screen until 2026-08-31:
    // this id was never added to the exclusion list in stateSources, so the
    // ambient toggler kept flipping the avatar every 20–40 seconds and buried
    // each real announcement. If you add an agent with a wire, exclude it
    // there — a comment claiming a wire is not one.
    id: 'forecaster',
    name: 'The Forecaster',
    role: 'agent',
    trimColor: AGENT_TRIM.forecaster,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: 8.2, y: 0, z: 5.0 },
    weathering: 0.15,
    wire: 'daemon',
  },
  // ── J11: the three the user turns on and reasons about ─────────────────
  // The World is the surface built to show what the fleet is doing, and it was
  // contradicting that for everything the user had actually enabled: the
  // Council, Polybot and the Picker had no seat at all (agent-QA D-N5-1, D22).
  // The four background drivers (initiative / onboarding_coach / playbook /
  // concierge, D17) stay out by the same ruling — they are plumbing, not
  // things a person switches on and asks about.
  //
  // All three are `wire: 'static'`. None of them has an emitter yet, and a
  // seat with no emitter renders as a fixed pose with a HUD that says what it
  // is waiting for — never as a plausible one. The emitter lane's render
  // targets now exist; landing one is a `static` → `daemon` edit here plus a
  // case in stateSources.
  {
    // The Council of LLMs — every configured provider debates the same brief
    // and a chair writes the report (crate::council + council_sweep.rs). Off by
    // default, exactly like the Guard, which has had a seat here all along:
    // that asymmetry was the whole of D-N5-1. It convenes on a real cadence
    // (Sunday 22:00 local, Monday catch-up — council/due.rs), so the HUD can
    // state the schedule as a standing fact while its live state stays honestly
    // unreported. Home faces the Agora threshold: the collective-mind portal is
    // the right doorway for the one worker that IS a collective.
    id: 'council',
    name: 'The Council',
    role: 'agent',
    trimColor: AGENT_TRIM.council,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -8.5, y: 0, z: -8.5 },
    weathering: 0.3,
    wire: 'static',
  },
  {
    // Polybot — the autonomous trading process the Finance tab drives
    // (`/api/finance` → polybot.status()). It is a SEPARATE process, so
    // "running" is a fact about the machine rather than about a model: the HUD
    // reads the real board and says OFF when the board says OFF. No
    // agent_state_changed is emitted anywhere for it (D22).
    id: 'polybot',
    name: 'Polybot',
    role: 'agent',
    trimColor: AGENT_TRIM.polybot,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -10.2, y: 0, z: 3.0 },
    weathering: 0.35,
    wire: 'static',
  },
  {
    // The Picker — the close-scan desk that ranks tomorrow's candidates
    // (`picker_close_scan.rs`). It DOES announce, but under the `financier`
    // id, so today its work lights the Financier's orb and nothing lights
    // here (D22's misattribution half — a Rust-side fix). What is real and
    // readable right now is its scanner: reachable, scanning, last scan and
    // how many results, straight off the finance board. Home sits beside the
    // Financier, whose desk it works.
    id: 'picker',
    name: 'The Picker',
    role: 'agent',
    trimColor: AGENT_TRIM.picker,
    isHenry: false,
    mezzanineLocked: false,
    home: { x: -6.0, y: 0, z: 8.2 },
    weathering: 0.2,
    wire: 'static',
  },
];

/**
 * The ids the ambient toggler may animate — every entry that admits nothing
 * reports it. ONE list, derived, so a roster entry and the simulation can never
 * disagree again (see `simRoster.test.ts`).
 */
export const SIM_AGENT_IDS: readonly string[] = ROSTER
  .filter((a) => a.wire === 'sim')
  .map((a) => a.id);

export function getIdentity(id: string): AgentIdentity | undefined {
  return ROSTER.find((a) => a.id === id);
}
