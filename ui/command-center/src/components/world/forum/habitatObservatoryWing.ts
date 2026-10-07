/**
 * Connected observatory garden wing (r27).
 *
 * Footprint (not a stretched main pad):
 * - Wide pedestrian bridge off the upper-terrace north edge (y = 3.4)
 * - Separate rounded garden terrace north of that bridge
 * - Central instrument plinth for a procedural telescope
 *
 * Walkability, paving, collision rails and Places share these constants.
 */
export type Point3 = [number, number, number];

export const OBSERVATORY_WING_HEIGHT = 3.4;

/** Minimum overlap between bridge, pad and main terrace for continuous walking. */
export const OBSERVATORY_SEAM_OVERLAP = 2.0;

/** Bridge deck linking main upper terrace (z≈-24) to the wing pad. */
export const OBSERVATORY_BRIDGE = {
  x0: 1,
  x1: 7,
  z0: -35,
  z1: -26,
} as const;

/** Rounded rectangular pad (half-extents before corner radius). */
export const OBSERVATORY_PAD = {
  cx: 4,
  cz: -44,
  hx: 14.0,
  hz: 11.0,
  corner: 4.0,
} as const;

export const OBSERVATORY_PLINTH = {
  x: OBSERVATORY_PAD.cx,
  z: OBSERVATORY_PAD.cz + .8,
  radius: 2.15,
  height: 0.32,
} as const;

export const OBSERVATORY_GARDEN_PLACE = {
  id: 'observatory_garden',
  name: 'Observatory garden',
  subtitle: 'Sky court & instrument plinth',
  description: 'A connected garden wing beyond the upper terrace. Cross the north bridge to the instrument court.',
  position: [OBSERVATORY_PAD.cx + 2.8, OBSERVATORY_WING_HEIGHT, OBSERVATORY_PAD.cz + 2.5] as Point3,
  agent: 'forecaster' as const,
};

/** North-rail opening matches the bridge width so Walk never hits a phantom barrier. */
export function onObservatoryBridgeOpening(x: number, z: number, margin = 0): boolean {
  return (
    x >= OBSERVATORY_BRIDGE.x0 + margin
    && x <= OBSERVATORY_BRIDGE.x1 - margin
    && z >= OBSERVATORY_BRIDGE.z0 + margin
    && z <= OBSERVATORY_BRIDGE.z1 - margin
  );
}

export function onObservatoryBridge(x: number, z: number, margin = 0): boolean {
  return onObservatoryBridgeOpening(x, z, margin);
}

export function onObservatoryPad(x: number, z: number, margin = 0): boolean {
  const { cx, cz, hx, hz, corner } = OBSERVATORY_PAD;
  const r = Math.max(0.05, corner - margin);
  const halfX = hx - margin;
  const halfZ = hz - margin;
  if (Math.abs(x - cx) > halfX || Math.abs(z - cz) > halfZ) return false;
  const dx = Math.max(Math.abs(x - cx) - (halfX - r), 0);
  const dz = Math.max(Math.abs(z - cz) - (halfZ - r), 0);
  return Math.hypot(dx, dz) <= r + 1e-6;
}

export function onObservatoryWing(x: number, z: number, margin = 0): boolean {
  return onObservatoryBridge(x, z, margin) || onObservatoryPad(x, z, margin);
}

export function observatoryWingHeight(_x: number, _z: number): number {
  return OBSERVATORY_WING_HEIGHT;
}

export function isOutsideObservatoryPlinth(x: number, z: number, margin = 0.55): boolean {
  const { x: px, z: pz, radius } = OBSERVATORY_PLINTH;
  return Math.hypot(x - px, z - pz) > radius + margin;
}

/** Prior main terrace rounded 32×44 m footprint (~1394 m²), not the inner 24×36 slab. */
export function mainTerraceAreaEstimate(): number {
  const width = 32;
  const depth = 44;
  const corner = 4;
  return width * depth - 4 * corner * corner * (1 - Math.PI / 4);
}

/** Numerical area of bridge + pad union via midpoint grid integration (m²). */
export function observatoryWingUnionArea(step = 0.25): number {
  const { x0, x1, z0, z1 } = OBSERVATORY_BRIDGE;
  const { cx, cz, hx, hz } = OBSERVATORY_PAD;
  const minX = Math.min(x0, cx - hx);
  const maxX = Math.max(x1, cx + hx);
  const minZ = Math.min(z0, cz - hz);
  const maxZ = Math.max(z1, cz + hz);
  let area = 0;
  for (let x = minX + step/2; x < maxX; x += step) {
    for (let z = minZ + step/2; z < maxZ; z += step) {
      if (onObservatoryWing(x, z, 0)) area += step * step;
    }
  }
  return area;
}

