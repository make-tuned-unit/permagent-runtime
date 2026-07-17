// Pure signal→interaction mappings for the environment-truth props. Each of
// these functions is the honesty boundary of one prop: REAL daemon signal in →
// exactly what the world shows out. No prop invents activity these don't
// license.

import { describe, expect, it } from 'vitest';
import { extractHenryWork } from './henryWork';
import {
  resolveNudgeDisplay,
  NUDGE_FLARE_MS,
  NUDGE_PRESENT_MS,
} from './watcherNudge';
import {
  reduceArtifact,
  pruneArtifacts,
  artifactVisual,
  ARTIFACT_CAP,
  COMPLETED_TTL_MS,
  FAILED_TTL_MS,
  GLOW_MS,
  type TaskArtifact,
} from './taskArtifacts';
import {
  horologiumTicks,
  prettifyScheduleName,
  HOROLOGIUM_CAP,
  type RunRow,
} from './runsSignals';
import { diffPetitions } from './decisionSignals';

// ── Henry's in-flight tool (#84) ─────────────────────────────────────────────

describe('extractHenryWork', () => {
  it('surfaces a real tool name and task count', () => {
    expect(extractHenryWork({ current_tool: 'web_search', tasks_in_flight: 2 })).toEqual({
      tool: 'web_search',
      tasksInFlight: 2,
    });
  });

  it('normalizes empty / whitespace / null tool to no chip (no stale claim)', () => {
    expect(extractHenryWork({ current_tool: '', tasks_in_flight: 0 }).tool).toBeNull();
    expect(extractHenryWork({ current_tool: '   ', tasks_in_flight: 0 }).tool).toBeNull();
    expect(extractHenryWork({ current_tool: null }).tool).toBeNull();
    expect(extractHenryWork({}).tool).toBeNull();
  });

  it('clamps a negative/fractional task count to a non-negative int', () => {
    expect(extractHenryWork({ tasks_in_flight: -3 }).tasksInFlight).toBe(0);
    expect(extractHenryWork({ tasks_in_flight: 2.9 }).tasksInFlight).toBe(2);
  });
});

// ── Watcher nudge presentation ───────────────────────────────────────────────

describe('resolveNudgeDisplay', () => {
  it('is dark before any nudge (seq 0)', () => {
    expect(resolveNudgeDisplay({ seq: 0, at: 0 }, 1000)).toEqual({
      active: false,
      flare: 0,
      plaqueAlpha: 0,
    });
  });

  it('flares as a bell over the first flare window, then settles', () => {
    const n = { seq: 1, at: 0 };
    expect(resolveNudgeDisplay(n, 0).flare).toBeCloseTo(0, 5); // rising edge
    expect(resolveNudgeDisplay(n, NUDGE_FLARE_MS / 2).flare).toBeCloseTo(1, 5); // peak
    expect(resolveNudgeDisplay(n, NUDGE_FLARE_MS).flare).toBeCloseTo(0, 5); // done flaring
    expect(resolveNudgeDisplay(n, NUDGE_FLARE_MS + 1).flare).toBe(0);
  });

  it('holds the plaque, then fades it out over the final window', () => {
    const n = { seq: 1, at: 0 };
    expect(resolveNudgeDisplay(n, 1000).plaqueAlpha).toBe(1);
    // Halfway through the 10s fade tail.
    const fadeMid = NUDGE_PRESENT_MS - 5000;
    expect(resolveNudgeDisplay(n, fadeMid).plaqueAlpha).toBeCloseTo(0.5, 1);
  });

  it('goes inactive once the presentation window elapses', () => {
    const d = resolveNudgeDisplay({ seq: 1, at: 0 }, NUDGE_PRESENT_MS);
    expect(d.active).toBe(false);
    expect(d.plaqueAlpha).toBe(0);
  });
});

// ── Task artifacts (real completions → bench stones) ─────────────────────────

