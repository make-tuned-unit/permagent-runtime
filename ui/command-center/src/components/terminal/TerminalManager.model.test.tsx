/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

const { invoke, terminalProps } = vi.hoisted(() => ({
  invoke: vi.fn(() => Promise.resolve(undefined)),
  terminalProps: { current: null as null | { onPtyData?: (data: string) => void } },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('./Terminal', () => ({ Terminal: (props: { onPtyData?: (data: string) => void }) => { terminalProps.current = props; return <div />; } }));
vi.mock('../../lib/native-drag-drop', () => ({ registerDropZone: () => () => {} }));
vi.mock('../settings/ProviderModelPicker', () => ({
  ProviderModelPicker: ({ onChange, value }: { onChange: (value: { provider: string; model: string }) => void; value: { provider: string | null; model: string | null } }) => (
    <button type="button" data-testid="mock-provider-model" data-value={`${value.provider ?? ''}/${value.model ?? ''}`} onClick={() => onChange({ provider: 'openai', model: 'gpt-5' })}>pick</button>
  ),
}));

import { TerminalManager, __resetTerminalPersistenceForTests, isPermagentInteractiveHarness } from './TerminalManager';
import { textSize } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  __resetTerminalPersistenceForTests();
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
});

describe('TerminalManager harness model control', () => {
  it('recognizes bundled absolute commands and equivalent recipe flag syntax', () => {
    expect(isPermagentInteractiveHarness({
      id: 'bundled',
      label: 'Build',
      sessionId: 'pty-bundled',
      initialCommand: '/Applications/Permagent.app/Contents/Resources/bin/permagent run --recipe=permagent-coding --interactive',
    })).toBe(true);
  });

  it('sends the provider/model command to the active harness PTY', async () => {
    act(() => root.render(<TerminalManager initialTab={{ id: 'harness', label: 'Build', sessionId: 'pty-1', initialCommand: 'permagent run --recipe permagent-coding --interactive' }} />));
    expect(container.querySelector('[data-testid="terminal-model-control"]')).not.toBeNull();

    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="mock-provider-model"]')!.click();
      await vi.waitFor(() => {
        expect(invoke).toHaveBeenCalledWith(
          'write_to_pty',
          { sessionId: 'pty-1', data: '/model openai/gpt-5\r' },
          undefined,
        );
      });
    });
    expect(container.querySelector('[data-testid="mock-provider-model"]')?.getAttribute('data-value')).toBe('/');
    expect(container.textContent).toContain('requested openai/gpt-5');
    expect(container.querySelector<HTMLElement>('[aria-live="polite"]')?.style.fontSize).toBe(`${textSize.micro}px`);

    await act(async () => {
      terminalProps.current?.onPtyData?.('\u001b[32mHarness model switched for this session only: openai/gpt-5. Chat settings were not changed.\u001b[0m\r\n');
    });
    expect(container.querySelector('[data-testid="mock-provider-model"]')?.getAttribute('data-value')).toBe('openai/gpt-5');
    expect(container.textContent).not.toContain('requested openai/gpt-5');
  });

  it('does not show the control for provider-specific terminal processes', () => {
    act(() => root.render(<TerminalManager initialTab={{ id: 'raw', label: 'Claude', sessionId: 'pty-2', initialCommand: 'claude' }} />));
    expect(container.querySelector('[data-testid="terminal-model-control"]')).toBeNull();
  });
});