/** Additional floor footprint excluding overlap already counted on the main terrace. */
export function observatoryWingAdditionalArea(onMainTerrace: (x: number, z: number) => boolean, step = 0.25): number {
  const { x0, x1, z0, z1 } = OBSERVATORY_BRIDGE;
  const { cx, cz, hx, hz } = OBSERVATORY_PAD;
  const minX = Math.min(x0, cx - hx);
  const maxX = Math.max(x1, cx + hx);
  const minZ = Math.min(z0, cz - hz);
  const maxZ = Math.max(z1, cz + hz);
  let area = 0;
  for (let x = minX + step/2; x < maxX; x += step) {
    for (let z = minZ + step/2; z < maxZ; z += step) {
      if (onObservatoryWing(x, z, 0) && !onMainTerrace(x, z)) area += step * step;
    }
  }
  return area;
}

/** Bridge/pad seam samples must stay walkable with production margins. */
export function observatorySeamSamples(): [number, number][] {
  const midX = (OBSERVATORY_BRIDGE.x0 + OBSERVATORY_BRIDGE.x1) / 2;
  const samples: [number, number][] = [];
  for (let z = OBSERVATORY_BRIDGE.z0 + 0.5; z <= OBSERVATORY_BRIDGE.z1 - 0.5; z += 0.5) {
    samples.push([midX, z]);
  }
  const seamZ = OBSERVATORY_BRIDGE.z0 + OBSERVATORY_SEAM_OVERLAP * 0.5;
  for (let x = OBSERVATORY_BRIDGE.x0 + 0.5; x <= OBSERVATORY_BRIDGE.x1 - 0.5; x += 0.75) {
    samples.push([x, seamZ]);
  }
  return samples;
}

/** Integer metre cells fully inside the bridge rectangle (no whole-metre spill). */
export function observatoryBridgeMetreCells(): { x: number; z: number }[] {
  const cells: { x: number; z: number }[] = [];
  const xStart = Math.ceil(OBSERVATORY_BRIDGE.x0);
  const xEnd = Math.floor(OBSERVATORY_BRIDGE.x1);
  const zStart = Math.ceil(OBSERVATORY_BRIDGE.z0);
  const zEnd = Math.floor(OBSERVATORY_BRIDGE.z1);
  for (let z = zStart; z < zEnd; z++) {
    for (let x = xStart; x < xEnd; x++) {
      const cx = x + 0.5;
      const cz = z + 0.5;
      if (onObservatoryBridge(cx, cz, 0)) cells.push({ x, z });
    }
  }
  return cells;
}


/** Rounded pad outline; corner centres stay fixed when the outline is inset. */
export function observatoryPadOutline(inset=0): [number,number][] {
  const {cx,cz,hx,hz,corner}=OBSERVATORY_PAD,r=Math.max(.05,corner-inset);
  const corners=[[cx+hx-corner,cz+hz-corner],[cx-hx+corner,cz+hz-corner],[cx-hx+corner,cz-hz+corner],[cx+hx-corner,cz-hz+corner]];
  return corners.flatMap(([x,z],j)=>Array.from({length:13},(_,i)=>{
    const a=(j+i/12)*Math.PI/2;return [x+Math.cos(a)*r,z+Math.sin(a)*r] as [number,number];
  }));
}
export type ObservatoryGuardSegment = {a:[number,number];b:[number,number]};
/** Shared visible/collision guards, including a genuine opening to the bridge. */
export function observatoryGuardSegments(): ObservatoryGuardSegment[] {
  const outline=observatoryPadOutline(.08),result:ObservatoryGuardSegment[]=[];
  const south=OBSERVATORY_PAD.cz+OBSERVATORY_PAD.hz-.08;
  const {x0,x1}=OBSERVATORY_BRIDGE;
  for(let i=0;i<outline.length;i++){
    const a=outline[i],b=outline[(i+1)%outline.length];
    if(Math.hypot(a[0]-b[0],a[1]-b[1])<1e-6)continue;
    if(Math.abs(a[1]-south)<1e-5&&Math.abs(b[1]-south)<1e-5&&a[0]>x1&&b[0]<x0){
      result.push({a,b:[x1,south]},{a:[x0,south],b});
    }else result.push({a,b});
  }
  for(const x of [x0+.08,x1-.08])result.push({a:[x,-27.6],b:[x,south]});
  return result;
}

/** Seating stays outside the central arrival and instrument paths. */
export const OBSERVATORY_BENCHES = [{x:OBSERVATORY_PAD.cx-6,z:OBSERVATORY_PAD.cz-4.8},{x:OBSERVATORY_PAD.cx+6,z:OBSERVATORY_PAD.cz-4.8}] as const;
