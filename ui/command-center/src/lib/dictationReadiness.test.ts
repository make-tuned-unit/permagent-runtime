// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
const { fetchMock, resolveMock } = vi.hoisted(() => ({ fetchMock: vi.fn(), resolveMock: vi.fn() }));
vi.mock('./api', () => ({ apiFetch: fetchMock }));
vi.mock('@tauri-apps/api/path', () => ({ resolveResource: resolveMock }));
const available = { status: 'available', reason: 'model_available', model: 'base', inference_verified: false };
const missing = { status: 'unavailable', reason: 'not_configured', model: null, inference_verified: false };
beforeEach(() => {
  vi.resetModules(); fetchMock.mockReset(); resolveMock.mockReset();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});
describe('dictation readiness', () => {
  it('deduplicates app/Record checks and accepts authoritative file availability', async () => {
    let complete!: (value: unknown) => void;
    fetchMock.mockReturnValue(new Promise(resolve => { complete = resolve; }));
    const store = await import('./dictationReadiness');
    const first = store.ensureDictationReadiness();
    expect(store.ensureDictationReadiness()).toBe(first);
    expect(store.getDictationReadiness().status).toBe('checking');
    complete(available); await first;
    expect(store.getDictationReadiness().status).toBe('ready');
    expect(store.getDictationReadiness().message).toContain('when audio is processed');
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
  it('browser missing setup never provisions or resolves a native path', async () => {
    fetchMock.mockResolvedValue(missing);
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('unavailable');
    expect(store.getDictationReadiness().message).toContain('connected computer');
    expect(resolveMock).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls.every(([url]) => url === '/api/dictation/status')).toBe(true);
  });
  it('native provisions then checks rather than trusting the acknowledgement', async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    resolveMock.mockResolvedValue('/bundle/whisper.gguf');
    fetchMock.mockResolvedValueOnce(missing).mockResolvedValueOnce({ status: 'ready' }).mockResolvedValueOnce(available);
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual(['/api/dictation/status', '/api/dictation/provision', '/api/dictation/status']);
    expect(store.getDictationReadiness().status).toBe('ready');
  });
  it('native missing bundle failure is visible and retry recovers without leaking paths', async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    fetchMock.mockResolvedValueOnce(missing);
    resolveMock.mockRejectedValue(new Error('/private/path must not leak'));
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('error');
    expect(store.getDictationReadiness().message).not.toContain('/private/path');
    fetchMock.mockResolvedValueOnce(available); await store.retryDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('ready');
  });
  it('older backend blocks with update guidance until retry succeeds', async () => {
    fetchMock.mockRejectedValueOnce({ status: 404 }).mockResolvedValueOnce(available);
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('error');
    expect(store.getDictationReadiness().message).toContain('Update');
    await store.retryDictationReadiness(); expect(store.getDictationReadiness().status).toBe('ready');
  });
  it.each(['feature_disabled', 'unsupported_model', 'unreadable_model', 'invalid_model', 'config_unavailable'])('does not overwrite existing setup on %s', async reason => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    fetchMock.mockResolvedValue({ ...missing, reason });
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('unavailable');
    expect(resolveMock).not.toHaveBeenCalled(); expect(fetchMock).toHaveBeenCalledTimes(1);
  });
  it('does not replace a missing user-selected model with bundled base', async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    fetchMock.mockResolvedValue({ ...missing, reason: 'missing_model', model: 'small' });
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('unavailable');
    expect(resolveMock).not.toHaveBeenCalled();
  });
  it('aborts a stalled backend check and makes retry available', async () => {
    vi.useFakeTimers();
    try {
      fetchMock.mockImplementation((_url, options) => new Promise((_resolve, reject) => {
        options.signal.addEventListener('abort', () => reject(new Error('aborted')));
      }));
      const store = await import('./dictationReadiness');
      const checking = store.ensureDictationReadiness();
      await vi.advanceTimersByTimeAsync(20_000); await checking;
      expect(store.getDictationReadiness().status).toBe('error');
      expect(store.getDictationReadiness().message).toContain('timed out');
      fetchMock.mockResolvedValueOnce(available); await store.retryDictationReadiness();
      expect(store.getDictationReadiness().status).toBe('ready');
    } finally { vi.useRealTimers(); }
  });
  it('rejects optimistic malformed status', async () => {
    fetchMock.mockResolvedValue({ status: 'available' });
    const store = await import('./dictationReadiness'); await store.ensureDictationReadiness();
    expect(store.getDictationReadiness().status).toBe('error');
  });
});
