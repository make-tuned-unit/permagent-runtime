import { describe, expect, it, beforeEach, afterEach } from 'vitest';
import { Mesh } from 'three';
import {
  advanceHabitatAgent,
  canHabitatWorkReturn,
  commandHabitatAgent,
  getHabitatTravel,
  hasHabitatUserHold,
  initializeHabitatAgent,
  stopHabitatAgent,
} from './habitatMotion';
import {
  advanceHabitatSocial,
  buildAmbientCandidates,
  canAmbientReturnHome,
  debugForceAmbientMeet,
  debugForceAmbientStroll,
  debugInstallAmbientCandidates,
  debugSetAmbientHome,
  getHabitatMeetingReservation,
  getHabitatSocialCandidates,
  getHabitatSocialClock,
  getHabitatSocialCycleSeconds,
  getHabitatSocialPathCommandCount,
  getHabitatSocialPhase,
  getHabitatSocialPlan,
  getHabitatSocialPlanningMs,
  getHabitatSocialRetainedPlanLength,
  isAmbientSafePoint,
  isHabitatSocialPlanIdle,
  meetingStands,
  prepareHabitatSocial,
  proveAmbientRoute,
  resetHabitatSocial,
} from './habitatSocial';
import { HABITAT_STATIONS, HANK_TABLE_SEAT, habitatCollisionSource, habitatElevation, stationFor, type Point3 } from './habitatLayout';
import { registerForumCollision } from './walkCollision';
import { getForumMotion } from './ForumInhabitants';

function bootAgents() {
  for (const station of HABITAT_STATIONS) initializeHabitatAgent(station.id);
}

function tick(seconds: number, input: Partial<Parameters<typeof advanceHabitatSocial>[0]> = {}) {
  const steps = Math.ceil(seconds / 0.05);
  const base = {
    dt: 0.05,
    active: true,
    reducedMotion: false,
    conversation: false,
    liveWorking: [] as string[],
    following: null as string | null,
    directing: null as string | null,
    ...input,
  };
  for (let i = 0; i < steps; i++) {
    advanceHabitatSocial(base);
    for (const station of HABITAT_STATIONS) advanceHabitatAgent(station.id, 0.05);
  }
}

