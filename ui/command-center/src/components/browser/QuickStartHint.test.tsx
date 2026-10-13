/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { QuickStartHint } from './QuickStartHint';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const render = (template: string | null, onFocus = vi.fn()) =>
  act(async () => root.render(<QuickStartHint template={template} colors={getThemedColors()} onFocusAddress={onFocus} />));

it('names no vendor and shows a neutral hint when no search template is set', async () => {
  await render(null);
  expect(host.textContent).toContain('set a search template');
  expect(host.textContent).not.toMatch(/google|github/i);
  expect(host.querySelector('button')).toBeNull();
});

it('offers the user-set template, and only that, when one exists', async () => {
  const focus = vi.fn();
  await render('https://search.example/?q=%s', focus);
  const button = host.querySelector('button')!;
  expect(button.textContent).toBe('Search with search.example');
  expect(host.textContent).not.toMatch(/google|github/i);
  act(() => button.click());
  expect(focus).toHaveBeenCalledOnce();
});
