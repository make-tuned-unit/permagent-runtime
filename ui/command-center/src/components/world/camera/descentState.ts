// Descent-mode camera store (Carved Cave correction, C-nav).
// Mirrors tourState: a tiny external store so DescentMode (drives the camera down the
// vertical rail), WorldCamera (must unmount OrbitControls while descent owns the camera),
// and StratumPlaque (shows the active chamber's real plaque) stay in sync without prop
// drilling. The active stop index drives WHICH chamber's plaque is legible.

import { useSyncExternalStore } from 'react';

let active = false;
// Index of the current stratum stop on the descent rail. 0 = the verge (top stop);
// rail stops descend through each chamber to the cave floor. -1 = not on a stop yet.
let stopIndex = 0;
const listeners = new Set<() => void>();

function notify(): void {
  listeners.forEach((l) => l());
}

export function isDescentActive(): boolean {
  return active;
}

export function setDescentActive(value: boolean): void {
  if (active === value) return;
  active = value;
  if (value) stopIndex = 0; // entering descent starts at the verge stop
  notify();
}

export function getDescentStop(): number {
  return stopIndex;
}

export function setDescentStop(index: number): void {
  if (stopIndex === index) return;
  stopIndex = index;
  notify();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useDescentActive(): boolean {
  return useSyncExternalStore(subscribe, isDescentActive);
}

export function useDescentStop(): number {
  return useSyncExternalStore(subscribe, getDescentStop);
}
