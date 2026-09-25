/** @vitest-environment jsdom
 *
 * R2 (dagK UX standard, 2026-09-25): a broken image src drew the browser's
 * native broken-image glyph with nothing explaining it. ImageMessage now
 * catches the <img> error event and shows a named error state instead.
 */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ImageMessage } from './ImageMessage';

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

describe('ImageMessage error state (R2)', () => {
  it('renders the <img> normally when nothing has gone wrong', () => {
    act(() => { root.render(<ImageMessage src="data:image/png;base64,AA==" />); });
    expect(container.querySelector('img')).toBeTruthy();
    expect(container.textContent).not.toContain('failed to load');
  });

  it('shows a named error state instead of a bare broken-image glyph when the <img> fails', () => {
    act(() => { root.render(<ImageMessage src="data:image/png;base64,not-really-an-image" />); });
    const img = container.querySelector('img') as HTMLImageElement;
    act(() => { img.dispatchEvent(new Event('error')); });

    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toContain('Image failed to load');
    expect(container.querySelector('[role="status"]')).toBeTruthy();
  });
});
