/** @vitest-environment jsdom */
/**
 * The Watcher's insights used to say "One card stalled 14+ days" and stop
 * there — the reader was told something was wrong and given no way to find it.
 * The daemon now names the cards and carries their ids on the insight; these
 * pin the half of that contract the UI owns.
 *
 * The `cards` key is absent on every insight written before the fix, so the
 * back-compat case is not hypothetical — it is most of the existing rows.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const openCardOnBoard = vi.fn();
vi.mock('../../lib/store', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../../lib/store');
  return {
    ...actual,
    useCommandCenter: (sel: (s: Record<string, unknown>) => unknown) =>
      sel({ openCardOnBoard, openGoalDetail: vi.fn(), growProject: vi.fn() }),
    navigateToTool: vi.fn(),
  };
});

import { WatcherInsightsPanel } from './ProjectOverview';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function project(insights: unknown): Project {
  return {
    id: 'p1',
    name: 'GetLadle',
    metadataJson: { watcher_insights: insights },
  } as unknown as Project;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  openCardOnBoard.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('Watcher insights', () => {
  it('renders a clickable chip for each card the insight names', async () => {
    await act(async () =>
      root.render(
        <WatcherInsightsPanel
          project={project([
            {
              text: 'Onboarding copy has sat untouched for three weeks.',
              created_at: '2026-08-08T09:00:00Z',
              cards: [{ id: 'card-1', title: 'Onboarding copy' }],
            },
          ])}
        />,
      ),
    );

    expect(container.textContent).toContain('Onboarding copy has sat untouched');
    const chip = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent === 'Onboarding copy',
    );
    expect(chip, 'the named card must be reachable, not just mentioned').toBeTruthy();

    await act(async () => chip!.click());
    expect(openCardOnBoard).toHaveBeenCalledWith('p1', 'card-1');
  });

  it('still renders insights written before the daemon named cards', async () => {
    // No `cards` key at all — the shape of every historical row.
    await act(async () =>
      root.render(
        <WatcherInsightsPanel
          project={project([
            { text: 'One card stalled 14+ days.', created_at: '2026-08-07T09:00:00Z' },
          ])}
        />,
      ),
    );

    expect(container.textContent).toContain('One card stalled 14+ days.');
    expect(container.querySelectorAll('button')).toHaveLength(0);
  });

  it('treats an empty cards array the same as an absent one', async () => {
    await act(async () =>
      root.render(
        <WatcherInsightsPanel
          project={project([
            { text: 'Steady traffic this week.', created_at: '2026-08-08T09:00:00Z', cards: [] },
          ])}
        />,
      ),
    );
    expect(container.querySelectorAll('button')).toHaveLength(0);
  });

  it('renders nothing at all when there are no insights', async () => {
    await act(async () => root.render(<WatcherInsightsPanel project={project([])} />));
    expect(container.textContent).toBe('');
  });
});