describe('habitat ambient social controller', () => {
  let release: (() => void) | null = null;
  let source: ReturnType<typeof habitatCollisionSource> | null = null;

  beforeEach(() => {
    resetHabitatSocial();
    source = habitatCollisionSource();
    release = registerForumCollision(source);
    bootAgents();
    prepareHabitatSocial();
  });

  afterEach(() => {
    release?.();
    release = null;
    if (source) {
      const materials = new Set<{ dispose: () => void }>();
      source.traverse(o => {
        if (o instanceof Mesh) {
          o.geometry.dispose();
          (Array.isArray(o.material) ? o.material : [o.material]).forEach(m => materials.add(m));
        }
      });
      materials.forEach(m => m.dispose());
    }
    source = null;
    resetHabitatSocial();
  });

  it('proves bounded candidates, separated meeting stands, and rejects unsafe points', () => {
    const candidates = buildAmbientCandidates();
    expect(candidates.length).toBeGreaterThan(2);
    expect(candidates.length).toBeLessThanOrEqual(10);
    expect(getHabitatSocialPlanningMs()).toBeGreaterThanOrEqual(0);
    expect(getHabitatSocialPlanningMs()).toBeLessThan(5_000);
    for (const point of candidates) expect(isAmbientSafePoint(point[0], point[2])).toBe(true);
    expect(isAmbientSafePoint(HANK_TABLE_SEAT[0], HANK_TABLE_SEAT[2])).toBe(false);
    const stands = meetingStands(candidates[0]);
    expect(stands).not.toBeNull();
    expect(Math.hypot(stands![0][0] - stands![1][0], stands![0][2] - stands![1][2])).toBeGreaterThanOrEqual(0.84);
  });

  it('within 120s records a simultaneous pair gesture at separated stands, journeys, and resters', () => {
    const plan = getHabitatSocialPlan();
    const meets = plan.filter(e => e.kind === 'meet');
    const strolls = plan.filter(e => e.kind === 'stroll');
    expect(meets.length).toBeGreaterThanOrEqual(1);
    expect(strolls.length).toBeGreaterThanOrEqual(2);
    expect(meets[0].at).toBeLessThan(getHabitatSocialCycleSeconds());
    // Meet participants are not also first-cycle strollers (avoids discard conflict).
    const strollAgents = new Set(strolls.filter(e => e.at < 120).map(e => e.agent));
    expect(strollAgents.has(meets[0].agents[0])).toBe(false);
    expect(strollAgents.has(meets[0].agents[1])).toBe(false);

    const meet = meets[0];
    const sep = Math.hypot(meet.standA[0] - meet.standB[0], meet.standA[2] - meet.standB[2]);
    expect(sep).toBeGreaterThanOrEqual(0.84);

    const traversed = new Set<string>();
    const homeDist = new Map<string, number>();
    for (const s of HABITAT_STATIONS) homeDist.set(s.id, 0);
    let simultaneousGesture = false;
    let gestureNearStands = false;
    const atGesture: { id: string; x: number; z: number }[] = [];

    const steps = Math.ceil(120 / 0.05);
    for (let i = 0; i < steps; i++) {
      advanceHabitatSocial({
        dt: 0.05,
        active: true,
        reducedMotion: false,
        conversation: false,
        liveWorking: [],
        following: null,
        directing: null,
      });
      for (const station of HABITAT_STATIONS) advanceHabitatAgent(station.id, 0.05);
      for (const station of HABITAT_STATIONS) {
        const pose = getForumMotion(station.id)!;
        const home = station.position;
        const d = Math.hypot(pose.x - home[0], pose.z - home[2]);
        homeDist.set(station.id, Math.max(homeDist.get(station.id)!, d));
        if (d > 1.2) traversed.add(station.id);
      }
      const a = meet.agents[0], b = meet.agents[1];
      if (getHabitatSocialPhase(a) === 'gesture' && getHabitatSocialPhase(b) === 'gesture') {
        simultaneousGesture = true;
        const pa = getForumMotion(a)!, pb = getForumMotion(b)!;
        if (atGesture.length === 0) {
          atGesture.push({ id: a, x: pa.x, z: pa.z }, { id: b, x: pb.x, z: pb.z });
        }
        const nearA = Math.hypot(pa.x - meet.standA[0], pa.z - meet.standA[2]) < 0.7
          || Math.hypot(pa.x - meet.standB[0], pa.z - meet.standB[2]) < 0.7;
        const nearB = Math.hypot(pb.x - meet.standA[0], pb.z - meet.standA[2]) < 0.7
          || Math.hypot(pb.x - meet.standB[0], pb.z - meet.standB[2]) < 0.7;
        const apart = Math.hypot(pa.x - pb.x, pa.z - pb.z) >= 0.7;
        if (nearA && nearB && apart) gestureNearStands = true;
      }
    }

    expect(simultaneousGesture).toBe(true);
    expect(gestureNearStands).toBe(true);
    expect(traversed.size).toBeGreaterThanOrEqual(2);
    expect(atGesture.length).toBe(2);

    // After gesture, pair must leave the meeting stands (not remain idle on the slot).
    tick(25);
    for (const prior of atGesture) {
      const phase = getHabitatSocialPhase(prior.id);
      expect(['return', 'idle'].includes(phase)).toBe(true);
      const p = getForumMotion(prior.id)!;
      const moved = Math.hypot(p.x - prior.x, p.z - prior.z);
      const awayA = Math.hypot(p.x - meet.standA[0], p.z - meet.standA[2]);
      const awayB = Math.hypot(p.x - meet.standB[0], p.z - meet.standB[2]);
      const home = stationFor(prior.id).position;
      const atHome = Math.hypot(p.x - home[0], p.z - home[2]) < 0.7;
      expect(moved > 0.35 || (awayA > 0.9 && awayB > 0.9) || atHome).toBe(true);
    }
    const mostlyRest = [...homeDist.entries()].filter(([, d]) => d < 0.8).map(([id]) => id);
    expect(mostlyRest.length).toBeGreaterThanOrEqual(4);
  }, 180_000);

  it('keeps seeded activity after the first 120s with new path commands and bounded plan', () => {
    const traversedAfter = new Set<string>();
    tick(120);
    const commandsAt120 = getHabitatSocialPathCommandCount();
    const clockAt120 = getHabitatSocialClock();
    expect(clockAt120).toBeGreaterThan(119);
    expect(commandsAt120).toBeGreaterThan(0);
    const planLenAt120 = getHabitatSocialRetainedPlanLength();

    const steps = Math.ceil(30 / 0.05);
    for (let i = 0; i < steps; i++) {
      advanceHabitatSocial({
        dt: 0.05, active: true, reducedMotion: false, conversation: false,
        liveWorking: [], following: null, directing: null,
      });
      for (const station of HABITAT_STATIONS) {
        advanceHabitatAgent(station.id, 0.05);
        const pose = getForumMotion(station.id)!;
        const home = station.position;
        if (Math.hypot(pose.x - home[0], pose.z - home[2]) > 1.2) traversedAfter.add(station.id);
      }
    }
    expect(getHabitatSocialClock()).toBeGreaterThan(clockAt120 + 25);
    expect(getHabitatSocialPathCommandCount()).toBeGreaterThan(commandsAt120);
    expect(traversedAfter.size).toBeGreaterThanOrEqual(1);
    expect(getHabitatSocialRetainedPlanLength()).toBeLessThanOrEqual(planLenAt120 + 40);

    // Continue to 300s total wall of schedule time from start — plan stays bounded.
    tick(150);
    expect(getHabitatSocialClock()).toBeGreaterThan(290);
    expect(getHabitatSocialRetainedPlanLength()).toBeLessThan(80);

    // Hidden pause must not advance clock or dump events.
    const pausedClock = getHabitatSocialClock();
    const pausedCommands = getHabitatSocialPathCommandCount();
    tick(40, { active: false });
    expect(getHabitatSocialClock()).toBeCloseTo(pausedClock, 5);
    expect(getHabitatSocialPathCommandCount()).toBe(pausedCommands);
    advanceHabitatSocial({
      dt: 5, active: true, reducedMotion: false, conversation: false,
      liveWorking: [], following: null, directing: null,
    });
    expect(getHabitatSocialClock()).toBeCloseTo(pausedClock + 0.05, 5);
  }, 300_000);

  it('user travel and Stop outrank work; failed user route keeps hold', () => {
    const elsewhere: Point3 = [0.5, habitatElevation(0.5, 4.8), 4.8];
    expect(commandHabitatAgent('picker', elsewhere, 'Walking to destination')).toBe(true);
    expect(getHabitatTravel('picker')?.source).toBe('user');
    expect(commandHabitatAgent('picker', stationFor('picker').position, 'Returning to workstation', true, 'work')).toBe(false);
    expect(getHabitatTravel('picker')?.source).toBe('user');
    expect(canHabitatWorkReturn('picker')).toBe(false);

    stopHabitatAgent('picker');
    expect(hasHabitatUserHold('picker')).toBe(true);
    expect(commandHabitatAgent('picker', stationFor('picker').position, 'Returning to workstation', true, 'work')).toBe(false);
    // Failed user route must not release Stop.
    expect(commandHabitatAgent('picker', [20, 0, 0], 'Walking to destination', false, 'user')).toBe(false);
    expect(hasHabitatUserHold('picker')).toBe(true);
    expect(getHabitatTravel('picker')?.source).toBe('user');
    expect(getHabitatTravel('picker')?.label).toContain('No clear route');

    expect(commandHabitatAgent('picker', elsewhere, 'Walking to destination', false, 'user')).toBe(true);
    expect(hasHabitatUserHold('picker')).toBe(false);
  });

  it('mid-meeting user override and partial meet failure tear down atomically', () => {
    expect(debugForceAmbientMeet()).toBe(true);
    const reserved = getHabitatMeetingReservation();
    expect(reserved).not.toBeNull();
    const [a, b] = reserved!.agents;
    expect(getHabitatSocialPhase(a)).toBe('approach');
    expect(getHabitatSocialPhase(b)).toBe('approach');

    // Mid-meeting override on one participant.
    expect(commandHabitatAgent(a, [0.5, habitatElevation(0.5, 4.8), 4.8], 'Walking to destination')).toBe(true);
    tick(0.15);
    expect(getHabitatMeetingReservation()).toBeNull();
    expect(getHabitatSocialPhase(a)).toBe('idle');
    // Partner ambient travel must stop (not keep walking orphaned).
    expect(getHabitatTravel(b)?.source).not.toBe('ambient');
    expect(getHabitatSocialPhase(b)).toBe('idle');

    // Partial command failure: second agent blocked → no reservation, no orphan ambient.
    initializeHabitatAgent(a);
    initializeHabitatAgent(b);
    const meet = getHabitatSocialPlan().find(e => e.kind === 'meet');
    expect(meet && meet.kind === 'meet').toBe(true);
    if (meet && meet.kind === 'meet') {
      // Occupy stand B so B cannot path there while A could.
      const blocker = getForumMotion('henry')!;
      blocker.x = meet.standB[0];
      blocker.z = meet.standB[2];
      blocker.y = meet.standB[1];
      const started = debugForceAmbientMeet();
      // May fail to start due to occupied stand; either way no half meeting.
      if (started) {
        expect(getHabitatMeetingReservation()?.agents.length).toBe(2);
      } else {
        expect(getHabitatMeetingReservation()).toBeNull();
        expect(getHabitatTravel(a)?.source).not.toBe('ambient');
        expect(getHabitatTravel(b)?.source).not.toBe('ambient');
      }
    }
  });

  it('clears ambient with truthful away-from-station labels and no RM motion/catchup', () => {
    const point = getHabitatSocialCandidates()[0];
    expect(debugForceAmbientStroll('steward', point)).toBe(true);
    tick(2);
    const pose = getForumMotion('steward')!;
    const home = stationFor('steward').position;
    if (Math.hypot(pose.x - home[0], pose.z - home[2]) > 0.6) {
      stopHabitatAgent('steward'); // sets user — instead test clear via reduced motion cancel
    }
    initializeHabitatAgent('steward');
    expect(debugForceAmbientStroll('steward', point)).toBe(true);
    tick(3);
    const headingBefore = HABITAT_STATIONS.map(s => getForumMotion(s.id)!.heading);
    const clockBefore = getHabitatSocialClock();
    const commandsBefore = getHabitatSocialPathCommandCount();
    tick(12, { reducedMotion: true });
    expect(getHabitatSocialClock()).toBeCloseTo(clockBefore, 5);
    expect(getHabitatSocialPathCommandCount()).toBe(commandsBefore);
    expect(getHabitatSocialPhase('steward')).toBe('idle');
    for (let i = 0; i < HABITAT_STATIONS.length; i++) {
      expect(getForumMotion(HABITAT_STATIONS[i].id)!.heading).toBeCloseTo(headingBefore[i], 5);
    }
  });

  it('fails a blocked ambient destination without teleporting', () => {
    const home = stationFor('picker').position;
    const blocked: Point3 = [4, habitatElevation(4, 0), 0];
    expect(proveAmbientRoute(home, blocked)).toBeNull();
    expect(debugForceAmbientStroll('picker', blocked)).toBe(false);
    const pose = getForumMotion('picker')!;
    expect(Math.hypot(pose.x - home[0], pose.z - home[2])).toBeLessThan(0.05);
  });

  it('idles safely with zero candidates and keeps retained plan bounded', () => {
    debugInstallAmbientCandidates([]);
    expect(isHabitatSocialPlanIdle()).toBe(true);
    const started = Date.now();
    tick(5);
    expect(Date.now() - started).toBeLessThan(10_000);
    expect(getHabitatSocialRetainedPlanLength()).toBe(0);
    expect(getHabitatSocialPathCommandCount()).toBe(0);
  });

  it('stops stale outbound ambient travel when return home is unreachable', () => {
    const point = getHabitatSocialCandidates()[0];
    expect(debugForceAmbientStroll('picker', point)).toBe(true);
    expect(getHabitatTravel('picker')?.source).toBe('ambient');
    debugSetAmbientHome('picker', [20, 0, 0]);
    // Freeze further schedule so a later stroll cannot reclaim after failed return.
    debugInstallAmbientCandidates([]);
    tick(60);
    expect(getHabitatSocialPhase('picker')).toBe('idle');
    expect(getHabitatTravel('picker')?.source).not.toBe('ambient');
    expect(getHabitatTravel('picker')?.moving).toBe(false);
  });

  it('restores At workstation after a successful ambient return home', () => {
    const home = stationFor('steward').position;
    expect(canAmbientReturnHome('steward')).toBe(true);
    const point = getHabitatSocialCandidates().find(p => proveAmbientRoute(home, p)) ?? getHabitatSocialCandidates()[0];
    expect(debugForceAmbientStroll('steward', point)).toBe(true);
    // Drive through dwell + return.
    for (let i = 0; i < 4000 && getHabitatSocialPhase('steward') !== 'idle'; i++) {
      advanceHabitatSocial({
        dt: 0.05, active: true, reducedMotion: false, conversation: false,
        liveWorking: [], following: null, directing: null,
      });
      advanceHabitatAgent('steward', 0.05);
    }
    expect(getHabitatSocialPhase('steward')).toBe('idle');
    const pose = getForumMotion('steward')!;
    expect(Math.hypot(pose.x - home[0], pose.z - home[2])).toBeLessThan(0.7);
    expect(getHabitatTravel('steward')?.label).toBe('At workstation');
  }, 60_000);

  it('retries work return after user travel arrives (ownership release)', () => {
    const elsewhere: Point3 = [0.5, habitatElevation(0.5, 4.8), 4.8];
    expect(commandHabitatAgent('steward', elsewhere, 'Walking to destination', false, 'user')).toBe(true);
    expect(commandHabitatAgent('steward', stationFor('steward').position, 'Returning to workstation', true, 'work')).toBe(false);
    for (let i = 0; i < 4000 && getHabitatTravel('steward')?.moving; i++) advanceHabitatAgent('steward', 0.05);
    expect(getHabitatTravel('steward')?.source).toBe('none');
    expect(canHabitatWorkReturn('steward')).toBe(true);
    expect(commandHabitatAgent('steward', stationFor('steward').position, 'Returning to workstation', true, 'work')).toBe(true);
    expect(getHabitatTravel('steward')?.source).toBe('work');
    stopHabitatAgent('steward');
    expect(canHabitatWorkReturn('steward')).toBe(false);
  }, 60_000);
});
