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
  extractSystemNotice: vi.fn(() => null),
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
    // The store also carries lib/sessionLabel.ts's naming/classifying fields
    // (working_dir, schedule_id, …) for every session, defaulted to null when
    // the daemon response omits them.
    expect(s.sessions).toEqual([
      {
        id: 's1', name: 'First chat', created_at: 'c', updated_at: 'u', message_count: 3,
        user_set_name: undefined, session_type: undefined,
        working_dir: null, schedule_id: null, recipe_title: null, first_user_text: null,
        goal_id: null, goal_title: null, goal_project_id: null, project_id: null, project_name: null,
      },
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


describe('session refresh races and retained data', () => {
  it('reports pending, retains cached records on failure, and clears pending', async () => {
    const cached = { id: 'cached', name: 'Saved conversation', created_at: 'c', message_count: 1 };
    useCommandCenter.setState({ sessions: [cached] });
    let reject!: (error: Error) => void;
    getSessions.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
    const pending = useCommandCenter.getState().loadSessions();
    expect(useCommandCenter.getState().sessionsLoading).toBe(true);
    expect(useCommandCenter.getState().sessions).toEqual([cached]);
    reject(new Error('offline'));
    await pending;
    expect(useCommandCenter.getState().sessionsLoading).toBe(false);
    expect(useCommandCenter.getState().sessionsError).toBe(true);
    expect(useCommandCenter.getState().sessions).toEqual([cached]);
  });

  it('an older failed request cannot replace a newer successful response', async () => {
    let reject!: (error: Error) => void;
    getSessions.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
    const older = useCommandCenter.getState().loadSessions();
    getSessions.mockResolvedValueOnce([{ id: 'new', name: 'New', created_at: 'c', message_count: 1 }]);
    await useCommandCenter.getState().loadSessions();
    reject(new Error('late failure'));
    await older;
    expect(useCommandCenter.getState().sessions[0].id).toBe('new');
    expect(useCommandCenter.getState().sessionsError).toBe(false);
    expect(useCommandCenter.getState().sessionsLoading).toBe(false);
  });
});
