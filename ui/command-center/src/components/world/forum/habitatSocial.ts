import { HABITAT_DECOR, HABITAT_PLACES, HABITAT_STATIONS, HANK_TABLE_SEAT, habitatElevation, habitatWalkable, onGardenBridge, stationFor, type Point3 } from './habitatLayout';
import { GARDEN_AGENT_SEATS } from './gardenLayout';
import { getForumMotion } from './ForumInhabitants';
import {
  clearAmbientHabitatTravel,
  commandHabitatAgent,
  findHabitatPath,
  getHabitatTravel,
  hasHabitatUserHold,
} from './habitatMotion';

export type SocialPhase = 'idle' | 'stroll' | 'dwell' | 'approach' | 'ready' | 'face' | 'gesture' | 'return';

export type SocialInput = {
  dt: number;
  /** When false (hidden World), freeze clock — no backlog catchup. */
  active: boolean;
  /** When true, freeze clock, cancel ambient paths, no heading motion. */
  reducedMotion: boolean;
  conversation: boolean;
  liveWorking: ReadonlySet<string> | readonly string[];
  following: string | null;
  directing: string | null;
};

type AgentSocial = {
  id: string;
  phase: SocialPhase;
  phaseUntil: number;
  partnerId: string | null;
  meetingId: string | null;
  target: Point3 | null;
  home: Point3;
  replans: number;
};

type MeetingReservation = {
  id: string;
  slot: Point3;
  standA: Point3;
  standB: Point3;
  agents: [string, string];
  until: number;
};

type PlannedStroll = { kind: 'stroll'; agent: string; at: number; point: Point3 };
type PlannedMeet = { kind: 'meet'; agents: [string, string]; at: number; slot: Point3; standA: Point3; standB: Point3 };
type Planned = PlannedStroll | PlannedMeet;

const FACE_SEC = 2.2;
const GESTURE_SEC = 3.4;
const DWELL_SEC = 2.8;
const MEET_TIMEOUT = 55;
const RETURN_TIMEOUT = 70;
const CYCLE_SEC = 120;
const MAX_CANDIDATES = 10;
const STAND_SEPARATION = 0.84;
const TURN_RATE = 4.5;
const RETURN_LABEL = 'Returning to station';
const STROLL_LABEL = 'Strolling';
const MEET_LABEL = 'Meeting briefly';
/** Strollers never include meet participants in the same cycle window. */
const STROLLERS = ['picker', 'steward', 'strix', 'forecaster', 'polybot'] as const;
const MEET_PAIR: [string, string] = ['reader', 'council'];

let clock = 0;
let pathCommands = 0;
let candidates: Point3[] = [];
let plan: Planned[] = [];
let planCursor = 0;
let cycleIndex = 0;
let planningMs = 0;
/** True once a zero-candidate (or persistently empty) plan is known — never spin append. */
let planIdle = false;
const agents = new Map<string, AgentSocial>();
let reservation: MeetingReservation | null = null;
let ready = false;

function hashUnit(id: string, salt: number): number {
  let h = salt | 0;
  for (let i = 0; i < id.length; i++) h = Math.imul(h ^ id.charCodeAt(i), 16777619);
  return ((h >>> 0) % 10_000) / 10_000;
}

function elevate(x: number, z: number): Point3 {
  return [x, habitatElevation(x, z), z];
}

/** Reject bridge choke points, water court, and the conversation seat. */
export function isAmbientSafePoint(x: number, z: number): boolean {
  if (onGardenBridge(x, z)) return false;
  if (!habitatWalkable(x, z, 0.5)) return false;
  if (Math.abs(x - 4) < 2.35 && z > -9.6 && z < -0.35) return false;
  if (Math.hypot(x - HANK_TABLE_SEAT[0], z - HANK_TABLE_SEAT[2]) < 1.15) return false;
  return true;
}

export function proveAmbientRoute(from: Point3, to: Point3, occupied: readonly Point3[] = []): Point3[] | null {
  if (!isAmbientSafePoint(to[0], to[2])) return null;
  return findHabitatPath(from, to, occupied);
}

