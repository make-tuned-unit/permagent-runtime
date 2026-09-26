/**
 * @vitest-environment jsdom
 *
 * BrainList's memory row: source Chip (R5 "a memory shows its source") and
 * the Forget → undo-toast flow (R5 "explicit forget with undo", R4 "offer
 * undo instead of a confirm"). Verifies the row's DELETE never fires on the
 * click itself, fires once the undo window lapses, and restores the row on
 * Undo or on a failed request — the exact contract `brainForget.ts` promises.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('../../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/api')>();
  return {
    ...actual,
    api: {
      ...actual.api,
      getBrainMemories: vi.fn(),
      forgetBrainMemory: vi.fn(),
    },
  };
});

vi.mock('../../lib/notifications', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/notifications')>();
  return { ...actual, toast: vi.fn() };
});

import { BrainList } from './BrainList';
import { api } from '../../lib/api';
import { toast } from '../../lib/notifications';

const getBrainMemoriesMock = vi.mocked(api.getBrainMemories);
const forgetBrainMemoryMock = vi.mocked(api.forgetBrainMemory);
const toastMock = vi.mocked(toast);

const KNOWN_MEMORY = {
  id: 'mem-1',
  key: 'permagent.card:card-1:body',
  text: 'ship the memory source lane by friday',
  description: 'Ship the memory source lane',
  ent: [],
  age: 0.1,
  weight: 0.7,
  timestamp: '2026-09-20T10:00:00+00:00',
  source: 'permagent.card',
  last_reinforced_at: '2026-09-24T10:00:00+00:00',
};

const UNKNOWN_SOURCE_MEMORY = {
  id: 'mem-2',
  key: 'legacy:note-2',
  text: 'an old memory from before source tracking existed',
  description: null,
  ent: [],
  age: 0.9,
  weight: 0.3,
  timestamp: '2025-01-01T00:00:00+00:00',
  source: null,
  last_reinforced_at: null,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  toastMock.mockReset();
  getBrainMemoriesMock.mockReset();
  forgetBrainMemoryMock.mockReset();
  getBrainMemoriesMock.mockResolvedValue({
    memories: [KNOWN_MEMORY, UNKNOWN_SOURCE_MEMORY],
    total: 2,
    has_more: false,
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function renderList() {
  await act(async () => {
    root.render(
      <BrainList
        onSelect={() => {}}
        selectedId={null}
        timeValue={1}
      />,
    );
  });
  // Let the initial getBrainMemories() promise resolve and its state land.
  await act(async () => { await Promise.resolve(); });
}

describe('BrainList memory row — source chip', () => {
  it('shows a humanized source phrase for a known tag, and "source unknown" for a legacy row', async () => {
    await renderList();
    expect(container.textContent).toContain('from a task');
    expect(container.textContent).toContain('source unknown');
  });
});

describe('BrainList memory row — Forget / undo', () => {
  it('does not call the DELETE endpoint on the Forget click itself', async () => {
    await renderList();
    const forgetButtons = Array.from(container.querySelectorAll('button'))
      .filter(b => b.textContent === 'Forget') as HTMLButtonElement[];
    expect(forgetButtons.length).toBeGreaterThan(0);

    await act(async () => { forgetButtons[0].click(); });

    expect(forgetBrainMemoryMock).not.toHaveBeenCalled();
    expect(toastMock).toHaveBeenCalledWith(
      'Memory forgotten',
      expect.any(String),
      undefined,
      expect.objectContaining({ action: expect.objectContaining({ label: 'Undo' }) }),
    );
  });

  it('Undo restores the row and never sends a request, even after the window lapses', async () => {
    await renderList();
    const forgetButtons = Array.from(container.querySelectorAll('button'))
      .filter(b => b.textContent === 'Forget') as HTMLButtonElement[];
    await act(async () => { forgetButtons[0].click(); });

    // Row is optimistically hidden.
    expect(container.textContent).not.toContain('ship the memory source lane by friday');

    const [, , , opts] = toastMock.mock.calls[0];
    await act(async () => { opts!.action!.onAction(); });

    // Undo brought it back.
    expect(container.textContent).toContain('ship the memory source lane by friday');

    // And the scheduled delete never fires, even once the window is long past.
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); });
    expect(forgetBrainMemoryMock).not.toHaveBeenCalled();
  });

  it('sends the DELETE once the undo window expires unactioned', async () => {
    forgetBrainMemoryMock.mockResolvedValue({ existed: true, verified: true });
    await renderList();
    const forgetButtons = Array.from(container.querySelectorAll('button'))
      .filter(b => b.textContent === 'Forget') as HTMLButtonElement[];
    await act(async () => { forgetButtons[0].click(); });

    await act(async () => { await vi.advanceTimersByTimeAsync(8_000); });

    expect(forgetBrainMemoryMock).toHaveBeenCalledWith('permagent.card:card-1:body');
    // Still hidden — a completed delete does not resurrect the row.
    expect(container.textContent).not.toContain('ship the memory source lane by friday');
  });

  it('restores the row and shows a failure toast when the request itself fails', async () => {
    forgetBrainMemoryMock.mockRejectedValue(new Error('daemon unreachable'));
    await renderList();
    const forgetButtons = Array.from(container.querySelectorAll('button'))
      .filter(b => b.textContent === 'Forget') as HTMLButtonElement[];
    await act(async () => { forgetButtons[0].click(); });

    await act(async () => { await vi.advanceTimersByTimeAsync(8_000); });

    expect(container.textContent).toContain('ship the memory source lane by friday');
    expect(toastMock).toHaveBeenCalledWith(
      'Could not forget that memory',
      expect.any(String),
      undefined,
      expect.objectContaining({ tone: 'danger' }),
    );
  });
});
