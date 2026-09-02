/**
 * @vitest-environment jsdom
 *
 * `CanvasLegend` is the one key the three canvases share.
 *
 * A 3D surface has no chrome to read: nothing about it says that dragging
 * turns it, that a dimmed face means something, or which glowing thing is a
 * real signal. Every canvas in the app had solved that differently, which is
 * to say none of them had. These tests hold the shape of the shared answer:
 * it teaches itself on a first visit, it goes away for good when told to, it
 * can be told to without a mouse, and it comes back on request.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { CanvasLegend } from './CanvasLegend';
import { canvasLegendStorageKey } from './legendMemory';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

const gestures = [{ term: 'Drag', meaning: 'turns the hall around you' }];
const vocabulary = [{ term: 'Rain', meaning: 'a memory being saved' }];

function render(canvasId = 'test-canvas') {
  act(() => root.render(
    <CanvasLegend canvasId={canvasId} gestures={gestures} vocabulary={vocabulary} />,
  ));
}

const panel = () => container.querySelector('[data-testid="canvas-legend"]');
const dismiss = () => container.querySelector<HTMLButtonElement>('[data-testid="canvas-legend-dismiss"]');
const reopen = () => container.querySelector<HTMLButtonElement>('[data-testid="canvas-legend-open"]');

describe('CanvasLegend', () => {
  it('teaches itself on a first visit', () => {
    render();
    expect(panel()).not.toBeNull();
    expect(panel()!.textContent).toContain('turns the hall around you');
    expect(panel()!.textContent).toContain('a memory being saved');
  });

  it('goes quiet once dismissed, and remembers it next time', () => {
    render();
    act(() => { dismiss()!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(panel()).toBeNull();
    expect(localStorage.getItem(canvasLegendStorageKey('test-canvas'))).toBe('dismissed');

    // A second visit, with the memory already written.
    act(() => root.unmount());
    root = createRoot(container);
    render();
    expect(panel()).toBeNull();
  });

  it('leaves a way back that says what it is', () => {
    render();
    act(() => { dismiss()!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    const back = reopen();
    expect(back).not.toBeNull();
    expect(back!.textContent).toContain('Key');
    expect(back!.getAttribute('aria-expanded')).toBe('false');

    act(() => { back!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(panel()).not.toBeNull();
    expect(localStorage.getItem(canvasLegendStorageKey('test-canvas'))).toBe('open');
  });

  it('can be dismissed from the keyboard', () => {
    render();
    act(() => {
      dismiss()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(panel()).toBeNull();
    expect(localStorage.getItem(canvasLegendStorageKey('test-canvas'))).toBe('dismissed');
  });

  it('keeps one canvas’s answer out of another’s', () => {
    render('world');
    act(() => { dismiss()!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    act(() => root.unmount());
    root = createRoot(container);
    render('brain-graph');
    expect(panel()).not.toBeNull();
  });

  it('names itself for a screen reader without shouting on screen', () => {
    render();
    expect(panel()!.getAttribute('aria-label')).toContain('Key');
    expect(dismiss()!.getAttribute('aria-label')).toContain('Dismiss');
  });
});
