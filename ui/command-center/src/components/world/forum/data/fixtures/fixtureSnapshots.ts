/**
 * TEST DATA — run each World fixture through the real snapshot loader.
 *
 * Phases load in order on ONE loader (so phase 1 of the disconnect fixture
 * sees phase 0's last good data), with the clock advanced past every
 * staleness threshold between phases. The returned snapshots are those of the
 * fixture's `displayPhase`.
 */
import { STALE_AFTER_MS } from '../bounds';
import { WORLD_PLACE_IDS, type WorldPlaceId } from '../identity';
import { createWorldSnapshotLoader, type PlaceSnapshot } from '../snapshotLoader';
import { FIXTURE_NOW, createFixtureClient, type WorldFixture } from './worldFixtures';

const PHASE_STEP_MS = Math.max(...Object.values(STALE_AFTER_MS)) + 1_000;

export interface FixtureRun {
  fixture: WorldFixture;
  snapshots: Record<WorldPlaceId, PlaceSnapshot>;
  /** Clock value the display phase was loaded at. */
  now: number;
  /** Requests per place for the display phase. */
  requestCounts: Record<WorldPlaceId, number>;
  calls: string[];
}

export async function runFixture(fixture: WorldFixture): Promise<FixtureRun> {
  let phase = 0;
  let clock = FIXTURE_NOW;
  const calls: string[] = [];
  const client = createFixtureClient(() => fixture.phases[phase], calls);
  const loader = createWorldSnapshotLoader(client, { now: () => clock });
  let snapshots = {} as Record<WorldPlaceId, PlaceSnapshot>;
  for (phase = 0; phase <= fixture.displayPhase; phase += 1) {
    if (phase > 0) {
      clock += PHASE_STEP_MS;
      loader.invalidate(['/']);
    }
    snapshots = {} as Record<WorldPlaceId, PlaceSnapshot>;
    for (const placeId of WORLD_PLACE_IDS) {
      snapshots[placeId] = await loader.loadPlace(placeId, { projectId: fixture.projectId });
    }
  }
  const requestCounts = Object.fromEntries(WORLD_PLACE_IDS.map((p) => [p, snapshots[p].requestCount])) as Record<WorldPlaceId, number>;
  return { fixture, snapshots, now: clock, requestCounts, calls };
}
