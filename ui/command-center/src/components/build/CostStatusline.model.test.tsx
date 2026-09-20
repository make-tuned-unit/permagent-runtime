/**
 * @vitest-environment jsdom
 *
 * The statusline model picker (the user 2026-09-18: "The model picker dropdown
 * disappeared. It would make sense for it to be on the bottom bar with the
 * costs and I could switch between models.").
 *
 * What is worth testing here is not that a picker renders — it is the WIRING:
 * the statusline picker and the terminal's own picker must be two views of one
 * selection, so a model chosen on the bottom bar is the model the terminal
 * shows, and neither surface can drift from the other. The second test covers
 * the honest-degradation half: with no harness session there is no live
 * control to click, only a note saying why.
 *
 * ProviderModelPicker is mocked — both surfaces import the same module, so one
 * stub stands in for both and exposes the value each was handed.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';

const { invoke, terminalProps } = vi.hoisted(() => ({
  // Typed so a call can be inspected by name and payload below.
  invoke: vi.fn((_command: string, _args?: { sessionId?: string; data?: string }) => Promise.resolve(undefined)),
  terminalProps: { current: null as null | { onPtyData?: (data: string) => void } },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../terminal/Terminal', () => ({
  Terminal: (props: { onPtyData?: (data: string) => void }) => { terminalProps.current = props; return <div />; },
}));
vi.mock('../../lib/native-drag-drop', () => ({ registerDropZone: () => () => {} }));
vi.mock('../../lib/useLiveGoals', () => ({
  useLiveGoals: () => ({ goals: [], activeCount: 0, loaded: true, refresh: () => {} }),
}));
vi.mock('../../lib/api', () => ({
  api: {
    getSessionCost: vi.fn(async () => ({ own: 0, childrenTotal: 0, perChild: [] })),
    getActiveHarnessRuns: vi.fn(async () => []),
    getPacks: vi.fn(async () => ({ prompt: false, configured: [], recommendation: { recommendations: [], considered: [] } })),
    // The idle control reads the harness role's saved default and writes it
    // back through the same call Settings → Models makes.
    readConfig: vi.fn(async (key: string) => (key === 'harness_model' ? 'glm-5.3' : 'zai')),
    setModelRoute: vi.fn(async () => ({})),
  },
}));
vi.mock('../settings/ProviderModelPicker', () => ({
  ProviderModelPicker: ({ onChange, value, 'aria-label': label }: {
    onChange: (value: { provider: string; model: string }) => void;
    value: { provider: string | null; model: string | null };
    'aria-label'?: string;
  }) => (
    <button
      type="button"
      data-testid="mock-provider-model"
      data-label={label}
      data-value={`${value.provider ?? ''}/${value.model ?? ''}`}
      onClick={() => onChange({ provider: 'openai', model: 'gpt-5' })}
    >pick</button>
  ),
}));

import { CostStatusline } from './CostStatusline';
import { TerminalManager, __resetTerminalPersistenceForTests } from '../terminal/TerminalManager';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const HARNESS_TAB = {
  id: 'harness',
  label: 'Build',
  sessionId: 'pty-1',
  initialCommand: 'permagent run --recipe permagent-coding --interactive',
};

let container: HTMLDivElement;
let root: Root;

function resetStore() {
  useCommandCenter.setState({
    liveTokens: null,
    codingSpend: null,
    codingSpendLastKnown: null,
    codingHarnessHydration: 'initial',
    chatSessionId: null,
    harnessModelSessionId: null,
    harnessModelSelections: {},
    harnessModelRequest: null,
  });
}

beforeEach(() => {
  __resetTerminalPersistenceForTests();
  resetStore();
  invoke.mockClear();
  terminalProps.current = null;
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = { invoke };
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  resetStore();
  vi.clearAllMocks();
});

describe('CostStatusline model picker', () => {
  it('switches the harness model from the bottom bar, and the terminal shows the same selection', async () => {
    await act(async () => {
      root.render(<><TerminalManager initialTab={HARNESS_TAB} /><CostStatusline /></>);
    });

    // The docked terminal no longer carries its own picker: the bar has the
    // only one, so there is exactly one stub on screen and it is the
    // statusline's.
    expect(container.querySelector('[data-testid="terminal-model-control"]')).toBeNull();
    const pickers = container.querySelectorAll('[data-testid="mock-provider-model"]');
    expect(pickers.length).toBe(1);
    expect(pickers[0].getAttribute('data-label')).toBe('Choose harness provider and model');
    expect(container.querySelector('[data-testid="statusline-model-control"]')).not.toBeNull();
    // The spend figure is still first on the line, ahead of the picker.
    const bar = container.querySelector('[role="status"]')!;
    expect(bar.textContent).toContain('$0.00');
    expect(bar.compareDocumentPosition(pickers[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // Choosing on the bar writes to the harness PTY the terminal published.
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="mock-provider-model"]')!.click();
      await vi.waitFor(() => {
        // Matched on the command and its payload rather than on the exact call
        // shape: the write now goes through the store, and `invoke` reaches the
        // spy with a different argument count depending on whether the module
        // mock or the Tauri internals shim answers it.
        expect(invoke.mock.calls.some(([command, args]) => command === 'write_to_pty'
          && args?.sessionId === 'pty-1'
          && args?.data === '/model openai/gpt-5\r')).toBe(true);
      });
    });
    // Still a request, not a claim: the CLI has not confirmed anything yet.
    expect(container.textContent).toContain('requested openai/gpt-5');
    expect(container.querySelector('[data-testid="mock-provider-model"]')?.getAttribute('data-value')).toBe('/');

    // The CLI's own confirmation, read off the PTY by the terminal, is what
    // promotes the request to a selection — on the bar that requested it.
    await act(async () => {
      terminalProps.current?.onPtyData?.('\u001b[32mHarness model switched for this session only: openai/gpt-5. Chat settings were not changed.\u001b[0m\r\n');
    });
    expect(container.querySelector('[data-testid="mock-provider-model"]')?.getAttribute('data-value')).toBe('openai/gpt-5');
    expect(container.textContent).not.toContain('requested openai/gpt-5');
    expect(useCommandCenter.getState().harnessModelSelections['pty-1']).toEqual({ provider: 'openai', model: 'gpt-5' });

    // …and the terminal's own picker (the detached pane window, the one place
    // it still renders) reads that same selection. One value, two views.
    await act(async () => {
      root.render(<TerminalManager initialTab={HARNESS_TAB} detached />);
    });
    const terminalPicker = container.querySelector('[data-testid="terminal-model-control"] [data-testid="mock-provider-model"]');
    expect(terminalPicker?.getAttribute('data-value')).toBe('openai/gpt-5');
  });

  it('keeps its right end clear of the floating chat pill', async () => {
    const { statuslineRightPad } = await import('./CostStatusline');
    // QA 2026-09-20: the pill covered "Change model" on the Build bar.
    expect(statuslineRightPad({ width: 150, height: 44 })).toBeGreaterThanOrEqual(150 + 18);
    expect(statuslineRightPad(null)).toBe(18);
  });

  // The user's ask was "it would make sense for it to be on the bottom bar
  // with the costs and I could switch between models". With no session running
  // the bar used to print "no harness — model fixed" and offer nothing; it now
  // sets the model the NEXT run starts on — the harness role Settings writes.
  it('with no harness session, sets the model the next run starts on', async () => {
    const { api } = await import('../../lib/api');
    await act(async () => {
      root.render(
        <>
          <TerminalManager initialTab={{ id: 'shell', label: 'zsh', sessionId: 'pty-9', initialCommand: 'zsh' }} />
          <CostStatusline />
        </>,
      );
    });

    expect(useCommandCenter.getState().harnessModelSessionId).toBeNull();
    // Not the live-session control...
    expect(container.querySelector('[data-testid="statusline-model-control"]')).toBeNull();
    // ...the default one, which says which run it affects.
    const idle = container.querySelector('[data-testid="statusline-model-default"]');
    expect(idle).not.toBeNull();
    expect(idle?.textContent).toContain('next run');

    // It shows the saved harness default rather than an empty control.
    const picker = container.querySelector('[data-testid="mock-provider-model"]') as HTMLElement;
    expect(picker).not.toBeNull();
    expect(picker.getAttribute('data-value')).toBe('zai/glm-5.3');

    // Choosing writes the role, and sends NOTHING to a terminal: there is no
    // session to take a /model command.
    await act(async () => { picker.click(); });
    expect(api.setModelRoute).toHaveBeenCalledWith('harness', 'openai', 'gpt-5');
    expect(invoke).not.toHaveBeenCalledWith('write_to_pty', expect.anything(), expect.anything());
    expect(useCommandCenter.getState().harnessModelRequest).toBeNull();
  });
});
