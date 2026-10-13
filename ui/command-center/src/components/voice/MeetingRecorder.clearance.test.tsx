/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('../../lib/api', () => ({ api: {}, apiFetch: async () => [] }));
vi.mock('../../hooks/useMeetingDictation', async original => ({
  ...await original<typeof import('../../hooks/useMeetingDictation')>(),
  useMeetingDictation: () => ({
    state: 'recording', error: null, elapsedSeconds: 12, failedChunks: 0,
    target: { projectId: 'fixture', projectName: 'Synthetic review' }, hasUnsavedTranscript: true,
    dictationReadiness: { status: 'ready' }, start: vi.fn(), stop: vi.fn(), retrySave: vi.fn(), discard: vi.fn(),
    systemAudio: false, recordMic: true, systemAudioAvailable: async () => false,
    recoveredDrafts: [], triage: null, userNotes: 'Synthetic notes', setUserNotes: vi.fn(),
    activeSources: { mic: true, system: false, app: null }, nearChunksHeard: 1, farChunksHeard: 0,
  }),
}));
import { MeetingRecorder } from './MeetingRecorder';
import { useCommandCenter } from '../../lib/store';
import { setTheme } from '../../styles/tokens';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
beforeEach(() => { localStorage.clear(); localStorage.setItem('permagent-reduce-motion', 'true'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); act(() => setTheme('dark')); });
for (const [width, height] of [[1280, 800], [1024, 640]]) for (const theme of ['dark', 'silver'] as const) for (const docked of [false, true]) {
  it(`DOM recorder ${width}x${height} ${theme} dock-${docked ? 'open' : 'closed'}`, async () => {
    Object.defineProperties(window, { innerWidth: { configurable: true, value: width }, innerHeight: { configurable: true, value: height } });
    host.style.width = `${width}px`; host.style.height = `${height}px`;
    act(() => { setTheme(theme); useCommandCenter.setState({ chatDockOpen: docked }); });
    await act(async () => root.render(<>{docked && <div id="meeting-dock-slot" />}<MeetingRecorder open /></>));
    const panel = document.querySelector('[aria-label="Meeting notepad"]')!.parentElement!;
    expect(panel.hasAttribute('data-pa-launcher-footer')).toBe(!docked);
    expect(panel).toMatchSnapshot();
  });
}
