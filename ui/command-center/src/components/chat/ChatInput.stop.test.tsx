/**
 * @vitest-environment jsdom
 *
 * Stop is the highest-traffic control in the app. When the cancel POST itself
 * fails, the button used to silently re-arm — identical to never having been
 * pressed — while the agent kept talking.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const mocks = vi.hoisted(() => ({
  sendMessage: vi.fn(),
  stopStreaming: vi.fn(),
}));

vi.mock('../../lib/store', () => {
  const state = {
    isStreaming: true,
    sendMessage: mocks.sendMessage,
    stopStreaming: mocks.stopStreaming,
  };
  const useCommandCenter = Object.assign(
    (selector: (value: typeof state) => unknown) => selector(state),
    { getState: () => state },
  );
  return { useCommandCenter };
});
vi.mock('../voice/VoiceButton', () => ({ VoiceButton: () => null }));

import { ChatInput } from './ChatInput';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

function stopButton(): HTMLButtonElement {
  return container.querySelector('[aria-label="Stop generating"]') as HTMLButtonElement;
}

beforeEach(() => {
  mocks.stopStreaming.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it('says so on screen when the cancel request itself fails', async () => {
  mocks.stopStreaming.mockRejectedValue(new Error('network down'));
  await act(async () => { root.render(<ChatInput />); });

  await act(async () => { stopButton().click(); });
  await settle();

  const alert = container.querySelector('[role="alert"]');
  expect(alert?.textContent).toMatch(/Couldn't stop the reply/i);
  // Still pressable, so the user can try again.
  expect(stopButton().disabled).toBe(false);
});

it('clears the failure notice when Stop is pressed again', async () => {
  mocks.stopStreaming.mockRejectedValueOnce(new Error('network down'));
  await act(async () => { root.render(<ChatInput />); });
  await act(async () => { stopButton().click(); });
  await settle();
  expect(container.querySelector('[role="alert"]')).toBeTruthy();

  mocks.stopStreaming.mockResolvedValue(true);
  await act(async () => { stopButton().click(); });
  await settle();
  expect(container.querySelector('[role="alert"]')).toBeNull();
});
