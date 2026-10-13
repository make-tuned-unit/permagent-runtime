/**
 * L2 exit stall (native r34, Web Inspector timeline): the first click out of a
 * freshly entered World waited 0.6-1.6 s behind one React task. 77% of that
 * task's JS samples were HeroGardenAgents' mount effect proving ambient stroll
 * routes (prepareHabitatSocial -> buildAmbientCandidates/appendCycle -> A*).
 * These tests pin the invariant that is checkable here: a rendered World never
 * proves routes inside the mounting task or a frame; the work is split into
 * bounded background slices and still produces the same plan.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { findHabitatPath, findHabitatPathSteps } from './habitatMotion';
import {
  advanceHabitatSocial,
  getHabitatSocialCandidates,
  getHabitatSocialClock,
  getHabitatSocialPlan,
  isHabitatSocialPlanning,
  prepareHabitatSocial,
  resetHabitatSocial,
  scheduleHabitatSocialPreparation,
} from './habitatSocial';
import { stationFor, type Point3 } from './habitatLayout';

const frame = { dt: 0.05, active: true, reducedMotion: false, conversation: false, liveWorking: [] as string[], following: null, directing: null };

function finishBackgroundPlanning(): number {
  let tasks = 0;
  while (isHabitatSocialPlanning()) {
    vi.advanceTimersToNextTimer();
    tasks++;
    if (tasks > 100_000) throw new Error('background planning never finished');
  }
  return tasks;
}

describe('habitat social planning stays off the input path', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    resetHabitatSocial();
  });
  afterEach(() => {
    resetHabitatSocial();
    vi.useRealTimers();
  });

  it('scheduling returns before any route is proven, and frames do not prove routes while it runs', () => {
    scheduleHabitatSocialPreparation();
    expect(isHabitatSocialPlanning()).toBe(true);
    // A frame during planning must not fall back to the synchronous planner.
    advanceHabitatSocial(frame);
    expect(isHabitatSocialPlanning()).toBe(true);
    expect(getHabitatSocialClock()).toBe(0);
    const tasks = finishBackgroundPlanning();
    // Split across several tasks rather than one monolithic block.
    expect(tasks).toBeGreaterThanOrEqual(3);
    advanceHabitatSocial(frame);
    expect(getHabitatSocialClock()).toBeGreaterThan(0);
  });

  it('background slices produce exactly the plan the synchronous planner proves', () => {
    prepareHabitatSocial();
    const syncCandidates = getHabitatSocialCandidates().map(p => [...p]);
    const syncPlan = JSON.parse(JSON.stringify(getHabitatSocialPlan()));
    resetHabitatSocial();
    scheduleHabitatSocialPreparation();
    finishBackgroundPlanning();
    expect(getHabitatSocialCandidates().map(p => [...p])).toEqual(syncCandidates);
    expect(JSON.parse(JSON.stringify(getHabitatSocialPlan()))).toEqual(syncPlan);
  });

  it('leaving the World cancels queued planning', () => {
    scheduleHabitatSocialPreparation();
    resetHabitatSocial();
    expect(isHabitatSocialPlanning()).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('the stepped A* returns the synchronous route and yields between node expansions', () => {
    const from = stationFor('picker').position;
    const to: Point3 = stationFor('council').position;
    const steps = findHabitatPathSteps(from, to);
    let yields = 0;
    let result: Point3[] | null = null;
    for (;;) {
      const next = steps.next();
      if (next.done) { result = next.value; break; }
      yields++;
    }
    expect(result).toEqual(findHabitatPath(from, to));
    expect(yields).toBeGreaterThan(0);
  });

  it('the World mount schedules planning instead of running it synchronously', () => {
    const source = readFileSync(fileURLToPath(new URL('./HeroGardenKit.tsx', import.meta.url)), 'utf8');
    expect(source).toContain('scheduleHabitatSocialPreparation()');
    expect(source).not.toMatch(/\bprepareHabitatSocial\s*\(/);
  });
});