function rawCandidatePool(): Point3[] {
  const points: Point3[] = [];
  const push = (x: number, z: number) => {
    if (!isAmbientSafePoint(x, z)) return;
    points.push(elevate(x, z));
  };
  push(0.5, 4.8);
  for (const place of HABITAT_PLACES) {
    if (place.id === 'commons') continue;
    push(place.position[0], place.position[2] + 1.55);
  }
  for (const decor of HABITAT_DECOR) {
    if (decor.asset !== 'bench') continue;
    const hx = Math.cos(decor.heading), hz = Math.sin(decor.heading);
    push(decor.position[0] + hz * 1.25, decor.position[2] + hx * 1.25);
  }
  // Prefer a few lane seats; skip dense GARDEN_AGENT_SEATS to bound A* cost.
  for (const seat of GARDEN_AGENT_SEATS.slice(0, 4)) push(seat[0], seat[2]);
  for (const [x, z] of [[-7.2, 6.2], [8.2, 5.5], [-2.4, -14.2], [5.5, -14.5]] as [number, number][]) {
    push(x, z);
  }
  return points;
}

function uniquePoints(points: Point3[]): Point3[] {
  const seen = new Set<string>();
  const out: Point3[] = [];
  for (const p of points) {
    const k = `${p[0].toFixed(2)},${p[2].toFixed(2)}`;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(p);
  }
  return out;
}

/** Build a bounded set of path-proven ambient destinations. */
export function buildAmbientCandidates(): Point3[] {
  const pool = uniquePoints(rawCandidatePool());
  const proven: Point3[] = [];
  const probes = [stationFor('picker'), stationFor('steward'), stationFor('henry'), stationFor('council')];
  for (const point of pool) {
    if (proven.length >= MAX_CANDIDATES) break;
    let reaches = 0;
    for (const station of probes) {
      if (proveAmbientRoute(station.position, point)) reaches++;
      if (reaches >= 2) break;
    }
    if (reaches >= 2) proven.push(point);
  }
  return proven;
}

/** Distinct meeting stands with >= STAND_SEPARATION; null if impossible. */
export function meetingStands(slot: Point3): [Point3, Point3] | null {
  const offsets: [number, number][] = [
    [-0.7, 0.2], [0.7, -0.2],
    [-0.9, 0], [0.9, 0],
    [0, -0.9], [0, 0.9],
  ];
  for (let i = 0; i < offsets.length; i++) {
    for (let j = i + 1; j < offsets.length; j++) {
      const a = elevate(slot[0] + offsets[i][0], slot[2] + offsets[i][1]);
      const b = elevate(slot[0] + offsets[j][0], slot[2] + offsets[j][1]);
      if (!isAmbientSafePoint(a[0], a[2]) || !isAmbientSafePoint(b[0], b[2])) continue;
      if (Math.hypot(a[0] - b[0], a[2] - b[2]) < STAND_SEPARATION) continue;
      return [a, b];
    }
  }
  return null;
}

function appendCycle(points: Point3[], index: number): Planned[] {
  if (points.length === 0) return [];
  const base = index * CYCLE_SEC;
  const events: Planned[] = [];
  for (let i = 0; i < STROLLERS.length; i++) {
    const id = STROLLERS[i];
    const at = base + 8 + i * 18 + hashUnit(id, 11 + index) * 4;
    const point = points[Math.floor(hashUnit(id, 29 + index) * points.length) % points.length];
    if (!proveAmbientRoute(stationFor(id).position, point)) continue;
    events.push({ kind: 'stroll', agent: id, at, point });
  }
  let meet: PlannedMeet | null = null;
  for (const point of points) {
    const stands = meetingStands(point);
    if (!stands) continue;
    const [sa, sb] = stands;
    if (!proveAmbientRoute(stationFor(MEET_PAIR[0]).position, sa)) continue;
    if (!proveAmbientRoute(stationFor(MEET_PAIR[1]).position, sb)) continue;
    meet = {
      kind: 'meet',
      agents: MEET_PAIR,
      at: base + 36 + hashUnit(MEET_PAIR[0], 7 + index) * 6,
      slot: point,
      standA: sa,
      standB: sb,
    };
    break;
  }
  if (meet) events.push(meet);
  const henryPoint = points[Math.floor(hashUnit('henry', 41 + index) * points.length) % points.length];
  if (proveAmbientRoute(stationFor('henry').position, henryPoint)) {
    events.push({ kind: 'stroll', agent: 'henry', at: base + 100, point: henryPoint });
  }
  return events.sort((a, b) => a.at - b.at);
}

