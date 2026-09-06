import { describe, expect, it } from 'vitest';
import { STATIONS } from '../../constants';
import { buildStationPedestalLayout, stationIdForInstance } from './HallStructure';

describe('station pedestal batching', () => {
  it('keeps every station transform aligned with its original cardinal anchor', () => {
    const layout = buildStationPedestalLayout();

    expect(layout.ids).toEqual(STATIONS.map((station) => station.id));
    expect(layout.pedestals).toHaveLength(STATIONS.length);
    expect(layout.plinths).toHaveLength(STATIONS.length);
    expect(layout.caps).toHaveLength(STATIONS.length);
    expect(layout.rings).toHaveLength(STATIONS.length);
    expect(layout.interactionTargets).toHaveLength(STATIONS.length);
    STATIONS.forEach((station, index) => {
      expect(layout.pedestals[index].position).toEqual([station.position[0], 0.75, station.position[2]]);
      expect(layout.interactionTargets[index].position).toEqual([station.position[0], 1.4, station.position[2]]);
    });
  });

  it('routes instanced hover/click identity to the original station IDs', () => {
    const ids = STATIONS.map((station) => station.id);

    expect(ids.map((_, index) => stationIdForInstance(ids, index))).toEqual(ids);
    expect(stationIdForInstance(ids, undefined)).toBeNull();
    expect(stationIdForInstance(ids, -1)).toBeNull();
    expect(stationIdForInstance(ids, ids.length)).toBeNull();
  });
});
