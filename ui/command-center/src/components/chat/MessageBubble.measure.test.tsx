/** @vitest-environment jsdom
 *
 * C3 (dagK walk, 2026-09-24): with the dock now able to reach 960px, a bubble
 * capped only at 85% of the transcript ran prose ~140 characters a line. Each
 * bubble keeps a readable measure regardless of how wide the dock is dragged.
 * jsdom has no layout, so this pins the declared ceiling; the harness page is
 * the visual proof.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/store', () => {
  const state = { agentName: 'Henry' };
  return {
    useCommandCenter: Object.assign(
      (selector: (v: typeof state) => unknown) => selector(state),
      { getState: () => state },
    ),
  };
});
vi.mock('./MessageRenderer', () => ({ MessageRenderer: () => null }));
vi.mock('../awareness/CitationMarker', () => ({ CitationMarker: () => null }));

import { BUBBLE_MAX_WIDTH, MessageBubble } from './MessageBubble';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('bubble reading measure (C3)', () => {
  it('caps at 72ch as well as 85% of the transcript', () => {
    expect(BUBBLE_MAX_WIDTH).toBe('min(85%, 72ch)');
  });

  it.each(['user', 'assistant', 'system'] as const)('applies the measure to a %s bubble', (role) => {
    act(() => {
      root.render(<MessageBubble message={{ id: 'm', role, content: 'x'.repeat(400), timestamp: '2026-09-24T15:21:00Z' }} />);
    });
    const bubble = container.querySelector('.rounded-xl') as HTMLElement;
    // Declared in the inline style, not as a Tailwind class, so the one value
    // is shared with the greeting bubble and cannot drift from it.
    expect(bubble.getAttribute('style')).toContain('max-width: min(85%, 72ch)');
    expect(bubble.className).not.toContain('max-w-');
  });
});
