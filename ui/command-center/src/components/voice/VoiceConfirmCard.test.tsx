/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { resolveVoiceConfirm, listeners } = vi.hoisted(() => ({
  resolveVoiceConfirm: vi.fn(() => Promise.resolve({ confirm_id: 'c1', outcome: 'approved' })),
  listeners: new Set<(e: { type: string; payload: Record<string, unknown>; replayed: boolean }) => void>(),
}));
vi.mock('../../lib/api', () => ({ api: { resolveVoiceConfirm }, getApiBaseUrl: () => '' }));
vi.mock('../world/shared/worldEvents', () => ({
  subscribeWorldEvents: (l: (e: never) => void) => { listeners.add(l as never); return () => listeners.delete(l as never); },
}));

import { VoiceConfirmCard } from './VoiceConfirmCard';
import { resetVoiceConfirms } from '../../lib/voiceConfirm';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const emit = (type: string, payload: Record<string, unknown>, replayed = false) =>
  act(() => listeners.forEach(l => l({ type, payload, replayed })));
const request = (ms = 30_000) => ({
  confirm_id: 'c1', session_id: 's1', tool: 'dispatch_goal', summary: 'Start the grow run',
  args_preview: '{"goal":"grow"}', expires_at: new Date(Date.now() + ms).toISOString(),
});
const buttons = () => Array.from(container.querySelectorAll('button'));

beforeEach(() => {
  resolveVoiceConfirm.mockClear(); resetVoiceConfirms();
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
  act(() => root.render(<VoiceConfirmCard />));
});
afterEach(() => { act(() => root.unmount()); container.remove(); vi.useRealTimers(); });

describe('VoiceConfirmCard', () => {
  it('shows the summary and a countdown, and Confirm approves with the session id', async () => {
    emit('voice_confirm_requested', request());
    expect(container.textContent).toContain('Start the grow run');
    expect(container.textContent).toMatch(/\d+s/);
    const confirm = buttons().find(b => b.textContent?.includes('Confirm'))!;
    await act(async () => { confirm.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(resolveVoiceConfirm).toHaveBeenCalledWith('c1', 's1', 'approve');
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).toBeNull();
  });
  it('Cancel denies', async () => {
    emit('voice_confirm_requested', request());
    const cancel = buttons().find(b => b.textContent?.includes('Cancel'))!;
    await act(async () => { cancel.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(resolveVoiceConfirm).toHaveBeenCalledWith('c1', 's1', 'deny');
  });
  it('dismisses when the server resolves it elsewhere, and ignores replayed requests', () => {
    emit('voice_confirm_requested', request(), true);
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).toBeNull();
    emit('voice_confirm_requested', request());
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).not.toBeNull();
    emit('voice_confirm_resolved', { confirm_id: 'c1', session_id: 's1', outcome: 'denied' });
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).toBeNull();
  });
  it('lapses at expires_at', () => {
    vi.useFakeTimers();
    emit('voice_confirm_requested', request(2000));
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).not.toBeNull();
    act(() => { vi.advanceTimersByTime(2600); });
    expect(container.querySelector('[data-testid="voice-confirm-card"]')).toBeNull();
  });
});
