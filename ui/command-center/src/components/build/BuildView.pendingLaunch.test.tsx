/**
 * @vitest-environment jsdom
 *
 * BuildView's pendingTerminalLaunch consumer — the "two tabs" half of the
 * "Send to Claude" bug. BuildView's effect calls createProjectTab and then
 * clears the store ASYNCHRONOUSLY; if BuildView mounts twice before that
 * clear lands (StrictMode's dev double-invoke, or a remount from
 * navigateToTool('build') racing the same tick), the second run used to see
 * the same pending launch and open a second tab. The fix is the consume-once
 * claim in pendingLaunch.ts, wired into BuildView's effect.
 *
 * Route taken: the REAL BuildView is mounted (not a stand-in). Its heavy
 * children (Browser is a native-webview wrapper; useDashboard polls
 * /api/dashboard; ProjectChip/CostStatusline pull in their own project/cost
 * fetches) are mocked so mounting touches no network and no webview, but the
 * component under test — BuildView's own pendingTerminalLaunch effect,
 * running against the REAL zustand store and the REAL claimLaunch — is not
 * stubbed. TerminalManager is mocked down to a spy on createProjectTab,
 * which is what "one tab" is measured by.
 *
 * The double-mount race is reproduced literally: two BuildView instances are
 * mounted into two roots inside the same `act()`, so both render against the
 * same (not-yet-cleared) pendingTerminalLaunch before either instance's
 * effect has run — exactly the StrictMode/remount race in production.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../dashboard/useDashboard', () => ({
  useDashboard: () => ({ data: null, loading: false, error: false }),
}));

vi.mock('../browser', () => ({ Browser: () => null }));

vi.mock('./ProjectChip', () => ({ ProjectChip: () => null }));

vi.mock('./CostStatusline', () => ({ CostStatusline: () => null }));

const createProjectTabSpy = vi.fn();
vi.mock('../terminal/TerminalManager', () => {
  const React = require('react');
  const TerminalManager = React.forwardRef((_props: unknown, ref: unknown) => {
    React.useImperativeHandle(ref, () => ({
      createProjectTab: (...args: unknown[]) => createProjectTabSpy(...args),
      getActiveTab: () => ({ id: 'tab-1', label: 'Terminal', sessionId: null }),
      getAllTabs: () => [],
      killTab: async () => {},
    }));
    return null;
  });
  return { TerminalManager };
});

import { BuildView } from './BuildView';
import { useCommandCenter } from '../../lib/store';
import { resetClaimedLaunches } from './pendingLaunch';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

let containers: HTMLDivElement[] = [];
let roots: Root[] = [];

beforeEach(() => {
  createProjectTabSpy.mockClear();
  resetClaimedLaunches();
  useCommandCenter.setState({
    pendingTerminalLaunch: null,
    buildTerminalHidden: false,
    buildBrowserHidden: false,
  });
  containers = [];
  roots = [];
});

afterEach(() => {
  act(() => { roots.forEach(r => r.unmount()); });
  containers.forEach(c => c.remove());
  useCommandCenter.setState({ pendingTerminalLaunch: null });
});

function mountBuildView(): { container: HTMLDivElement; root: Root } {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  containers.push(container);
  roots.push(root);
  return { container, root };
}

describe('BuildView pendingTerminalLaunch — consume once', () => {
  it('opens exactly one tab for one launch even when two BuildView instances race the same pending launch', async () => {
    useCommandCenter.getState().setPendingTerminalLaunch({
      rootPath: '/tmp/proj',
      label: 'proj · claude',
      command: 'claude',
      followUpInput: 'do the thing',
    });
    const launchId = useCommandCenter.getState().pendingTerminalLaunch?.id;
    expect(launchId).toBeTruthy();

    const a = mountBuildView();
    const b = mountBuildView();

    // Both roots render — and their effects flush — inside the SAME act(),
    // reproducing a double-mount before either has cleared the store.
    await act(async () => {
      a.root.render(<BuildView />);
      b.root.render(<BuildView />);
      await Promise.resolve();
    });

    expect(createProjectTabSpy).toHaveBeenCalledTimes(1);
    expect(createProjectTabSpy).toHaveBeenCalledWith(
      '/tmp/proj',
      'proj · claude',
      'claude',
      undefined,
      expect.objectContaining({ followUpInput: 'do the thing', launchId }),
    );
    // Consumed: the store seam is cleared, not stuck for the next mount.
    expect(useCommandCenter.getState().pendingTerminalLaunch).toBeNull();
  });

  it('setting the same launch id twice opens only one tab', async () => {
    const fixedId = 'fixed-launch-id';
    const { root } = mountBuildView();
    await act(async () => {
      root.render(<BuildView />);
      await Promise.resolve();
    });

    await act(async () => {
      useCommandCenter.getState().setPendingTerminalLaunch({
        id: fixedId,
        rootPath: '/tmp/proj',
        label: 'proj · claude',
      });
      await Promise.resolve();
    });
    expect(createProjectTabSpy).toHaveBeenCalledTimes(1);

    // The same id arriving again (e.g. a re-fired agent event with a caller-
    // supplied id) must not open a second tab.
    await act(async () => {
      useCommandCenter.getState().setPendingTerminalLaunch({
        id: fixedId,
        rootPath: '/tmp/proj',
        label: 'proj · claude',
      });
      await Promise.resolve();
    });
    expect(createProjectTabSpy).toHaveBeenCalledTimes(1);
  });

  it('unhides a hidden terminal pane and still opens exactly one tab', async () => {
    // With the pane hidden, BuildView doesn't render TerminalManager at all
    // (see the `!buildTerminalHidden &&` guard around it), so terminalRef is
    // null going in — this is the case the "just call the ref" fix can't
    // cover, because there is no manager to call.
    useCommandCenter.setState({ buildTerminalHidden: true });

    const { root } = mountBuildView();
    await act(async () => {
      root.render(<BuildView />);
      await Promise.resolve();
    });
    expect(useCommandCenter.getState().buildTerminalHidden).toBe(true);

    let launchId: string | undefined;
    await act(async () => {
      useCommandCenter.getState().setPendingTerminalLaunch({
        rootPath: '/tmp/proj2',
        label: 'proj2 · claude',
        command: 'claude',
      });
      launchId = useCommandCenter.getState().pendingTerminalLaunch?.id;
      // First pass: the pane is still hidden at this point, so the effect
      // has nothing to call createProjectTab on — it can only flip the pane
      // visible, not claim the launch. If this fired, the launch would be
      // silently dropped instead of retried once a manager exists.
      expect(createProjectTabSpy).not.toHaveBeenCalled();
      // Let the unhide re-render, TerminalManager mount (setting the ref via
      // its layout effect), and the effect's re-run — triggered because
      // buildTerminalHidden is in its dep array — consume the launch.
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(launchId).toBeTruthy();
    expect(useCommandCenter.getState().buildTerminalHidden).toBe(false);
    expect(createProjectTabSpy).toHaveBeenCalledTimes(1);
    expect(createProjectTabSpy).toHaveBeenCalledWith(
      '/tmp/proj2',
      'proj2 · claude',
      'claude',
      undefined,
      expect.objectContaining({ launchId }),
    );
    // Consumed, not left queued behind the unhide.
    expect(useCommandCenter.getState().pendingTerminalLaunch).toBeNull();
  });
});
