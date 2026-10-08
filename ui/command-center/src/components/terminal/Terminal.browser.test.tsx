/** @vitest-environment jsdom */
import { act, StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const { invoke, listen, xterm } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), xterm: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));
vi.mock('@xterm/xterm', () => ({ Terminal: xterm }));
import { Terminal } from './Terminal';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it('explains the desktop requirement without initializing xterm, invoking PTY commands or sending a pending launch', async () => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  const host = document.createElement('div');
  document.body.append(host);
  const root = createRoot(host);
  const spawned = vi.fn();
  try {
    await act(async () => root.render(<StrictMode><Terminal sessionId={null} initialCommand="fixture-command" followUpInput="fixture prompt" onSessionSpawned={spawned} isVisible /></StrictMode>));
    expect(host.querySelector('[role="status"]')?.textContent).toContain('Desktop App Required');
    expect(host.textContent).toContain('Open Build in the desktop app');
    expect(host.textContent).not.toContain('Failed to spawn');
    expect(host.querySelector('.pty-terminal')).toBeNull();
    expect(xterm).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
    expect(listen).not.toHaveBeenCalled();
    expect(spawned).not.toHaveBeenCalled();
  } finally {
    act(() => root.unmount());
    host.remove();
  }
});
