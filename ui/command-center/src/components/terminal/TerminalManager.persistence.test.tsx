/**
 * @vitest-environment jsdom
 *
 * TerminalManager persistence across a SINGLE-COMMIT remount.
 *
 * The regression this pins (2026-08-06): toggling the Build browser pane
 * changes the panel Group's key, which unmounts and remounts TerminalManager
 * in one React commit. React runs the NEW instance's useState initializer
 * during render, BEFORE the old instance's effect cleanup runs in the commit
 * phase — so an unmount-only persist handed the new instance stale (or null)
 * state: the user's tabs vanished into one fresh terminal while their PTYs
 * (and Claude Code) kept running, and only came back on the NEXT toggle.
 * Persistence must therefore be continuous, surviving any remount schedule.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

// The real Terminal spins up xterm + Tauri listeners; this suite is about the
// MANAGER's tab state, so the child records its props and does nothing.
interface RenderedTerminal {
  sessionId: string | null;
  cwd?: string;
  initialCommand?: string;
}
const rendered: RenderedTerminal[] = [];
vi.mock('./Terminal', () => ({
  Terminal: (props: RenderedTerminal) => {
    rendered.push({ sessionId: props.sessionId, cwd: props.cwd, initialCommand: props.initialCommand });
    return <div data-terminal-session={props.sessionId ?? ''} />;
  },
}));
vi.mock('../../lib/native-drag-drop', () => ({
  registerDropZone: () => () => {},
}));

import {
  TerminalManager,
  __resetTerminalPersistenceForTests,
  type TerminalManagerHandle,
} from './TerminalManager';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  __resetTerminalPersistenceForTests();
  rendered.length = 0;
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function tabLabels(): string[] {
  // Tab strip buttons carry the label inside a truncating span.
  return Array.from(container.querySelectorAll('button span.truncate')).map(
    el => el.textContent ?? '',
  );
}

describe('TerminalManager tab persistence', () => {
  it('keeps all tabs and the active session through a single-commit key remount', () => {
    const ref = createRef<TerminalManagerHandle>();
    act(() => {
      root.render(<TerminalManager key="a" ref={ref} />);
    });

    // A project launch adds a second tab (the Claude Code case).
    act(() => {
      ref.current!.createProjectTab('/Users/x/dev/proj', 'proj · claude', 'claude');
    });
    const projTab = ref.current!.getActiveTab();

    // The module-level tab counter is intentionally not reset per mount, so
    // pin the SHAPE (default tab + project tab) and compare identity across
    // the remount rather than hardcoding "Terminal 1".
    const before = tabLabels();
    expect(before).toHaveLength(2);
    expect(before[0]).toMatch(/^Terminal \d+$/);
    expect(before[1]).toBe('proj · claude');
    expect(projTab.label).toBe('proj · claude');

    // The Build pane toggle: same position, new key — unmount+remount in ONE
    // commit. Before the fix, this rendered a single fresh "Terminal N" tab.
    const ref2 = createRef<TerminalManagerHandle>();
    act(() => {
      root.render(<TerminalManager key="b" ref={ref2} />);
    });

    expect(tabLabels()).toEqual(before);
    // The active tab survives too — the user comes back to what they were
    // looking at, not to the first tab.
    expect(ref2.current!.getActiveTab().label).toBe('proj · claude');
    expect(ref2.current!.getAllTabs()).toHaveLength(2);
  });

  it('remains correct across repeated toggles (there and back again)', () => {
    const ref = createRef<TerminalManagerHandle>();
    act(() => {
      root.render(<TerminalManager key="a" ref={ref} />);
    });
    act(() => {
      ref.current!.createProjectTab('/Users/x/dev/one', 'one · codex');
    });
    const stable = tabLabels();
    expect(stable).toHaveLength(2);

    for (const k of ['b', 'c', 'd']) {
      const r = createRef<TerminalManagerHandle>();
      act(() => {
        root.render(<TerminalManager key={k} ref={r} />);
      });
      expect(tabLabels()).toEqual(stable);
    }
  });

  it('a genuinely fresh mount still starts with one empty tab', () => {
    act(() => {
      root.render(<TerminalManager key="fresh" />);
    });
    expect(tabLabels()).toHaveLength(1);
    expect(tabLabels()[0]).toMatch(/^Terminal \d+$/);
  });

  it('detached pane windows do not write into the shared persistence', () => {
    const detachedTab = {
      id: 'tab-detached',
      label: 'popped-out',
      sessionId: 'pty-99',
    };
    act(() => {
      root.render(<TerminalManager key="det" detached initialTab={detachedTab} />);
    });
    expect(tabLabels()).toEqual(['popped-out']);

    // A later docked mount must NOT inherit the detached window's tab.
    act(() => {
      root.render(<TerminalManager key="docked" />);
    });
    expect(tabLabels()).toHaveLength(1);
    expect(tabLabels()[0]).toMatch(/^Terminal \d+$/);
  });
});
