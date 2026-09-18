import { OVERLAY_TITLEBAR, reinsetTrafficLights } from './windowChrome';
import type { BrowserTab } from '../components/browser/BrowserTabs';
import type { TerminalTab } from '../components/terminal/TerminalManager';

export type PaneKind = 'terminal' | 'browser';
export type PaneTab = TerminalTab | BrowserTab;

const HANDOFF_PREFIX = 'permagent.pane-window.';

export function paneWindowLabel(kind: PaneKind): string {
  return `${kind}-${crypto.randomUUID()}`;
}

export function stashPaneTab(label: string, tab: PaneTab): void {
  localStorage.setItem(HANDOFF_PREFIX + label, JSON.stringify(tab));
}

export function takePaneTab(label: string): PaneTab | null {
  const key = HANDOFF_PREFIX + label;
  const raw = localStorage.getItem(key);
  localStorage.removeItem(key);
  if (!raw) return null;
  try { return JSON.parse(raw) as PaneTab; } catch { return null; }
}

export async function createPaneWindow(kind: PaneKind, tab: PaneTab): Promise<string> {
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  const label = paneWindowLabel(kind);
  stashPaneTab(label, tab);
  const win = new WebviewWindow(label, {
    url: `index.html?view=pane&kind=${kind}&owner=${encodeURIComponent(label)}`,
    title: kind === 'terminal' ? 'Permagent Terminal' : 'Permagent Browser',
    width: kind === 'terminal' ? 860 : 1100,
    height: kind === 'terminal' ? 560 : 760,
    minWidth: 420,
    minHeight: 300,
    center: true,
    // `decorations: true` keeps the free macOS window corner radius and the
    // native drop shadow; the overlay titlebar is what lets the pane's own
    // chrome run edge-to-edge under it. See lib/windowChrome.ts.
    decorations: true,
    ...OVERLAY_TITLEBAR,
    resizable: true,
    focus: true,
  });
  await new Promise<void>((resolve, reject) => {
    void win.once('tauri://created', () => resolve());
    void win.once('tauri://error', e => reject(e.payload));
  });
  // The builder path drops `trafficLightPosition`; PaneWindowApp asks for the
  // inset on mount too, but doing it here as well means a pane whose webview is
  // slow to boot does not sit with its buttons in the wrong place meanwhile.
  await reinsetTrafficLights(label);
  return label;
}

export async function emitRedock(kind: PaneKind, tab: PaneTab): Promise<void> {
  const { emit } = await import('@tauri-apps/api/event');
  await emit('pane_redock', { kind, tab });
}