function ensureAgents() {
  for (const station of HABITAT_STATIONS) {
    if (agents.has(station.id)) continue;
    agents.set(station.id, {
      id: station.id,
      phase: 'idle',
      phaseUntil: 0,
      partnerId: null,
      meetingId: null,
      target: null,
      home: [...station.position] as Point3,
      replans: 0,
    });
  }
}

/** One-time candidate/plan prep — call from mount, not from every getter. */
export function prepareHabitatSocial(): number {
  if (ready) return planningMs;
  const started = typeof performance !== 'undefined' ? performance.now() : Date.now();
  ensureAgents();
  candidates = buildAmbientCandidates();
  plan = appendCycle(candidates, 0);
  planCursor = 0;
  cycleIndex = 1;
  planIdle = candidates.length === 0;
  ready = true;
  planningMs = (typeof performance !== 'undefined' ? performance.now() : Date.now()) - started;
  return planningMs;
}

function pruneConsumedPlan() {
  if (planCursor <= 0) return;
  plan = plan.slice(planCursor);
  planCursor = 0;
}

/** At most one bounded append per tick. Empty candidates → idle; never while-spin. */
function ensurePlanAhead() {
  if (!ready) prepareHabitatSocial();
  pruneConsumedPlan();
  if (planIdle) return;
  if (candidates.length === 0) {
    planIdle = true;
    return;
  }
  const horizon = clock + CYCLE_SEC * 0.5;
  if (plan.length > 0 && plan[plan.length - 1].at >= horizon) return;
  const next = appendCycle(candidates, cycleIndex++);
  if (next.length === 0) {
    // No valid events this cycle — idle rather than retrying forever in one frame.
    // Future ticks may still try later cycle indices only via a single append each.
    return;
  }
  plan.push(...next);
  plan.sort((a, b) => a.at - b.at);
}

function workingSet(input: SocialInput): Set<string> {
  return input.liveWorking instanceof Set ? input.liveWorking : new Set(input.liveWorking);
}

function isPreempted(id: string, input: SocialInput, working: Set<string>): boolean {
  if (hasHabitatUserHold(id)) return true;
  if (input.following === id || input.directing === id) return true;
  if (input.conversation && id === 'henry') return true;
  if (working.has(id)) return true;
  const travel = getHabitatTravel(id);
  if (travel && (travel.source === 'user' || travel.source === 'work')) return true;
  return false;
}

function clearAmbientPose(id: string) {
  const motion = getForumMotion(id);
  if (motion) motion.engaged = 'none';
}

function resetAgent(social: AgentSocial) {
  social.phase = 'idle';
  social.phaseUntil = 0;
  social.partnerId = null;
  social.meetingId = null;
  social.target = null;
  social.replans = 0;
  clearAmbientPose(social.id);
}

function stopAmbientTravelOnly(id: string) {
  clearAmbientHabitatTravel(id);
}

/** Atomic meeting teardown: reset both agents and stop only ambient-owned routes. */
function releaseMeeting(meetingId: string | null, stopTravel: boolean) {
  if (!meetingId) return;
  const res = reservation && reservation.id === meetingId ? reservation : null;
  const pair = res?.agents ?? [...agents.values()].filter(a => a.meetingId === meetingId).map(a => a.id);
  if (reservation && reservation.id === meetingId) reservation = null;
  for (const id of pair) {
    const social = agents.get(id);
    if (!social) continue;
    if (stopTravel) stopAmbientTravelOnly(id);
    if (social.meetingId === meetingId || social.phase !== 'idle') resetAgent(social);
  }
}

function cancelAmbient(id: string, stopTravel: boolean) {
  const social = agents.get(id);
  if (!social) {
    if (stopTravel) stopAmbientTravelOnly(id);
    clearAmbientPose(id);
    return;
  }
  if (social.meetingId) {
    releaseMeeting(social.meetingId, stopTravel);
    // Partner teardown handled; ensure this id's ambient travel cleared if still ambient.
    if (stopTravel) stopAmbientTravelOnly(id);
    if (social.phase !== 'idle') resetAgent(social);
    return;
  }
  if (stopTravel) stopAmbientTravelOnly(id);
  if (social.phase !== 'idle') resetAgent(social);
  else clearAmbientPose(id);
}

function issueAmbient(id: string, to: Point3, label: string): boolean {
  if (!isAmbientSafePoint(to[0], to[2])) return false;
  const ok = commandHabitatAgent(id, to, label, false, 'ambient');
  if (ok) pathCommands++;
  return ok;
}

