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
  const browserRef = useRef<{ getActiveTab: () => BrowserTab }>(null);
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
    let redocking = false;
    const redock = async () => {
      if (redocking) return;
      redocking = true;
      try {
        if (kind === 'browser') {
          const tab = browserRef.current?.getActiveTab();
          if (tab?.webviewId) await invoke('reparent_browser', { webviewId: tab.webviewId, windowLabel: 'main' });
          if (tab) await emitRedock('browser', tab);
        } else {
          const tab = terminalRef.current?.getActiveTab();
          if (tab) await emitRedock('terminal', tab);
        }
        await win.destroy();
      } catch (error) {
        console.error('Failed to redock detached pane', error);
      } finally {
        redocking = false;
      }
    };
    void win.onCloseRequested(event => {
      event.preventDefault();
      void redock();
    }).then(fn => { unClose = fn; });
    // Native minimize is intentionally treated as "put this back in the app".
    // Tauri reports it through the resize stream; check native state rather
    // than inferring from a zero-sized payload (which varies by platform).
    void win.onResized(async () => {
      if (await win.isMinimized()) void redock();
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
