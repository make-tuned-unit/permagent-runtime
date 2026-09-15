import { expect, it } from 'vitest';
import { DISTRICTS } from './districts';
import { MESH_GATE, gateSignedDistance } from './meshPortal';

it('puts the mesh district on the gate court, with no tool behind it', () => {
  const mesh = DISTRICTS.find(d => d.id === 'mesh');
  expect(mesh).toBeTruthy();
  // No tab, no panel: the Mesh has no product surface to open, and a tool here
  // would put a fake destination in the UI.
  expect(mesh!.tool).toBeNull();
  expect(mesh!.subtitle).toBe('The gate to the MESH');
  expect(mesh!.accent).toBe(MESH_GATE_ACCENT);
  // The waypoint sits on the gate court itself rather than the old inner
  // threshold at (10, 0, -12).
  const [x, y, z] = mesh!.position;
  expect(Math.abs(gateSignedDistance({ x, y, z }))).toBeLessThan(1);
  expect(Math.hypot(x, z)).toBeCloseTo(MESH_GATE.courtRadius, 0);
});

it('keeps every district id unique', () => {
  expect(new Set(DISTRICTS.map(d => d.id)).size).toBe(DISTRICTS.length);
});

// Kept literal on purpose: the Stargate/Mesh family colour is fixed by the
// palette, and this is the one district allowed to use it as a destination.
const MESH_GATE_ACCENT = '#5599FF';
