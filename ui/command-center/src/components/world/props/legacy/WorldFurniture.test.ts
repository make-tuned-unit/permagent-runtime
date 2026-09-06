import { describe, expect, it } from 'vitest';
import { buildForumLayout, buildWorkbenchLayout } from './WorldFurniture';

describe('active legacy furniture batching layouts', () => {
  it('keeps the workbench parts and screen line counts', () => {
    const layout = buildWorkbenchLayout();
    expect(layout.tops).toHaveLength(2);
    expect(layout.legs).toHaveLength(8);
    expect(layout.drawers).toHaveLength(6);
    expect(layout.handles).toHaveLength(6);
    expect(layout.circuits).toHaveLength(2);
    expect(layout.panels).toHaveLength(2);
    expect(layout.frames).toHaveLength(2);
    expect(layout.lines).toHaveLength(10);
    expect(layout.rackBack).toHaveLength(1);
    expect(layout.rackShelves).toHaveLength(3);
    expect(layout.toolsCyan).toHaveLength(2);
    expect(layout.toolsAmber).toHaveLength(2);
    expect(layout.stoolSeats).toHaveLength(2);
    expect(layout.stoolStems).toHaveLength(2);
    expect(layout.stoolBases).toHaveLength(2);
    expect(layout.tops[0]?.position).toEqual([0, 0.9, 0]);
    expect(layout.tops[1]?.position).toEqual([4, 0.9, -1]);
  });

  it('keeps the three couch placements and low table transforms', () => {
    const layout = buildForumLayout();
    expect(layout.couchSeats).toHaveLength(3);
    expect(layout.couchBacks).toHaveLength(3);
    expect(layout.couchArms).toHaveLength(6);
    expect(layout.couchLegs).toHaveLength(12);
    expect(layout.couchTrims).toHaveLength(3);
    expect(layout.tableTops).toHaveLength(1);
    expect(layout.tablePillars).toHaveLength(1);
    expect(layout.tableBases).toHaveLength(1);
    expect(layout.couchSeats[0]?.position).toEqual([-3, 0.35, 3]);
    expect(layout.couchSeats[0]?.rotation?.[1]).toBeCloseTo(-0.4);
    expect(layout.tableTops[0]?.position).toEqual([0, 0.4, 3]);
  });
});
