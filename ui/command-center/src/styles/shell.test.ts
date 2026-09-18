/**
 * The window shell, and the three gates that keep it honest.
 *
 * The shell is the one part of the app whose geometry is shared with something
 * outside the webview. `titleBarStyle: "Overlay"` + `hiddenTitle` make the
 * system titlebar transparent and hand us the whole window, but the traffic
 * lights stay native: they are an AppKit surface compositing above everything
 * the webview draws, at a position declared in `tauri.conf.json` and reapplied
 * from `src-tauri/src/chrome.rs`. So the same four numbers exist in three
 * places, in three languages, and none of them can see the others.
 *
 * That is what these tests are for.
 *
 *   1. The numbers are frozen, with the constraint that produced them.
 *   2. The three copies are compared against each other, by reading the other
 *      two files. If the config drifts, the chrome silently reverts to AppKit's
 *      (9, 9) — silently, because nothing throws — and the CSS keeps reserving
 *      a band the buttons are no longer in.
 *   3. The shell files carry no hardcoded colour. The shell is the surface
 *      every theme has to survive; a literal here is a theme that is wrong
 *      everywhere at once.
 */

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { shell, trafficLightSpan } from './tokens';

const CONF = fileURLToPath(new URL('../../../desktop/src-tauri/tauri.conf.json', import.meta.url));
const CHROME_RS = fileURLToPath(new URL('../../../desktop/src-tauri/src/chrome.rs', import.meta.url));

/** Files that make up the window shell — the layout, not the screens inside it. */
const SHELL_FILES = [
  '../App.tsx',
  '../PaneWindowApp.tsx',
  '../lib/windowChrome.ts',
  '../components/sidebar/Sidebar.tsx',
].map(p => fileURLToPath(new URL(p, import.meta.url)));

/** Strip comments so prose about a colour is not mistaken for a colour. */
function code(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1');
}

describe('shell geometry', () => {
  it('is the set the chrome was designed around', () => {
    expect(shell).toEqual({
      titlebar: 40,
      trafficLights: { x: 12, y: 22, buttonSize: 14, spacing: 23 },
      rail: { open: 208, collapsed: 76 },
    });
  });

  it('centres the window buttons in the titlebar band', () => {
    // `y` is not a top inset: AppKit sizes the titlebar container
    // `buttonSize + y` tall, pinned to the top edge, and the button keeps its
    // own 9pt origin inside it — so the visible inset is `y - 9` (measured by
    // the A1a spike, reproduced in chrome.rs's unit tests).
    const inset = shell.trafficLights.y - 9;
    expect(inset).toBe(13);
    expect(inset + shell.trafficLights.buttonSize + inset).toBe(shell.titlebar);
  });

  it('keeps the window buttons inside the collapsed rail', () => {
    // THE constraint that fixed x at 12. The rail is full-height now, so the
    // traffic lights sit inside it in both states; a rail narrower than their
    // span hangs the zoom button over the rail's edge onto the content pane.
    expect(trafficLightSpan()).toBe(72);
    expect(shell.rail.collapsed).toBeGreaterThanOrEqual(trafficLightSpan());
    expect(shell.rail.open).toBeGreaterThanOrEqual(trafficLightSpan());
    // What it rules out: the research doc's provisional x = 20.
    expect(trafficLightSpan(20)).toBeGreaterThan(shell.rail.collapsed);
  });
});

describe('shell geometry agrees with the native window', () => {
  const conf = JSON.parse(readFileSync(CONF, 'utf8'));
  const main = conf.app.windows.find((w: { label: string }) => w.label === 'main');

  it('declares the overlay titlebar in the config, which is the only path that works', () => {
    // Under the `unstable` cargo feature — which we cannot drop, the in-app
    // browser is built on `Window::add_child` — the Rust builder silently
    // discards `trafficLightPosition`. Config is the only path that applies it.
    expect(main.titleBarStyle).toBe('Overlay');
    expect(main.hiddenTitle).toBe(true);
    // `decorations: false` would forfeit the free macOS corner radius and the
    // native drop shadow, which is the whole reason to stay decorated.
    expect(main.decorations).toBe(true);
    // No transparency: measured at ~+6 points of whole-GPU utilisation at idle.
    expect(main.transparent ?? false).toBe(false);
  });

  it('puts the traffic lights where the CSS reserves room for them', () => {
    expect(main.trafficLightPosition).toEqual({
      x: shell.trafficLights.x,
      y: shell.trafficLights.y,
    });
  });

  it('agrees with the Rust constants that reapply the inset', () => {
    const rust = readFileSync(CHROME_RS, 'utf8');
    const constant = (name: string) => {
      const m = new RegExp(`pub const ${name}: f64 = ([0-9.]+);`).exec(rust);
      if (!m) throw new Error(`chrome.rs no longer declares ${name}`);
      return Number(m[1]);
    };
    expect(constant('TRAFFIC_LIGHT_X')).toBe(shell.trafficLights.x);
    expect(constant('TRAFFIC_LIGHT_Y')).toBe(shell.trafficLights.y);
    expect(constant('BUTTON_SIZE')).toBe(shell.trafficLights.buttonSize);
    expect(constant('BUTTON_SPACING')).toBe(shell.trafficLights.spacing);
  });
});

describe('the shell has no colour of its own', () => {
  it('carries no hardcoded hex, rgb() or hsl()', () => {
    const offenders: string[] = [];
    for (const file of SHELL_FILES) {
      code(readFileSync(file, 'utf8')).split('\n').forEach((line, i) => {
        if (/#[0-9a-fA-F]{3,8}\b/.test(line) || /\b(rgba?|hsla?)\(/.test(line)) {
          offenders.push(`${file.split('/src/')[1]}:${i + 1}  ${line.trim()}`);
        }
      });
    }
    // Everything the shell paints comes from `useTheme()` — `gradient.shell`,
    // `gradient.sidebar`, `colors.border`. It has to: the shell is the one
    // surface all three themes and both appearance modes share.
    expect(offenders, 'the shell paints from theme tokens only').toEqual([]);
  });
});
