/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

import { StateBlock } from './StateBlock';
import { getThemedColors } from '../../styles/tokens';

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

function render(block: React.ReactElement) {
  act(() => root.render(block));
  return container.firstElementChild as HTMLElement;
}

function asRendered(color: string): string {
  const probe = document.createElement('span');
  probe.style.color = color;
  return probe.style.color;
}

describe('StateBlock', () => {
  it('uses the readable muted semantic for explanatory detail copy', () => {
    const root_ = render(
      <StateBlock tone="empty" title="No projects yet" detail="Create a project to see it here." />,
    );
    const detail = root_.querySelector('div > div:nth-child(2)') as HTMLElement;

    expect(detail.textContent).toContain('Create a project');
    expect(detail.style.color).toBe(asRendered(getThemedColors().textMuted));
    expect(detail.style.color).not.toBe(asRendered(getThemedColors().textDim));
  });

  it('keeps error emphasis and retry action semantics unchanged', () => {
    const retry = vi.fn();
    const root_ = render(
      <StateBlock tone="error" title="Could not load projects" detail="The daemon is unavailable." onRetry={retry} />,
    );

    expect(root_.textContent).toContain('Could not load projects');
    expect(root_.textContent).toContain('Try again');
    const button = root_.querySelector('button') as HTMLButtonElement;
    expect(button.textContent).toBe('Try again');
    act(() => button.click());
    expect(retry).toHaveBeenCalledTimes(1);
  });
});