/** Station homes are intentional return endpoints — skip stroll safety filter; mark workArrival for label. */
function issueAmbientReturn(id: string, home: Point3): boolean {
  const ok = commandHabitatAgent(id, home, RETURN_LABEL, true, 'ambient');
  if (ok) pathCommands++;
  return ok;
}

/** True when a station return destination is path-reachable from the agent (not stroll-filtered). */
export function canAmbientReturnHome(id: string): boolean {
  const motion = getForumMotion(id);
  const home = stationFor(id).position;
  if (!motion) return false;
  return !!findHabitatPath([motion.x, motion.y, motion.z], home);
}

function easeFacePartner(id: string, partnerId: string, dt: number) {
  const motion = getForumMotion(id);
  const partner = getForumMotion(partnerId);
  if (!motion || !partner) return;
  const yaw = Math.atan2(partner.x - motion.x, partner.z - motion.z);
  const delta = Math.atan2(Math.sin(yaw - motion.heading), Math.cos(yaw - motion.heading));
  motion.heading += delta * (1 - Math.exp(-dt * TURN_RATE));
  motion.engaged = 'standing';
}

function arrivedOrIdle(id: string): boolean {
  return !getHabitatTravel(id)?.moving;
}

function near(id: string, point: Point3, radius = 0.55): boolean {
  const motion = getForumMotion(id);
  if (!motion) return false;
  return Math.hypot(motion.x - point[0], motion.z - point[2]) < radius;
}

function partnerReady(partnerId: string | null): boolean {
  if (!partnerId) return false;
  const partner = agents.get(partnerId);
  return !!partner && (partner.phase === 'ready' || partner.phase === 'face' || partner.phase === 'gesture');
}

function startStroll(id: string, point: Point3) {
  const social = agents.get(id);
  if (!social || social.phase !== 'idle') return;
  if (!issueAmbient(id, point, STROLL_LABEL)) return;
  social.phase = 'stroll';
  social.target = point;
  social.phaseUntil = clock + 55;
  social.replans = 0;
}

function startMeet(event: PlannedMeet) {
  if (reservation) return false;
  const [a, b] = event.agents;
  const socialA = agents.get(a), socialB = agents.get(b);
  if (!socialA || !socialB || socialA.phase !== 'idle' || socialB.phase !== 'idle') return false;
  if (Math.hypot(event.standA[0] - event.standB[0], event.standA[2] - event.standB[2]) < STAND_SEPARATION) return false;

  const goA = issueAmbient(a, event.standA, MEET_LABEL);
  const goB = issueAmbient(b, event.standB, MEET_LABEL);
  if (!goA || !goB) {
    // Atomic rollback — only ambient-owned routes are cleared.
    if (goA) stopAmbientTravelOnly(a);
    if (goB) stopAmbientTravelOnly(b);
    resetAgent(socialA);
    resetAgent(socialB);
    return false;
  }
  const id = `meet:${a}:${b}:${Math.floor(event.at)}`;
  reservation = { id, slot: event.slot, standA: event.standA, standB: event.standB, agents: [a, b], until: clock + MEET_TIMEOUT };
  for (const [social, stand, partner] of [[socialA, event.standA, b], [socialB, event.standB, a]] as const) {
    social.phase = 'approach';
    social.target = stand;
    social.partnerId = partner;
    social.meetingId = id;
    social.phaseUntil = reservation.until;
    social.replans = 0;
  }
  return true;
}

function beginReturn(social: AgentSocial) {
  const meetingId = social.meetingId;
  clearAmbientPose(social.id);
  social.partnerId = null;
  social.meetingId = null;
  social.target = social.home;
  social.phase = 'return';
  social.phaseUntil = clock + RETURN_TIMEOUT;
  social.replans = 0;
  if (meetingId && reservation?.id === meetingId) {
    // Leaving the meeting releases the reservation; partner continues independently to return.
    reservation = null;
  }
  if (!issueAmbientReturn(social.id, social.home)) {
    // Failed return must stop any stale outbound ambient route before idling.
    stopAmbientTravelOnly(social.id);
    resetAgent(social);
  }
}

