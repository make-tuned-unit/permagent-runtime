/**
 * loadSessions failure honesty (C6 nav-honesty) — store tests.
 *
 * A silent catch used to collapse "daemon unreachable" into `sessions: []`,
 * which SessionsList rendered as "No sessions yet. Click + New" — a lie. The
 * store now latches `sessionsError` (#568 empty-body lesson, mirroring
 * providersError) so the list can render an inline error + retry, and clears
 * it on the next successful load.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const { getSessions } = vi.hoisted(() => ({
  getSessions: vi.fn(),
}));

vi.mock('./api', () => ({
  api: { getSessions },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { useCommandCenter } from './store';

beforeEach(() => {
  getSessions.mockReset();
  useCommandCenter.setState({ sessions: [], sessionsError: false });
});

describe('loadSessions error latch', () => {
  it('latches sessionsError when the daemon is unreachable instead of faking an empty list', async () => {
    getSessions.mockRejectedValue(new Error('ECONNREFUSED'));
    await useCommandCenter.getState().loadSessions();
    const s = useCommandCenter.getState();
    expect(s.sessionsError).toBe(true);
    expect(s.sessions).toEqual([]);
  });

  it('clears the latch and stores the list on a successful retry', async () => {
    getSessions.mockRejectedValueOnce(new Error('down'));
    await useCommandCenter.getState().loadSessions();
    expect(useCommandCenter.getState().sessionsError).toBe(true);

    getSessions.mockResolvedValueOnce([
      { id: 's1', name: 'First chat', created_at: 'c', updated_at: 'u', message_count: 3 },
    ]);
    await useCommandCenter.getState().loadSessions();
    const s = useCommandCenter.getState();
    expect(s.sessionsError).toBe(false);
    expect(s.sessions).toEqual([
      { id: 's1', name: 'First chat', created_at: 'c', updated_at: 'u', message_count: 3 },
    ]);
  });

  it('a genuinely empty daemon is NOT an error (the empty state stays honest)', async () => {
    getSessions.mockResolvedValue([]);
    await useCommandCenter.getState().loadSessions();
    const s = useCommandCenter.getState();
    expect(s.sessionsError).toBe(false);
    expect(s.sessions).toEqual([]);
  });
});