describe('task artifacts', () => {
  it('appends completions FIFO and caps the bench', () => {
    let list: TaskArtifact[] = [];
    for (let i = 0; i < ARTIFACT_CAP + 3; i++) {
      list = reduceArtifact(list, 'completed', `t${i}`, i);
    }
    expect(list).toHaveLength(ARTIFACT_CAP);
    // Oldest three were shifted off; newest survives.
    expect(list[0].id).toBe('t3');
    expect(list[list.length - 1].id).toBe(`t${ARTIFACT_CAP + 2}`);
  });

  it('a retried task that later completes replaces its failed entry (no dup)', () => {
    let list: TaskArtifact[] = [];
    list = reduceArtifact(list, 'failed', 'task-42', 100);
    list = reduceArtifact(list, 'completed', 'task-42', 200);
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ id: 'task-42', kind: 'completed', at: 200 });
  });

  it('prunes stones past their TTL (completed lingers, failed dies fast)', () => {
    const list: TaskArtifact[] = [
      { id: 'done', kind: 'completed', at: 0 },
      { id: 'fail', kind: 'failed', at: 0 },
    ];
    // Just after the failed TTL: the ember is gone, the completed stone remains.
    const mid = pruneArtifacts(list, FAILED_TTL_MS + 1);
    expect(mid.map((a) => a.id)).toEqual(['done']);
    // Past the completed TTL: bench is clear.
    expect(pruneArtifacts(list, COMPLETED_TTL_MS + 1)).toHaveLength(0);
  });

  it('completed stones cool from amber glow to plain stone over the glow window', () => {
    const a: TaskArtifact = { id: 'x', kind: 'completed', at: 0 };
    expect(artifactVisual(a, 0).glow).toBeCloseTo(1, 5); // fresh amber
    expect(artifactVisual(a, GLOW_MS / 2).glow).toBeCloseTo(0.5, 5);
    expect(artifactVisual(a, GLOW_MS).glow).toBe(0); // cooled to stone
    expect(artifactVisual(a, COMPLETED_TTL_MS).age01).toBe(1);
  });
});

// ── Horologium (real schedule roster → clock ticks) ──────────────────────────

describe('prettifyScheduleName', () => {
  it('strips path + extension and humanizes separators', () => {
    expect(prettifyScheduleName('/a/b/morning-brief.yaml')).toBe('morning brief');
    expect(prettifyScheduleName('weekly_report.yml')).toBe('weekly report');
    expect(prettifyScheduleName('standup')).toBe('standup');
  });
  it('never invents a name', () => {
    expect(prettifyScheduleName('')).toBe('');
  });
});

describe('horologiumTicks', () => {
  const row = (id: string, status: RunRow['status']): RunRow => ({
    kind: 'schedule',
    id,
    name: `${id}.yaml`,
    status,
  });

  it('an empty roster is a bare, still wheel (honest)', () => {
    const m = horologiumTicks([]);
    expect(m.ticks).toHaveLength(0);
    expect(m.anyRunning).toBe(false);
    expect(m.runningName).toBeNull();
    expect(m.overflow).toBe(0);
  });

  it('places one job at top-dead-center and reports it if running', () => {
    const m = horologiumTicks([row('nightly', 'working')]);
    expect(m.ticks).toHaveLength(1);
    expect(m.ticks[0].angle).toBe(0);
    expect(m.anyRunning).toBe(true);
    expect(m.runningName).toBe('nightly');
  });

  it('orders ticks stably by id so they never shuffle between polls', () => {
    const a = horologiumTicks([row('c', 'idle'), row('a', 'idle'), row('b', 'idle')]);
    const b = horologiumTicks([row('b', 'idle'), row('c', 'idle'), row('a', 'idle')]);
    expect(a.ticks.map((t) => t.id)).toEqual(['a', 'b', 'c']);
    expect(b.ticks.map((t) => t.id)).toEqual(a.ticks.map((t) => t.id));
  });

  it('carries the real status onto each tick and overflows honestly past the cap', () => {
    const rows = Array.from({ length: HOROLOGIUM_CAP + 4 }, (_, i) =>
      row(`job${String(i).padStart(2, '0')}`, i === 0 ? 'error' : 'idle'),
    );
    const m = horologiumTicks(rows);
    expect(m.ticks).toHaveLength(HOROLOGIUM_CAP);
    expect(m.overflow).toBe(4);
    expect(m.ticks[0].status).toBe('error');
  });

  it('reports the FIRST running job as the running name', () => {
    const m = horologiumTicks([row('a', 'idle'), row('b', 'working'), row('c', 'working')]);
    expect(m.runningName).toBe('b');
  });
});

// ── Petition diff (real inbox delta → drop-in / ascend) ──────────────────────

describe('diffPetitions', () => {
  it('detects arrivals and resolutions', () => {
    const d = diffPetitions(['a', 'b'], ['b', 'c']);
    expect(d.arrived).toEqual(['c']);
    expect(d.resolved).toEqual(['a']);
  });
  it('is empty when the open set is unchanged', () => {
    expect(diffPetitions(['x', 'y'], ['y', 'x'])).toEqual({ arrived: [], resolved: [] });
  });
  it('treats first load as all-arrived, none-resolved', () => {
    expect(diffPetitions([], ['a', 'b'])).toEqual({ arrived: ['a', 'b'], resolved: [] });
  });
});
