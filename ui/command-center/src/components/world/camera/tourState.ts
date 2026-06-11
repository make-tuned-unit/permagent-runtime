// W4 camera — tour mode activity flag (bible §7).
// Tiny module store so TourMode (drives the camera) and WorldCamera (must
// unmount OrbitControls while the tour owns the camera) stay in sync without
// prop drilling through WorldView.

import { useSyncExternalStore } from 'react';

let active = false;
const listeners = new Set<() => void>();

export function isTourActive(): boolean {
  return active;
}

export function setTourActive(value: boolean): void {
  if (active === value) return;
  active = value;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useTourActive(): boolean {
  return useSyncExternalStore(subscribe, isTourActive);
}