function bothReturnFromMeeting(meetingId: string) {
  const pair = reservation?.id === meetingId ? reservation.agents : null;
  if (reservation?.id === meetingId) reservation = null;
  const ids = pair ?? [...agents.values()].filter(a => a.meetingId === meetingId).map(a => a.id);
  for (const id of ids) {
    const social = agents.get(id);
    if (social) beginReturn(social);
  }
}

function advanceAgent(social: AgentSocial, input: SocialInput, working: Set<string>, dt: number) {
  if (isPreempted(social.id, input, working)) {
    cancelAmbient(social.id, true);
    return;
  }
  if (social.phase === 'idle') return;

  if (social.phase === 'stroll') {
    if (clock > social.phaseUntil) {
      beginReturn(social);
      return;
    }
    if (arrivedOrIdle(social.id)) {
      if (social.target && near(social.id, social.target, 0.55)) {
        social.phase = 'dwell';
        social.phaseUntil = clock + DWELL_SEC;
      } else if (social.replans < 1 && social.target) {
        social.replans++;
        if (!issueAmbient(social.id, social.target, STROLL_LABEL)) beginReturn(social);
      } else {
        beginReturn(social);
      }
    }
    return;
  }

  if (social.phase === 'dwell') {
    if (clock >= social.phaseUntil) beginReturn(social);
    return;
  }

  if (social.phase === 'approach') {
    if (clock > social.phaseUntil) {
      bothReturnFromMeeting(social.meetingId ?? '');
      return;
    }
    if (arrivedOrIdle(social.id) && social.target && near(social.id, social.target, 0.55)) {
      social.phase = 'ready';
      social.phaseUntil = reservation?.until ?? social.phaseUntil;
      return;
    }
    if (arrivedOrIdle(social.id) && social.target) {
      if (social.replans < 1) {
        social.replans++;
        if (!issueAmbient(social.id, social.target, MEET_LABEL)) {
          releaseMeeting(social.meetingId, true);
          beginReturn(social);
        }
      } else {
        releaseMeeting(social.meetingId, true);
        beginReturn(social);
      }
    }
    return;
  }

  if (social.phase === 'ready') {
    if (clock > social.phaseUntil) {
      bothReturnFromMeeting(social.meetingId ?? '');
      return;
    }
    if (partnerReady(social.partnerId)) {
      social.phase = 'face';
      social.phaseUntil = clock + FACE_SEC;
      if (social.partnerId) easeFacePartner(social.id, social.partnerId, dt);
    }
    return;
  }

  if (social.phase === 'face') {
    if (social.partnerId) easeFacePartner(social.id, social.partnerId, dt);
    // Stay in face until both have entered face/gesture (partner may still be ready→face this frame).
    const partner = social.partnerId ? agents.get(social.partnerId) : null;
    if (partner && partner.phase === 'ready') return;
    if (clock >= social.phaseUntil) {
      social.phase = 'gesture';
      social.phaseUntil = clock + GESTURE_SEC;
      const motion = getForumMotion(social.id);
      if (motion) motion.engaged = 'standing';
    }
    return;
  }

  if (social.phase === 'gesture') {
    if (social.partnerId) easeFacePartner(social.id, social.partnerId, dt);
    if (clock >= social.phaseUntil) {
      const meetingId = social.meetingId;
      beginReturn(social);
      if (meetingId) {
        const partner = [...agents.values()].find(a => a.meetingId === meetingId);
        if (partner && (partner.phase === 'gesture' || partner.phase === 'face')) {
          // Partner finishes their own gesture timer; reservation already cleared in beginReturn.
        } else if (reservation?.id === meetingId) {
          reservation = null;
        }
      }
    }
    return;
  }

  if (social.phase === 'return') {
    if (clock > social.phaseUntil) {
      // Timeout: stop ambient travel in place — never teleport.
      stopAmbientTravelOnly(social.id);
      resetAgent(social);
      return;
    }
    if (arrivedOrIdle(social.id) && near(social.id, social.home, 0.55)) {
      // workArrival on the return command restores "At workstation" on path complete.
      resetAgent(social);
      return;
    }
    if (arrivedOrIdle(social.id)) {
      if (social.replans < 1) {
        social.replans++;
        if (!issueAmbientReturn(social.id, social.home)) {
          stopAmbientTravelOnly(social.id);
          resetAgent(social);
        }
      } else {
        stopAmbientTravelOnly(social.id);
        resetAgent(social);
      }
    }
  }
}

