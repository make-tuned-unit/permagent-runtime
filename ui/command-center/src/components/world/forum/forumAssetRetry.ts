import { useSyncExternalStore } from 'react';
import { useLoader } from '@react-three/fiber';

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
  target.load = (url, onLoad, onProgress, onError) => {
    noteForumAssetRequest(`gltf:${url}`, [url], () => useLoader.clear(proto as never, url));
    return load(url, data => { noteForumAssetLoaded(url); onLoad(data); }, onProgress, onError);
  };
}

export function resetForumAssetRetryForTests() { entries.clear(); loaded.clear(); }
