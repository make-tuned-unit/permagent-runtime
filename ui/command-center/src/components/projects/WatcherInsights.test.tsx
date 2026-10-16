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
    name: 'Beacon',
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

  it('truncates a long card title at its right end instead of bleeding past the panel', async () => {
    // Real titles from the app, 2026-09-18, rendered clipped mid-word at BOTH
    // ends ("e email template and curate first 8 weeks of events (W"): the
    // chip had a fixed 260px cap wider than a narrow panel, and `.pa-btn`
    // centres its content, so the overflow spilled equally left and right.
    // jsdom cannot measure the clip; this pins the invariants that prevent it.
    const title = 'Design the welcome email template and curate first 8 weeks of events (Wave 1)';
    await act(async () =>
      root.render(
        <WatcherInsightsPanel
          project={project([
            {
              text: 'Two launch cards have not moved.',
              created_at: '2026-09-18T09:00:00Z',
              cards: [{ id: 'card-9', title }],
            },
          ])}
        />,
      ),
    );
    const chip = container.querySelector('[data-testid="watcher-card-chip"]') as HTMLElement;
    expect(chip, 'the chip must render').toBeTruthy();
    // Capped at its row, whatever the row's width — never a fixed pixel cap.
    expect(chip.style.maxWidth).toBe('100%');
    expect(chip.style.minWidth).toBe('0');
    // Content starts at the left edge, so any overflow is on the right only.
    expect(chip.style.justifyContent).toBe('flex-start');
    // The title is an element that can take an ellipsis, not a bare text node.
    const label = chip.querySelector('[data-testid="watcher-card-chip-title"]') as HTMLElement;
    expect(label.textContent).toBe(title);
    expect(label.style.textOverflow).toBe('ellipsis');
    expect(label.style.overflow).toBe('hidden');
    expect(label.style.whiteSpace).toBe('nowrap');
    expect(label.style.minWidth).toBe('0');
    // The full title survives for assistive tech even when the eye sees "…".
    expect(chip.getAttribute('aria-label')).toBe(`Open "${title}" on the board`);
  });

  it('renders nothing at all when there are no insights', async () => {
    await act(async () => root.render(<WatcherInsightsPanel project={project([])} />));
    expect(container.textContent).toBe('');
  });
});


it('reports a failed latest check without implying the project is clear', async () => {
  const p = project([]);
  p.metadataJson.watcher_check = { status: 'failed', checked_at: '2026-10-15T12:00:00Z' };
  await act(async () => root.render(<WatcherInsightsPanel project={p} />));
  expect(container.textContent).toContain("couldn't complete its latest check");
  expect(container.textContent).not.toContain('all clear');
});

it('folds duplicate stable evidence identities even when no card is named', async () => {
  await act(async () => root.render(<WatcherInsightsPanel project={project([
    { id: 'same-evidence', text: 'New wording', created_at: '2026-10-15' },
    { id: 'same-evidence', text: 'Repeated wording', created_at: '2026-10-14' },
  ])} />));
  expect(container.textContent).toContain('New wording');
  expect(container.textContent).not.toContain('Repeated wording');
});
