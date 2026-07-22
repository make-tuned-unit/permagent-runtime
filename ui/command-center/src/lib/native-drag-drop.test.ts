/**
 * Regression tests for position-scoped native drop routing (#550).
 *
 * The Build tab has a terminal pane and a browser pane inside the same window,
 * but Tauri delivers file drops as ONE window-level stream. The old design
 * routed every drop to a single global handler (the chat), so a file dropped on
 * the terminal pane could never reach it. These tests pin the fix: drops are
 * routed by position to the highest-priority zone whose bounds contain the drop
 * point, and a window-wide fallback (the chat) only wins where no pane claims.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  pickDropZone,
  registerDropZone,
  handleDragDropPayload,
  setDropHandlers,
  __resetDropZonesForTest,
  type FileDropZone,
} from './native-drag-drop';

afterEach(() => {
  __resetDropZonesForTest();
  vi.unstubAllGlobals();
});

/** A zone whose element reports a fixed CSS-pixel rectangle. */
function zoneAt(
  id: string,
  rect: { left: number; top: number; right: number; bottom: number },
  extra: Partial<FileDropZone> = {},
): FileDropZone {
  const el = {
    getBoundingClientRect: () => ({
      left: rect.left,
      top: rect.top,
      right: rect.right,
      bottom: rect.bottom,
      width: rect.right - rect.left,
      height: rect.bottom - rect.top,
    }),
  } as unknown as HTMLElement;
  return {
    id,
    getElement: () => el,
    onDrop: () => {},
    ...extra,
  };
}

describe('pickDropZone', () => {
  const terminal = zoneAt('terminal', { left: 0, top: 0, right: 100, bottom: 100 }, { priority: 10 });
  const chat: FileDropZone = { id: 'chat', getElement: () => null, priority: 0, onDrop: () => {} };

  it('routes a drop inside the terminal bounds to the terminal, not the window-wide chat', () => {
    expect(pickDropZone([chat, terminal], { x: 50, y: 50 }, 1)?.id).toBe('terminal');
  });

  it('falls back to the window-wide chat zone outside every pane', () => {
    expect(pickDropZone([chat, terminal], { x: 500, y: 500 }, 1)?.id).toBe('chat');
  });

  it('converts physical drop coordinates to CSS pixels via the device pixel ratio', () => {
    // Physical (180,180) on a 2x display is CSS (90,90) — inside the terminal.
    expect(pickDropZone([chat, terminal], { x: 180, y: 180 }, 2)?.id).toBe('terminal');
    // Physical (220,220) on 2x is CSS (110,110) — outside the terminal.
    expect(pickDropZone([chat, terminal], { x: 220, y: 220 }, 2)?.id).toBe('chat');
  });

  it('ignores a collapsed (zero-size) pane element', () => {
    const collapsed = zoneAt('collapsed', { left: 0, top: 0, right: 0, bottom: 0 }, { priority: 99 });
    expect(pickDropZone([chat, collapsed], { x: 0, y: 0 }, 1)?.id).toBe('chat');
  });

  it('higher priority wins an overlap between two element zones', () => {
    const low = zoneAt('low', { left: 0, top: 0, right: 100, bottom: 100 }, { priority: 1 });
    const high = zoneAt('high', { left: 0, top: 0, right: 100, bottom: 100 }, { priority: 5 });
    expect(pickDropZone([low, high], { x: 10, y: 10 }, 1)?.id).toBe('high');
  });

  it('returns null when nothing matches and there is no fallback', () => {
    expect(pickDropZone([terminal], { x: 999, y: 999 }, 1)).toBeNull();
  });
});

describe('handleDragDropPayload routing', () => {
  it('delivers the raw paths of a terminal-targeted drop to the terminal zone only', async () => {
    const chatDrop = vi.fn();
    const termDrop = vi.fn();
    setDropHandlers({ onEnter: () => {}, onLeave: () => {}, onDrop: chatDrop });
    registerDropZone(zoneAt('terminal', { left: 0, top: 0, right: 100, bottom: 100 }, {
      priority: 10,
      onDrop: (paths) => termDrop(paths),
    }));

    await handleDragDropPayload({ type: 'drop', paths: ['/tmp/a.txt'], position: { x: 50, y: 50 } });

    expect(termDrop).toHaveBeenCalledWith(['/tmp/a.txt']);
    expect(chatDrop).not.toHaveBeenCalled();
  });

  it('routes a drop outside the terminal to the chat fallback', async () => {
    const termDrop = vi.fn();
    registerDropZone(zoneAt('terminal', { left: 0, top: 0, right: 100, bottom: 100 }, {
      priority: 10,
      onDrop: (paths) => termDrop(paths),
    }));
    const seen: string[][] = [];
    registerDropZone({ id: 'chat', getElement: () => null, priority: 0, onDrop: (paths) => { seen.push(paths); } });

    await handleDragDropPayload({ type: 'drop', paths: ['/tmp/b.txt'], position: { x: 500, y: 500 } });

    expect(termDrop).not.toHaveBeenCalled();
    expect(seen).toEqual([['/tmp/b.txt']]);
  });

  it('drives enter/leave overlays on the zone under the cursor, switching as it moves between panes', async () => {
    const terminalEnter = vi.fn();
    const terminalLeave = vi.fn();
    const chatEnter = vi.fn();
    const chatLeave = vi.fn();
    registerDropZone({ id: 'chat', getElement: () => null, priority: 0, onEnter: chatEnter, onLeave: chatLeave, onDrop: () => {} });
    registerDropZone(zoneAt('terminal', { left: 0, top: 0, right: 100, bottom: 100 }, {
      priority: 10, onEnter: terminalEnter, onLeave: terminalLeave, onDrop: () => {},
    }));

    // Enter over the chat area (outside the terminal).
    await handleDragDropPayload({ type: 'enter', paths: ['/tmp/c.txt'], position: { x: 500, y: 500 } });
    expect(chatEnter).toHaveBeenCalledTimes(1);
    expect(terminalEnter).not.toHaveBeenCalled();

    // Move onto the terminal pane — chat overlay hides, terminal overlay shows.
    await handleDragDropPayload({ type: 'over', position: { x: 50, y: 50 } });
    expect(chatLeave).toHaveBeenCalledTimes(1);
    expect(terminalEnter).toHaveBeenCalledTimes(1);

    // Leaving the window clears the active overlay.
    await handleDragDropPayload({ type: 'leave' });
    expect(terminalLeave).toHaveBeenCalledTimes(1);
  });

  it('ignores an internal card drag (enter with empty paths) — no overlay flash', async () => {
    const enter = vi.fn();
    registerDropZone({ id: 'chat', getElement: () => null, priority: 0, onEnter: enter, onDrop: () => {} });
    await handleDragDropPayload({ type: 'enter', paths: [], position: { x: 10, y: 10 } });
    expect(enter).not.toHaveBeenCalled();
  });

  it('reads file bytes only when a zone asks for them', async () => {
    const paths = ['/tmp/needs-bytes.png'];
    let received: string[] | null = null;
    registerDropZone({
      id: 'chat', getElement: () => null, priority: 0,
      onDrop: (p) => { received = p; }, // never calls readFiles
    });
    await handleDragDropPayload({ type: 'drop', paths, position: { x: 1, y: 1 } });
    // Path delivered without any Tauri file-read invocation (none is mocked,
    // so a read attempt would surface as an unhandled import).
    expect(received).toEqual(paths);
  });
});
