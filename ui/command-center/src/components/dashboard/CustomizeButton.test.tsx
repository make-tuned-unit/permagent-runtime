/** @vitest-environment jsdom
 *
 * The whole customize system hangs off this one control, so it has to name
 * itself at rest. A hover `title` is not a name: it is only read by someone
 * who already decided the glyph was worth investigating.
 */

import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { CustomizeButton } from './CustomizeButton';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe('Home customize door', () => {
  it('says what it opens without being hovered', () => {
    act(() => { root.render(<CustomizeButton editing={false} onToggle={() => {}} colors={colors} />); });
    const btn = host.querySelector<HTMLButtonElement>('[data-testid="dashboard-customize"]')!;
    expect(btn.textContent).toContain('Customize');
  });

  it('says how to leave once inside', () => {
    act(() => { root.render(<CustomizeButton editing onToggle={() => {}} colors={colors} />); });
    expect(host.querySelector('[data-testid="dashboard-customize"]')!.textContent).toContain('Done');
  });

  it('toggles', () => {
    const onToggle = vi.fn();
    act(() => { root.render(<CustomizeButton editing={false} onToggle={onToggle} colors={colors} />); });
    const btn = host.querySelector<HTMLButtonElement>('[data-testid="dashboard-customize"]')!;
    act(() => { btn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(onToggle).toHaveBeenCalledOnce();
  });
});
