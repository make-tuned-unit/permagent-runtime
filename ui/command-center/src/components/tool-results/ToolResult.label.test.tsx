/** @vitest-environment jsdom
 *
 * R3 (dagK UX standard, 2026-09-25): the tool card badge showed the wire
 * identifier (`card_create`, `gmail__search`) verbatim. It now shows
 * toolLabel(call.name) as the headline, with the raw name still available
 * on hover (Tooltip) and as a line in the expanded detail.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ToolResult } from './ToolResult';
import type { ToolCall } from '../../lib/store';

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

function render(call: ToolCall) {
  act(() => { root.render(<ToolResult call={call} />); });
}

describe('ToolResult label (R3)', () => {
  it('shows a plain-English label instead of the raw identifier for an overridden tool', () => {
    render({ name: 'card_create', arguments: { title: 'x' }, result: 'card 1 created', success: true });
    expect(container.textContent).toContain('Create a card');
    expect(container.textContent).not.toContain('card_create');
  });

  it('keeps the raw name reachable in the expanded detail even though the badge is renamed', () => {
    render({ name: 'gmail__search', arguments: {}, result: '[]', success: true });
    expect(container.textContent).toContain('Search Gmail');
    expect(container.textContent).not.toContain('gmail__search');

    const toggle = container.querySelector('button') as HTMLButtonElement;
    act(() => { toggle.click(); });
    expect(container.textContent).toContain('gmail__search');
  });

  it('falls back to the mechanical label for a tool with no override', () => {
    render({ name: 'widget_status', arguments: {}, result: undefined, success: undefined });
    expect(container.textContent).toContain('Widget status');
  });
});
