/**
 * The Automate Hall's steles, pinned at the point where they make a claim.
 *
 * Before D19 the gallery lit a deterministic ~20% of its cells amber and
 * breathed them on a 4-second clock no matter what the scheduler was doing.
 * The rule now: amber is reserved for a run the daemon reports as in flight,
 * and the number of steles is the number of real jobs.
 */

import { describe, expect, it } from 'vitest';
import { buildSteleLayout, plateText } from './AutomateSteles';
import type { HallJob } from '../areas/automate/scheduleActivity';

const job = (over: Partial<HallJob> = {}): HallJob => ({
  id: 'a',
  label: 'Workspace snapshot',
  cadence: '0 0 8 * * 1-5',
  outcome: 'ok',
  running: false,
  lastRun: null,
  failureStreak: 0,
  ...over,
});

describe('AutomateSteles layout', () => {
  it('renders nothing at all when the scheduler is unknown or empty', () => {
    const l = buildSteleLayout([]);
    expect(l.steles).toHaveLength(0);
    expect(l.running).toHaveLength(0);
    // Not even the furniture: an empty gallery is the honest reading.
    expect(l.marbleWork).toHaveLength(0);
  });

  it('puts up exactly one stele per real job', () => {
    expect(buildSteleLayout([job({ id: 'a' }), job({ id: 'b' })]).steles).toHaveLength(2);
  });

  it('lights no amber for a job that is not running — the whole of D19', () => {
    const l = buildSteleLayout([job({ outcome: 'ok' }), job({ id: 'b', outcome: 'failed' })]);
    expect(l.running).toHaveLength(0);
  });

  it('lights amber only for the job that is actually in flight', () => {
    const l = buildSteleLayout([job({ id: 'a' }), job({ id: 'b', running: true })]);
    expect(l.running.length).toBeGreaterThan(0);
    // Every pulsing cell belongs to the running stele, never a neighbour.
    const bX = l.steles[1].position[0];
    expect(l.running.every(c => Math.abs(c.position[0] - bX) < 0.6)).toBe(true);
  });

  it('colours a failed or missed job as failed, never as healthy', () => {
    expect(buildSteleLayout([job({ outcome: 'failed' })]).healthy).toHaveLength(0);
    expect(buildSteleLayout([job({ outcome: 'missed' })]).failed.length).toBeGreaterThan(0);
  });

  it('draws a never-run or switched-off job as dormant, not as working', () => {
    expect(buildSteleLayout([job({ outcome: 'off' })]).dormant.length).toBeGreaterThan(0);
    expect(buildSteleLayout([job({ outcome: 'never' })]).healthy).toHaveLength(0);
  });

  it('names the automation on the plate, and says so when it is running', () => {
    expect(plateText(job())).toBe('Workspace snapshot');
    expect(plateText(job({ running: true }))).toContain('RUNNING');
    expect(plateText(job({ label: 'a'.repeat(40) })).length).toBeLessThanOrEqual(20);
  });
});
