/** @vitest-environment jsdom
 *
 * A code block that is still streaming in used to be re-tokenised by shiki on
 * every delta. Measured: highlighting a 60-line TypeScript block line by line
 * costs 340ms of main-thread time, and the final pass alone 10.6ms — most of a
 * 16ms frame, taken from the paint that shows the text.
 *
 * The two things that must both hold: the highlighter runs once for a settled
 * block instead of once per delta, AND the code on screen is never a stale
 * snapshot while it waits.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const codeToHtml = vi.hoisted(() => vi.fn());
vi.mock('shiki', () => ({ codeToHtml }));
vi.mock('../../styles/useTheme', () => ({
  useTheme: () => ({ theme: 'dark', colors: { codeBg: '#000', border: '#111', surface: '#222', textMuted: '#333', text: '#444', success: '#0f0', danger: '#f00' } }),
}));

import { CodeBlock } from './CodeBlock';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  codeToHtml.mockReset();
  codeToHtml.mockImplementation((code: string) => Promise.resolve(`<pre data-hl>${code}</pre>`));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function render(code: string) {
  await act(async () => { root.render(<CodeBlock language="ts" code={code} />); });
}

describe('highlighting a block that is still arriving', () => {
  it('highlights once when the block settles, not once per delta', async () => {
    await render('line 1\n');
    expect(codeToHtml).toHaveBeenCalledTimes(1); // first paint is immediate

    for (let i = 2; i <= 20; i++) await render(`line ${i}\n`.repeat(i));
    expect(codeToHtml).toHaveBeenCalledTimes(1); // 19 deltas, zero extra passes

    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(codeToHtml).toHaveBeenCalledTimes(2);
  });

  it('shows the newest code on every delta rather than a stale colourised snapshot', async () => {
    await render('const a = 1;');
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(container.querySelector('[data-hl]')).not.toBeNull();

    await render('const a = 1;\nconst b = 2;');
    // Colour is behind, so the plain <pre> takes over — and it has the new line.
    expect(container.querySelector('[data-hl]')).toBeNull();
    expect(container.textContent).toContain('const b = 2;');

    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    expect(container.querySelector('[data-hl]')?.textContent).toBe('const a = 1;\nconst b = 2;');
  });

  it('paints a block that arrives complete without waiting on the settle timer', async () => {
    await render('already done');
    await act(async () => { await Promise.resolve(); });
    expect(container.querySelector('[data-hl]')?.textContent).toBe('already done');
  });
});
