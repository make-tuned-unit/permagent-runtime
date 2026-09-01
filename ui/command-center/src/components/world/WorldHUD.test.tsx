/**
 * @vitest-environment jsdom
 *
 * The World's key, and the one thing it must not do: wait.
 *
 * "WASD to walk · ESC to exit" was the hall's only interaction hint, and it
 * appeared *only once the camera had already switched to walking mode* — after
 * a curious click on an avatar had silently changed the control scheme. A hint
 * that arrives after the surprise is worse than none, because it implies a key
 * exists somewhere to be found.
 *
 * So these tests hold the walking keys on screen in ORBIT — the mode every
 * user starts in — and hold the honesty line the World's ambience needs: the
 * rain and the river are the Brain, the marble is not, and the avatars that
 * nothing reports are named as ambience rather than left to look like work.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { WorldHUD } from './WorldHUD';
import { ROSTER } from './agents/roster';

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

function render(mode: 'orbit' | 'third-person' = 'orbit') {
  act(() => root.render(
    <WorldHUD mode={mode} showFps={false} hoveredStation={null} stationTooltip={null} />,
  ));
}

const legend = () => container.querySelector('[data-testid="canvas-legend"]');

describe('World key', () => {
  it('teaches the walking keys BEFORE the camera switches, not after', () => {
    render('orbit');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('WASD');
    expect(text).toContain('Esc');
    // And says what makes the switch happen, so the change of control scheme
    // is something the user chose rather than something that happened to them.
    expect(text).toContain('Click an agent');
  });

  it('teaches the gestures orbit mode actually has', () => {
    render('orbit');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('Drag');
    expect(text).toContain('Scroll');
    expect(text).toContain('arrow keys');
    expect(text).toContain('Click a station');
  });

  it('separates the Brain-driven ambience from the set dressing', () => {
    render('orbit');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('Rain');
    expect(text).toContain('recall');
    expect(text).toContain('set dressing');
  });

  it('names the avatars nothing reports, from the roster itself', () => {
    render('orbit');
    const text = legend()!.textContent ?? '';
    const ambient = ROSTER.filter(a => a.wire === 'sim');
    expect(ambient.length).toBeGreaterThan(0);
    for (const agent of ambient) expect(text).toContain(agent.name);
    expect(text).toContain('ambience, not a claim');
  });

  it('stops offering orbit gestures once they are dead', () => {
    // Walking mode unmounts <OrbitControls> — drag and scroll do nothing there,
    // so the key must not keep offering them.
    render('third-person');
    const text = legend()!.textContent ?? '';
    expect(text).not.toContain('Drag');
    expect(text).not.toContain('Scroll');
    expect(text).toContain('WASD');
    expect(text).toContain('Esc');
  });

  it('keeps the camera-mode badge it always had', () => {
    render('orbit');
    expect(container.textContent).toContain('ORBIT');
    render('third-person');
    expect(container.textContent).toContain('WALKING');
  });

  it('goes quiet for good once dismissed', () => {
    render('orbit');
    const dismiss = container.querySelector<HTMLButtonElement>('[data-testid="canvas-legend-dismiss"]')!;
    act(() => dismiss.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(legend()).toBeNull();
    act(() => root.unmount());
    root = createRoot(container);
    render('orbit');
    expect(legend()).toBeNull();
    expect(container.querySelector('[data-testid="canvas-legend-open"]')).not.toBeNull();
  });
});
