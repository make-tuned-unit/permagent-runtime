/** @vitest-environment jsdom */
import { createElement } from 'react';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useDecisionQueue } from './useDecisionQueue';
import type { Decision } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
let queue: ReturnType<typeof useDecisionQueue>;
const decision = (id: string, tier = 2, created_at = '2026-01-01') => ({ id, tier, created_at } as Decision);
function Harness({ decisions }: { decisions: Decision[] }) { queue = useDecisionQueue({ decisions }); return null; }
function render(decisions: Decision[]) { act(() => root.render(createElement(Harness, { decisions }))); }
beforeEach(() => { container = document.createElement('div'); root = createRoot(container); });
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('D1 queue — rulings 4 and 5', () => {
  it('all pending decisions are reachable beyond the first ten', () => {
    render(Array.from({ length: 23 }, (_, i) => decision(String(i).padStart(2, '0'))));
    const visited = new Set<string>();
    for (let i = 0; i < 23; i++) { visited.add(queue.focusedId!); act(() => queue.next()); }
    expect(visited.size).toBe(23);
    expect(queue.position).toEqual({ index: 22, total: 23 });
    act(() => queue.next());
    expect(queue.position?.index).toBe(22);
  });
  it('focus survives refetch, new arrivals and reordering', () => {
    render([decision('b'), decision('a'), decision('low', 1)]);
    expect(queue.ordered.map(d => d.id)).toEqual(['a', 'b', 'low']);
    act(() => queue.focus('b'));
    render([decision('new', 3), decision('low', 1), decision('b', 2, '2025-01-01'), decision('a')]);
    expect(queue.focusedId).toBe('b');
    expect(queue.position).toEqual({ index: 1, total: 4 });
  });
  it('external resolution updates position and advances deterministically', () => {
    render(['a', 'b', 'c', 'd'].map(id => decision(id)));
    act(() => queue.focus('b'));
    render(['b', 'c', 'd'].map(id => decision(id)));
    expect(queue.focusedId).toBe('b');
    expect(queue.position?.index).toBe(0);
    render([decision('d'), decision('c', 3)]);
    expect(queue.focusedId).toBe('c');
    act(() => queue.focus('d'));
    render([decision('c', 3)]);
    expect(queue.focusedId).toBe('c');
    render([]);
    expect(queue.focusedId).toBeNull();
    expect(queue.position).toBeNull();
  });
  it('Later defers locally without answering or hiding pending work', () => {
    const rows = ['a', 'b', 'c'].map(id => decision(id));
    render(rows);
    act(() => queue.later());
    expect(queue.ordered.map(d => d.id)).toEqual(['b', 'c', 'a']);
    expect(queue.focusedId).toBe('b');
    expect(queue.deferred).toEqual(new Set(['a']));
    act(() => queue.next()); act(() => queue.next());
    expect(queue.focusedId).toBe('a');
    expect(rows.map(d => d.id)).toEqual(['a', 'b', 'c']);
  });
});
