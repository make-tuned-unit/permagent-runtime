import { useEffect, useSyncExternalStore } from 'react';
import { apiFetch } from './api';

type Status = 'checking' | 'ready' | 'unavailable' | 'error';
export interface DictationReadiness { status: Status; message: string | null }
interface BackendStatus { status: string; reason: string; model: string | null; inference_verified: boolean }
let snapshot: DictationReadiness = { status: 'checking', message: 'Checking local transcription…' };
const listeners = new Set<() => void>();
let started = false;
let pending: Promise<void> | null = null;
const publish = (value: DictationReadiness) => { snapshot = value; listeners.forEach(listener => listener()); };
export const getDictationReadiness = () => snapshot;
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const reasons: Record<string, string> = {
  not_configured: 'No local transcription model is configured.',
  missing_model: 'The configured local transcription model is missing.',
  unreadable_model: 'The local transcription model cannot be read.',
  invalid_model: 'The local transcription model file is invalid or incomplete.',
  unsupported_model: 'The configured transcription model is not supported by this service.',
  feature_disabled: 'This background service was built without local transcription.',
  config_unavailable: 'The background service could not read transcription settings.',
};

/** One in-flight setup/check shared by App and Record, including StrictMode.
 * Browsers only inspect their paired daemon; never guess a host resource path.
 */
export function retryDictationReadiness(): Promise<void> {
  if (pending) return pending;
  started = true;
  publish({ status: 'checking', message: 'Preparing local transcription…' });
  const native = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 20_000);
  pending = (async () => {
    try {
      const check = () => apiFetch<BackendStatus>('/api/dictation/status', { signal: controller.signal });
      let result = await check();
      if (native && result.status === 'unavailable' && (result.reason === 'not_configured' || (result.reason === 'missing_model' && result.model === 'base'))) {
        const { resolveResource } = await import('@tauri-apps/api/path');
        const path = await resolveResource('whisper/whisper-base-q8_0.gguf');
        await apiFetch('/api/dictation/provision', {
          method: 'POST', body: JSON.stringify({ bundled_path: path }), signal: controller.signal,
        });
        // A provision acknowledgement alone does not establish file availability.
        result = await check();
      }
      if (result.status === 'available' && result.reason === 'model_available' && typeof result.model === 'string' && result.model.length > 0 && result.inference_verified === false) {
        publish({ status: 'ready', message: 'Local transcription model is available. Transcription is checked when audio is processed.' });
      } else if (result.status === 'unavailable' && reasons[result.reason]) {
        publish({ status: 'unavailable', message: `${reasons[result.reason]} ${native ? 'Retry setup. If it still fails, reinstall the current desktop app with its bundled model.' : 'Open the desktop app on the connected computer to finish model setup, then retry.'}` });
      } else {
        throw new Error('invalid_status');
      }
    } catch (error) {
      const status = (error as { status?: number })?.status;
      publish({ status: 'error', message: status === 404
        ? 'This background service cannot check transcription readiness. Update the connected desktop app and background service, then retry.'
        : controller.signal.aborted
          ? 'Local transcription setup timed out. Retry when the background service is ready.'
          : 'Local transcription setup could not be verified. Check the background service and bundled model, then retry.' });
    } finally {
      clearTimeout(timeout);
      pending = null;
    }
  })();
  return pending;
}

export function ensureDictationReadiness(): Promise<void> {
  return pending ?? (started ? Promise.resolve() : retryDictationReadiness());
}
export function useDictationReadiness(enabled = true) {
  const value = useSyncExternalStore(subscribe, getDictationReadiness, getDictationReadiness);
  useEffect(() => { if (enabled) void ensureDictationReadiness(); }, [enabled]);
  return { ...value, retry: retryDictationReadiness };
}
