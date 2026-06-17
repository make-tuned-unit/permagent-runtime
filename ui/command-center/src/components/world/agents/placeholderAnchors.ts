// TEMPORARY — REMOVE AFTER W2'S ANCHOR-BEARING PROPS LAND.
//
// W3 consumes the shared/anchors.ts registry (W2 registers, W3 claims). Until W2's
// prop library lands in feature/world-v2, this registers a few seat anchors at the
// existing station pedestals so working agents have somewhere real to sit.
// Convention assumed (to confirm with W2): anchor.position.y is the FLOOR elevation
// at the seat; the seated pose drops the root so total seated height ≈ 1.7u.

import { getAnchors, registerAnchors } from '../shared/anchors';

let registered = false;

export function ensurePlaceholderAnchors(): void {
  if (registered) return;
  // If W2 anchors exist, never register placeholders.
  if (getAnchors().length > 0) {
    registered = true;
    return;
  }
  registerAnchors([
    {
      id: 'w3.placeholder.workbench.seatL',
      areaId: 'hall',
      kind: 'seat',
      position: [-1.2, 0, -9.2],
      facing: Math.PI, // face the workbench station at (0, 0, -10)
    },
    {
      id: 'w3.placeholder.workbench.seatR',
      areaId: 'hall',
      kind: 'seat',
      position: [1.2, 0, -9.2],
      facing: Math.PI,
    },
    {
      id: 'w3.placeholder.library.seat',
      areaId: 'hall',
      kind: 'seat',
      position: [9.2, 0, 1.2],
      facing: Math.PI / 2, // face the library pedestal at (10, 0, 0)
    },
  ]);
  registered = true;
}
