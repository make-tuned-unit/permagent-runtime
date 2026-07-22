// @vitest-environment jsdom
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CycleTabsButton } from './CycleTabsButton';

let root: Root | null = null;
let container: HTMLDivElement | null = null;

afterEach(() => {
  if (root) act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

describe('CycleTabsButton', () => {
  it('is always rendered as a labeled button and cycles by click', () => {
    const onCycle = vi.fn();
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    act(() => root?.render(<CycleTabsButton pane="terminal" onCycle={onCycle} />));

    const button = container.querySelector('button[aria-label="Cycle terminal tabs"]') as HTMLButtonElement;
    expect(button).not.toBeNull();
    act(() => button.click());
    expect(onCycle).toHaveBeenCalledOnce();
  });
});
