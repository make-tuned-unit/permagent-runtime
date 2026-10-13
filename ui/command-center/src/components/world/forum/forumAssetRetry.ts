import { useSyncExternalStore } from 'react';
import { useLoader } from '@react-three/fiber';
import { LoadingManager } from 'three';
import { WORLD_ASSET_DEADLINE_MS } from './ForumStageStatus';

/**
 * Retry World must request again whatever did not load. R3F's `useLoader`
 * caches every result by URL for the life of the page, including a rejection
 * and a request that never settled, so remounting the Canvas alone re-throws
 * the cached failure and "Retry World" can never recover. Loads register here;
 * `clearUnsettledForumAssets` drops only the entries that have not loaded, so
 * assets already decoded and uploaded are reused.
 */
type Entry = { urls: string[]; clear: () => void };
const entries = new Map<string, Entry>();
const loaded = new Set<string>();

export function noteForumAssetRequest(key: string, urls: string[], clear: () => void) {
  if (!entries.has(key)) entries.set(key, { urls, clear });
}
export function noteForumAssetLoaded(url: string) { loaded.add(url); }

/**
 * One loading manager for every World GLB and texture request. three's
 * FileLoader/ImageBitmapLoader fetches carry the manager's abort signal, and a
 * hung request also sits in FileLoader's module-level in-flight map, where the
 * next load of the same URL would join it instead of fetching again. Retry
 * aborts the manager, which rejects those fetches and frees the map entry.
 * (Per-request abort is not exposed by GLTFLoader, so the abort is all-or-
 * nothing; it only reaches requests that have not settled.)
 */
export const forumAssetManager = new LoadingManager();

export class ForumAssetTimeoutError extends Error {
  constructor(readonly url: string) { super(`World asset did not load within ${WORLD_ASSET_DEADLINE_MS / 1000} s: ${url}`); this.name = 'ForumAssetTimeoutError'; }
}

export type ForumAssetHealth = { pending: number; timedOut: number };
let health: ForumAssetHealth = { pending: 0, timedOut: 0 };
const healthListeners = new Set<() => void>();
const publishHealth = (next: Partial<ForumAssetHealth>) => {
  health = { ...health, ...next };
  healthListeners.forEach(listener => listener());
};
const subscribeForumHealth = (listener: () => void) => { healthListeners.add(listener); return () => { healthListeners.delete(listener); }; };
const readForumHealth = () => health;
/** Requests still in flight, and requests that hit the deadline since the last Retry. */
export function useForumAssetHealth(): ForumAssetHealth {
  return useSyncExternalStore(subscribeForumHealth, readForumHealth, readForumHealth);
}

const inFlight = new Map<symbol, () => void>();
/**
 * Wraps a loader's callbacks with the World asset deadline. The first of load,
 * error or deadline wins; a result arriving after the deadline (or after Retry
 * cancelled the request) is ignored. Records loads for `clearUnsettledForumAssets`.
 */
export function guardForumAssetRequest<T>(url: string, onLoad?: (data: T) => void, onError?: (error: unknown) => void) {
  const token = Symbol(url);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const finish = () => {
    if (!inFlight.delete(token)) return false;
    if (timer !== undefined) clearTimeout(timer);
    return true;
  };
  inFlight.set(token, finish);
  publishHealth({ pending: inFlight.size });
  timer = setTimeout(() => {
    if (!finish()) return;
    publishHealth({ pending: inFlight.size, timedOut: health.timedOut + 1 });
    onError?.(new ForumAssetTimeoutError(url));
  }, WORLD_ASSET_DEADLINE_MS);
  return {
    onLoad: (data: T) => { if (!finish()) return; noteForumAssetLoaded(url); publishHealth({ pending: inFlight.size }); onLoad?.(data); },
    onError: (error: unknown) => { if (!finish()) return; publishHealth({ pending: inFlight.size }); onError?.(error); },
  };
}

/**
 * Retry generation. Components that latch an asset failure outside the forum
 * Canvas (the agent portraits keep their own Canvas and failure boundary, so
 * remounting the stage does not reach them) key that latch by this value, so
 * Retry World gives them a fresh attempt too.
 */
let retryGeneration = 0;
const retryListeners = new Set<() => void>();
const subscribeForumRetry = (listener: () => void) => { retryListeners.add(listener); return () => { retryListeners.delete(listener); }; };
const readForumRetry = () => retryGeneration;
export function useForumRetryGeneration(): number {
  return useSyncExternalStore(subscribeForumRetry, readForumRetry, readForumRetry);
}

/** Retry World: clears unsettled cache entries and starts a new retry
 * generation; returns how many entries were dropped. */
export function clearUnsettledForumAssets(): number {
  retryGeneration++;
  retryListeners.forEach(listener => listener());
  // Cancel unsettled requests silently (their cache entries are dropped below), then abort the
  // fetches so a hung request no longer holds the URL in three's in-flight map.
  for (const finish of [...inFlight.values()]) finish();
  publishHealth({ pending: 0, timedOut: 0 });
  try { forumAssetManager.abort(); } catch { /* abort is best-effort */ }
  let cleared = 0;
  for (const [key, entry] of entries) {
    if (entry.urls.every(url => loaded.has(url))) continue;
    entry.clear(); entries.delete(key); cleared++;
  }
  return cleared;
}

type LoaderLike = { load: (url: string, onLoad: (data: unknown) => void, onProgress?: (event: ProgressEvent) => void, onError?: (error: unknown) => void) => unknown };
const tracked = new WeakSet<object>();
/** Called from the shared GLTF loader's `extendLoader`; the loader instance is
 * reused by every `useGLTF` call, so it is wrapped once. Pass-through otherwise. */
export function trackForumGltfLoader(loader: object) {
  if (tracked.has(loader) || typeof (loader as Partial<LoaderLike>).load !== 'function') return;
  tracked.add(loader);
  const target = loader as LoaderLike;
  const proto = loader.constructor as new () => unknown;
  const load = target.load.bind(loader);
  (loader as { manager?: LoadingManager }).manager = forumAssetManager;
  target.load = (url, onLoad, onProgress, onError) => {
    noteForumAssetRequest(`gltf:${url}`, [url], () => useLoader.clear(proto as never, url));
    const guarded = guardForumAssetRequest<unknown>(url, onLoad, onError);
    return load(url, guarded.onLoad, onProgress, guarded.onError);
  };
}

export function resetForumAssetRetryForTests() {
  entries.clear(); loaded.clear();
  for (const finish of [...inFlight.values()]) finish();
  publishHealth({ pending: 0, timedOut: 0 });
}
