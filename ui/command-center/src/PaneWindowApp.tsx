import { useEffect, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { Browser } from './components/browser';
import type { BrowserTab } from './components/browser/BrowserTabs';
import { TerminalManager, type TerminalManagerHandle, type TerminalTab } from './components/terminal/TerminalManager';
import { emitRedock, takePaneTab, type PaneKind } from './lib/paneWindows';
import { useTheme } from './styles/useTheme';

export default function PaneWindowApp() {
  const params = new URLSearchParams(location.search);
  const kind = params.get('kind') as PaneKind;
  const owner = params.get('owner') || '';
  const handoffConsumed = useRef(false);
  const [initial, setInitial] = useState<{ loaded: boolean; tab: TerminalTab | BrowserTab | null }>({ loaded: false, tab: null });
  const terminalRef = useRef<TerminalManagerHandle>(null);
  const browserRef = useRef<{ getActiveTab: () => BrowserTab; getAllTabs: () => BrowserTab[] }>(null);
  const { gradient } = useTheme();

  useEffect(() => {
    if (handoffConsumed.current) return;
    handoffConsumed.current = true;
    setInitial({ loaded: true, tab: takePaneTab(owner) });
  }, [owner]);

  useEffect(() => {
    const win = getCurrentWindow();
    let unClose: (() => void) | undefined;
    let unResize: (() => void) | undefined;
    let closing = false;

    // Close AND minimize both mean "put this back in the app" (2026-07-27
    // ruling): content the user had open — a loaded site, a live shell —
    // redocks into the main window as tabs, and the pane window ALWAYS dies.
    // The original bug was never the redock; it was the window refusing to
    // close when the handoff threw. So every path here ends in destroy():
    // blank tabs simply close with the window, redock failures fall back to
    // the atomic native teardown (`destroy_pane_window`) so no child webview
    // is left stranded offscreen, and the finally destroys unconditionally.
    // Redocking moves live webviews back to the main window before the pane
    // dies — but reparent/emit are main-thread native ops, and if the main
    // window is busy (e.g. its bounds pump is churning) they can stall. The
    // pane window's own teardown doc (browser.rs) warns that awaiting per-tab
    // IPC before destroy() is exactly what makes the window refuse to close.
    // So the redock is best-effort under a hard deadline: whatever hasn't
    // handed over in time is torn down atomically instead, and the window is
    // ALWAYS destroyed. A pane that won't close is a worse bug than a browser
    // tab that closes with its window.
    const withDeadline = <T,>(p: Promise<T>, ms: number): Promise<T | undefined> =>
      Promise.race([
        p,
        new Promise<undefined>(resolve => setTimeout(() => resolve(undefined), ms)),
      ]);

    const redockAndClose = async () => {
      if (closing) return;
      closing = true;
      try {
        await withDeadline(redockAll(), 2000);
      } finally {
        // `destroy()` MUST be bounded too. It was the one await in this path
        // without a deadline, and it is the one that strands a window: after
        // `redockAll` reparents the native child webviews out of this window,
        // destroying it can stall on the same main-thread contention the
        // redock was already guarded against. Observed 2026-08-04 — the tab
        // came back to the main app, its content flashed behind, and the now
        // EMPTY pane window refused to close. A bounded redock followed by an
        // unbounded destroy just moves the hang one line down.
        //
        // So: try the JS teardown briefly, then fall back to the atomic native
        // one (`destroy_pane_window`), which does not depend on this webview's
        // IPC round-trip completing. No child webview ids — `redockAll` has
        // already moved them to the main window and destroying them here would
        // kill the tabs the user just got back.
        const destroyed = await withDeadline(
          win.destroy().then(() => true).catch(() => true),
          1200,
        );
        if (!destroyed) {
          await invoke('destroy_pane_window', {
            windowLabel: win.label,
            webviewIds: [],
          }).catch(() => { /* nothing left that can close it */ });
        }
        closing = false;
      }
    };

    const redockAll = async () => {
      {
        if (kind === 'browser') {
          // Hand EVERY loaded tab back (the old active-tab-only redock leaked
          // the rest). Reparent moves the native webview — DOM, history,
          // cookies intact — then the redock event re-tabs it in the app.
          const tabs = (browserRef.current?.getAllTabs() ?? []).filter(tab => tab.webviewId);
          const stranded: string[] = [];
          for (const tab of tabs) {
            try {
              // Park the webview offscreen BEFORE the handoff: a reparented
              // webview keeps its pane-window geometry, so un-hidden it lands
              // splayed across the main window (flashing over the terminal)
              // until the bounds pump catches it. Hidden, it only appears once
              // the main Browser places it.
              await invoke('hide_browser', { webviewId: tab.webviewId }).catch(() => {});
              await invoke('reparent_browser', { webviewId: tab.webviewId, windowLabel: 'main' });
              await emitRedock('browser', tab);
            } catch (error) {
              console.error('Failed to redock browser tab on close', error);
              stranded.push(tab.webviewId as string);
            }
          }
          if (stranded.length > 0) {
            await invoke('destroy_pane_window', {
              windowLabel: win.label,
              webviewIds: stranded,
            }).catch(() => { /* the finally destroy still runs */ });
          }
        } else {
          // PTYs live in the main process — redocking the tab is enough for
          // the shell to keep running; every live session goes back.
          const tabs = (terminalRef.current?.getAllTabs() ?? []).filter(tab => tab.sessionId);
          for (const tab of tabs) {
            try {
              await emitRedock('terminal', tab);
            } catch (error) {
              console.error('Failed to redock terminal tab on close', error);
            }
          }
        }
      }
    };

    void win.onCloseRequested(event => {
      // preventDefault so the handoff can run; redockAndClose always destroys
      // the window itself.
      event.preventDefault();
      void redockAndClose();
    }).then(fn => { unClose = fn; });
    // Native minimize is intentionally treated as "put this back in the app".
    // Tauri reports it through the resize stream; check native state rather
    // than inferring from a zero-sized payload (which varies by platform).
    void win.onResized(async () => {
      if (await win.isMinimized()) void redockAndClose();
    }).then(fn => { unResize = fn; });
    return () => { unClose?.(); unResize?.(); };
  }, [kind]);

  return (
    <div style={{ width: '100vw', height: '100vh', background: gradient.workspace }}>
      {kind === 'browser'
        ? initial.loaded && <Browser ref={browserRef} initialTab={initial.tab as BrowserTab | null} ownerWindowLabel={owner} detached />
        : initial.loaded && <TerminalManager ref={terminalRef} initialTab={initial.tab as TerminalTab | null} detached />}
    </div>
  );
}
