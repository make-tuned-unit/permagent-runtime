/**
 * systemAudio — the renderer's side of the ScreenCaptureKit helper
 * (`ui/desktop/src-tauri/src/system_audio.rs`). The far side of a call: the
 * microphone hears only the user.
 *
 * Every call is a no-op outside the desktop app. Chunk paths arrive two ways:
 * as `system_audio_chunk` events while capturing, and — from natives that
 * support it — as the return value of `stop_system_audio`, which lists the
 * chunks still unread once the helper's stdout reader has been joined. The two
 * overlap by design, so consumers must dedupe by path.
 */

export const SYSTEM_AUDIO_CHUNK_EVENT = 'system_audio_chunk';
export const SYSTEM_AUDIO_ERROR_EVENT = 'system_audio_error';

export interface SystemAudioErrorPayload {
  kind: string;
  detail: string;
}

function inDesktopApp(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

// Loaded once and shared: the chunk path issues reads concurrently with the
// stop, and one module promise keeps every call on the same instance.
let corePromise: Promise<typeof import('@tauri-apps/api/core')> | null = null;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  corePromise ??= import('@tauri-apps/api/core');
  const core = await corePromise;
  return core.invoke<T>(cmd, args);
}

/** Whether this build carries the capture helper at all. */
export async function systemAudioAvailable(): Promise<boolean> {
  if (!inDesktopApp()) return false;
  try {
    return await invoke<boolean>('system_audio_available');
  } catch {
    return false;
  }
}

/** A running app whose audio can be captured on its own. */
export interface SystemAudioApp {
  bundleId: string;
  name: string;
}

/** Normalise `list_system_audio_apps`: well-formed entries only, sorted by
 *  name. Pure; tested. */
export function normalizeAppList(raw: unknown): SystemAudioApp[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((a): a is SystemAudioApp =>
      !!a && typeof a === 'object'
      && typeof (a as SystemAudioApp).bundleId === 'string' && (a as SystemAudioApp).bundleId.trim() !== ''
      && typeof (a as SystemAudioApp).name === 'string')
    .map(a => ({ bundleId: a.bundleId, name: a.name.trim() || a.bundleId }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Running apps that could be captured on their own. `null` means this native
 * cannot target an app at all (the command is missing, or we are outside the
 * desktop app) — distinct from an empty list, which means it can but nothing
 * is running. Callers must not offer per-app capture on `null`: the helper
 * would record everything while the UI claimed one app.
 */
export async function listSystemAudioApps(): Promise<SystemAudioApp[] | null> {
  if (!inDesktopApp()) return null;
  try {
    return normalizeAppList(await invoke<unknown>('list_system_audio_apps'));
  } catch {
    return null;
  }
}

/** Begin capture into `outDir`, cutting a chunk every `chunkSeconds`. With
 *  `includeAppBundleId`, only that app's audio is captured; without it, all
 *  system audio (the original behaviour). The key is sent only when set, so
 *  an older native sees exactly the arguments it always did. */
export async function startSystemAudioCapture(
  outDir: string,
  chunkSeconds: number,
  includeAppBundleId?: string | null,
): Promise<void> {
  if (!inDesktopApp()) return;
  await invoke('start_system_audio', {
    outDir,
    chunkSeconds,
    ...(includeAppBundleId ? { includeAppBundleId } : {}),
  });
}

/**
 * Normalise what `stop_system_audio` returned. Newer natives return the
 * sorted unread WAV paths; older ones return nothing (`null`/unit), which is
 * reported as `null` so the caller can tell "none left" from "not supported".
 * Pure; tested.
 */
export function normalizeStopResult(raw: unknown): string[] | null {
  if (!Array.isArray(raw)) return null;
  return raw.filter((p): p is string => typeof p === 'string' && p.trim().length > 0);
}

export interface SystemAudioStopReceipt { unreadPaths: string[] | null; error: string | null }

/** Accept current receipts and older path-array/unit protocols. */
export function normalizeStopReceipt(raw: unknown): SystemAudioStopReceipt {
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    const receipt = raw as { unreadPaths?: unknown; error?: unknown };
    const paths = normalizeStopResult(receipt.unreadPaths);
    const validError = receipt.error === null || typeof receipt.error === 'string';
    return { unreadPaths: paths, error: paths !== null && validError
      ? (receipt.error as string | null)
      : 'The capture helper returned an invalid stop receipt; final audio may be incomplete' };
  }
  return { unreadPaths: normalizeStopResult(raw), error: raw == null || Array.isArray(raw)
    ? null : 'The capture helper returned an invalid stop receipt; final audio may be incomplete' };
}

/** Preserve unread tails even when native reports an unsuccessful final flush. */
export async function stopSystemAudioCapture(): Promise<SystemAudioStopReceipt> {
  if (!inDesktopApp()) return { unreadPaths: null, error: null };
  return normalizeStopReceipt(await invoke<unknown>('stop_system_audio'));
}

/** Read (and natively delete) one completed chunk. */
export async function readSystemAudioChunk(path: string): Promise<Uint8Array<ArrayBuffer>> {
  return new Uint8Array(await invoke<number[]>('read_audio_chunk', { path }));
}

/** Subscribe to chunk paths and capture errors. Resolves to one unsubscribe. */
export async function listenSystemAudio(
  onChunk: (path: string) => void,
  onError: (e: SystemAudioErrorPayload) => void,
): Promise<() => void> {
  if (!inDesktopApp()) return () => {};
  const { listen } = await import('@tauri-apps/api/event');
  const offs: Array<() => void> = [];
  const off = () => offs.forEach(fn => { try { fn(); } catch { /* already gone */ } });
  try {
    offs.push(await listen<string>(SYSTEM_AUDIO_CHUNK_EVENT, e => onChunk(e.payload)));
    offs.push(await listen<SystemAudioErrorPayload>(SYSTEM_AUDIO_ERROR_EVENT, e => onError(e.payload)));
  } catch (e) {
    off();
    throw e;
  }
  return off;
}