function cancelAllAmbientForReducedMotion(input: SocialInput, working: Set<string>) {
  for (const social of agents.values()) {
    if (social.phase !== 'idle' || social.meetingId) cancelAmbient(social.id, true);
    else clearAmbientPose(social.id);
    // Preempt flags still cancel ambient travel if somehow active.
    if (isPreempted(social.id, input, working)) stopAmbientTravelOnly(social.id);
  }
  reservation = null;
}

export function advanceHabitatSocial(input: SocialInput): void {
  if (!ready) prepareHabitatSocial();
  // Hidden or reduced-motion: freeze schedule clock (no burst catchup on resume).
  if (!input.active || input.reducedMotion) {
    const working = workingSet(input);
    if (input.reducedMotion && input.active) cancelAllAmbientForReducedMotion(input, working);
    return;
  }
  const dt = Math.min(Math.max(input.dt, 0), 0.05);
  if (!(dt > 0)) return;
  clock += dt;
  const working = workingSet(input);
  ensurePlanAhead();

  while (planCursor < plan.length && plan[planCursor].at <= clock) {
    const event = plan[planCursor++];
    if (event.kind === 'stroll') {
      if (!isPreempted(event.agent, input, working)) startStroll(event.agent, event.point);
    } else if (!isPreempted(event.agents[0], input, working) && !isPreempted(event.agents[1], input, working)) {
      startMeet(event);
    }
  }

  for (const social of agents.values()) advanceAgent(social, input, working, dt);
}

export function resetHabitatSocial(): void {
  for (const social of agents.values()) {
    stopAmbientTravelOnly(social.id);
    resetAgent(social);
  }
  clock = 0;
  pathCommands = 0;
  candidates = [];
  plan = [];
  planCursor = 0;
  cycleIndex = 0;
  planningMs = 0;
  planIdle = false;
  reservation = null;
  agents.clear();
  ready = false;
}

export function getHabitatSocialClock(): number {
  return clock;
}

export function getHabitatSocialPhase(id: string): SocialPhase {
  if (!ready) return 'idle';
  return agents.get(id)?.phase ?? 'idle';
}

export function getHabitatSocialPathCommandCount(): number {
  return pathCommands;
}

export function getHabitatSocialCandidates(): readonly Point3[] {
  if (!ready) prepareHabitatSocial();
  return candidates;
}

export function getHabitatSocialPlan(): readonly Planned[] {
  if (!ready) prepareHabitatSocial();
  return plan;
}

export function getHabitatMeetingReservation(): MeetingReservation | null {
  return reservation;
}

export function getHabitatSocialPlanningMs(): number {
  return planningMs;
}

export function getHabitatSocialCycleSeconds(): number {
  return CYCLE_SEC;
}

export function getHabitatSocialRetainedPlanLength(): number {
  return Math.max(0, plan.length - planCursor);
}

export function isHabitatSocialPlanIdle(): boolean {
  return planIdle || (ready && candidates.length === 0);
}

/** Test helper: install candidates/plan without rebuild (zero-candidate / blocked plan). */
export function debugInstallAmbientCandidates(points: Point3[]): void {
  ensureAgents();
  candidates = points;
  plan = points.length ? appendCycle(points, 0) : [];
  planCursor = 0;
  cycleIndex = 1;
  planIdle = points.length === 0;
  ready = true;
}

/** Test helper: override return home (unreachable return / stale-route clear). */
export function debugSetAmbientHome(id: string, home: Point3): void {
  ensureAgents();
  const social = agents.get(id);
  if (social) social.home = home;
}

/** Test helper for targeted ownership/teardown cases — not used by the 120s acceptance. */
export function debugForceAmbientStroll(id: string, point: Point3): boolean {
  if (!ready) prepareHabitatSocial();
  const social = agents.get(id);
  if (!social) return false;
  if (social.meetingId) releaseMeeting(social.meetingId, true);
  resetAgent(social);
  startStroll(id, point);
  return social.phase === 'stroll';
}

/** Test helper for reservation/partial-failure cases — not used by the 120s acceptance. */
export function debugForceAmbientMeet(): boolean {
  if (!ready) prepareHabitatSocial();
  if (reservation) return false;
  const meet = plan.find((e): e is PlannedMeet => e.kind === 'meet');
  if (!meet) return false;
  for (const id of meet.agents) {
    const social = agents.get(id);
    if (social) resetAgent(social);
  }
  return startMeet(meet);
}
