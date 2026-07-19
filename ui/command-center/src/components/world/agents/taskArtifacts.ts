// Task artifacts — REAL completions leave something behind on the benches.
//
// The daemon emits task_completed / task_failed on /events for every tracked
// agent task (goose tasks/mod.rs). Each LIVE completion sets a small glowing
// work-stone on the bay benches that cools from amber to plain stone over ten
// minutes — the evening's work, visible. A LIVE failure leaves a brief red
// ember that dies quickly. Replayed buffer events are ignored; the bench
// starts the session honestly empty.
//
// This is "artifacts appearing on real completions": the stones are a
// session-scoped, event-driven record — never decorative clutter, never
// pre-populated.
//
// PURE CORE (unit-tested): a FIFO reducer over (kind, id, now) + an
// age-resolved visual. Zero per-frame allocation: BenchArtifacts re-buckets on
// a slow interval, not per frame.

export type ArtifactKind = 'completed' | 'failed';

export interface TaskArtifact {
  id: string;
  kind: ArtifactKind;
  /** Epoch ms the event arrived (live). */
  at: number;
}

/** Bench capacity — matches the 2×6 stone slots BenchArtifacts lays out. */
export const ARTIFACT_CAP = 12;
export const COMPLETED_TTL_MS = 10 * 60_000;
export const FAILED_TTL_MS = 60_000;
/** Fresh-glow window: a completed stone reads amber this long, then cools. */
export const GLOW_MS = 90_000;

let artifacts: TaskArtifact[] = [];
const listeners = new Set<() => void>();

function publish(): void {
  listeners.forEach((l) => l());
}

/**
 * Pure reducer (unit-tested): append one live task event to a FIFO list.
 * A task id that completes after failing (retry succeeded) replaces its entry.
 */
export function reduceArtifact(
  list: TaskArtifact[],
  kind: ArtifactKind,
  id: string,
  now: number,
): TaskArtifact[] {
  const next = list.filter((a) => a.id !== id);
  next.push({ id, kind, at: now });
  while (next.length > ARTIFACT_CAP) next.shift();
  return next;
}

/** Pure expiry sweep (unit-tested). */
export function pruneArtifacts(list: TaskArtifact[], now: number): TaskArtifact[] {
  return list.filter(
    (a) => now - a.at < (a.kind === 'completed' ? COMPLETED_TTL_MS : FAILED_TTL_MS),
  );
}

export interface ArtifactVisual {
  /** 0 fresh … 1 expired (age over its TTL). */
  age01: number;
  /** Emissive strength [0,1]: completed cools over GLOW_MS; failed ember dies fast. */
  glow: number;
}

/** Pure visual resolution (unit-tested). */
export function artifactVisual(a: TaskArtifact, now: number): ArtifactVisual {
  const ttl = a.kind === 'completed' ? COMPLETED_TTL_MS : FAILED_TTL_MS;
  const age = Math.max(0, now - a.at);
  const age01 = Math.min(1, age / ttl);
  const glow =
    a.kind === 'completed' ? Math.max(0, 1 - age / GLOW_MS) : Math.max(0, 1 - age / FAILED_TTL_MS);
  return { age01, glow };
}

/** Live-event entry point (called from the /events binding — live only). */
export function noteTaskEvent(kind: ArtifactKind, id: string, now: number): void {
  artifacts = reduceArtifact(artifacts, kind, id, now);
  publish();
}

/** Sweep expired stones; publishes only when something actually left. */
export function sweepArtifacts(now: number): void {
  const next = pruneArtifacts(artifacts, now);
  if (next.length !== artifacts.length) {
    artifacts = next;
    publish();
  }
}

export function getArtifacts(): TaskArtifact[] {
  return artifacts;
}

export function subscribeArtifacts(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}
