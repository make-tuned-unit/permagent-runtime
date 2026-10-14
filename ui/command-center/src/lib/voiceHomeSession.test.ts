import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./api', () => ({
  api: { getVoiceHomeSession: vi.fn(), createSession: vi.fn() },
}));

import { api } from './api';
import { resetVoiceHomeSession, resolveVoiceHomeSession } from './voiceHomeSession';

const home = vi.mocked(api.getVoiceHomeSession);
const create = vi.mocked(api.createSession);

function memoryStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

describe('resolveVoiceHomeSession', () => {
  beforeEach(() => {
    resetVoiceHomeSession();
    home.mockReset();
    create.mockReset();
    vi.stubGlobal('localStorage', memoryStorage());
  });
  afterEach(() => { vi.unstubAllGlobals(); });

  it('uses the daemon voice-home session and caches it', async () => {
    home.mockResolvedValue({ session_id: 'voice-1', name: 'Henry — voice', created: false });
    expect(await resolveVoiceHomeSession()).toBe('voice-1');
    expect(await resolveVoiceHomeSession()).toBe('voice-1');
    expect(home).toHaveBeenCalledTimes(1);
    expect(create).not.toHaveBeenCalled();
  });

  it('shares one in-flight request between concurrent callers', async () => {
    home.mockResolvedValue({ session_id: 'voice-1', name: 'n', created: true });
    const [a, b] = await Promise.all([resolveVoiceHomeSession(), resolveVoiceHomeSession()]);
    expect(a).toBe(b);
    expect(home).toHaveBeenCalledTimes(1);
  });

  it('falls back to a fresh plain chat when the daemon route fails, and reuses it', async () => {
    home.mockRejectedValue(new Error('404'));
    create.mockResolvedValue({ id: 'fresh-1' } as never);
    expect(await resolveVoiceHomeSession()).toBe('fresh-1');
    resetVoiceHomeSession();
    expect(await resolveVoiceHomeSession()).toBe('fresh-1');
    expect(create).toHaveBeenCalledTimes(1);
    // Created with no working dir and no project: never a coding session.
    expect(create).toHaveBeenCalledWith();
  });

  it('prefers the daemon again once it answers, dropping the fallback', async () => {
    home.mockRejectedValueOnce(new Error('down'));
    create.mockResolvedValue({ id: 'fresh-1' } as never);
    await resolveVoiceHomeSession();
    resetVoiceHomeSession();
    home.mockResolvedValue({ session_id: 'voice-9', name: 'n', created: false });
    expect(await resolveVoiceHomeSession()).toBe('voice-9');
  });

  it('rejects (and does not cache) when both paths fail', async () => {
    home.mockRejectedValue(new Error('down'));
    create.mockRejectedValue(new Error('down'));
    await expect(resolveVoiceHomeSession()).rejects.toThrow('down');
    home.mockResolvedValue({ session_id: 'voice-2', name: 'n', created: false });
    expect(await resolveVoiceHomeSession()).toBe('voice-2');
  });
});
