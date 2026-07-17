/**
 * Agent-driven open_item dispatcher — the last-mile wiring that maps a daemon
 * app_open_item event's `kind` to the store seam that already backs the human
 * button (goal → openGoalDetail, grow → growProject). Mirrors the daemon's
 * app_conductor ITEM_CATALOG 1:1; this asserts each mapped kind fires its seam
 * and every un-mapped/incomplete payload is dropped (never guessed).
 *
 * `../lib/api` is mocked (the brainFocus.store pattern) so importing the store +
 * hook touches no network; with no workspaces loaded, the seams' navigateToTool
 * is a no-op, isolating the pending-state wiring. `reason` is omitted so the
 * DOM-only navigation cue never runs under the node test env.
 */

import { describe, expect, it, beforeEach, vi } from 'vitest';

vi.mock('../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { dispatchOpenItem } from './useAppNavigate';
import { useCommandCenter } from '../lib/store';

beforeEach(() => {
  useCommandCenter.setState({ goalDetail: null, openGrowForProject: null, workspaces: [] });
});

describe('dispatchOpenItem — agent last-mile item open', () => {
  it('kind "goal" opens the goal-detail modal with both ids', () => {
    dispatchOpenItem({ kind: 'goal', project_id: 'p1', card_id: 'c1' });
    expect(useCommandCenter.getState().goalDetail).toEqual({ projectId: 'p1', cardId: 'c1' });
  });

  it('kind "grow" preselects the project in the Grow planner', () => {
    dispatchOpenItem({ kind: 'grow', project_id: 'p9' });
    expect(useCommandCenter.getState().openGrowForProject).toBe('p9');
  });

  it('drops a goal missing its card_id (unopenable, never guessed)', () => {
    dispatchOpenItem({ kind: 'goal', project_id: 'p1' });
    expect(useCommandCenter.getState().goalDetail).toBeNull();
  });

  it('drops an unknown kind rather than firing a seam', () => {
    dispatchOpenItem({ kind: 'person', project_id: 'p1', card_id: 'c1' });
    expect(useCommandCenter.getState().goalDetail).toBeNull();
    expect(useCommandCenter.getState().openGrowForProject).toBeNull();
  });

  it('drops a payload with no project_id', () => {
    dispatchOpenItem({ kind: 'grow' });
    expect(useCommandCenter.getState().openGrowForProject).toBeNull();
  });
});
