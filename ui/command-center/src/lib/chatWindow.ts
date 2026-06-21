/**
 * Chat-window creation + geometry persistence.
 *
 * Single source of truth for opening the separate `chat` WebviewWindow so the
 * main window's drop handler (App.tsx) and the launcher button (ChatLauncher)
 * stay in sync — previously these were duplicated and App.tsx omitted `theme`.
 *
 * Position/size are persisted in localStorage (shared origin across the main
 * and chat windows) and restored on open, so the window reappears where the
 * user left it instead of re-centering every launch. Geometry is stored in
 * LOGICAL pixels to match Tauri's WebviewWindow option units.
 */
import type { WebviewWindow as WebviewWindowType } from '@tauri-apps/api/webviewWindow';

const CHAT_GEOMETRY_KEY = 'permagent.chatWindow.geometry';

export interface ChatGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

const DEFAULT_SIZE = { width: 480, height: 700 };
const MIN_SIZE = { minWidth: 360, minHeight: 400 };

export function loadChatGeometry(): ChatGeometry | null {
  try {
    const raw = localStorage.getItem(CHAT_GEOMETRY_KEY);
    if (!raw) return null;
    const g = JSON.parse(raw);
    if (
      typeof g?.x === 'number' && typeof g?.y === 'number' &&
      typeof g?.width === 'number' && typeof g?.height === 'number'
    ) {
      return g;
    }
  } catch { /* corrupt entry — fall back to centered default */ }
  return null;
}

export function saveChatGeometry(g: ChatGeometry): void {
  try { localStorage.setItem(CHAT_GEOMETRY_KEY, JSON.stringify(g)); } catch { /* ignore */ }
}

/**
 * Create the chat window with consolidated options. Restores last-known
 * geometry when available, otherwise centers at the default size.
 * `appTheme` is the app theme key ('silver' → light, else dark).
 */
export async function createChatWindow(appTheme: string): Promise<WebviewWindowType> {
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  const saved = loadChatGeometry();
  const placement = saved
    ? { x: saved.x, y: saved.y, width: saved.width, height: saved.height }
    : { ...DEFAULT_SIZE, center: true };

  return new WebviewWindow('chat', {
    url: 'index.html?view=chat',
    title: 'Permagent Chat',
    ...MIN_SIZE,
    decorations: true,
    resizable: true,
    focus: true,
    // The in-app browser is a native child webview of the MAIN window. Native
    // child webviews always paint above the main window's HTML, and they ride
    // the main window's z-order — so the moment the user clicks the browser,
    // the main window (and its browser webview) comes forward and paints over
    // any overlapping part of the chat window, clipping it at the browser's
    // edge. `focus` alone only front-orders once and is lost on that next
    // click. `alwaysOnTop` decouples the chat window from the main window's
    // z-order entirely, so the browser can never clip it.
    alwaysOnTop: true,
    theme: appTheme === 'silver' ? 'light' : 'dark',
    ...placement,
  });
}

/**
 * Attach move/resize listeners to the current (chat) window that persist its
 * geometry. Call from inside the chat window (ChatApp). Returns a cleanup fn.
 */
export async function trackChatGeometry(): Promise<() => void> {
  if (!('__TAURI_INTERNALS__' in window)) return () => {};
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const win = getCurrentWindow();

  const persist = async () => {
    try {
      const factor = await win.scaleFactor();
      const pos = (await win.outerPosition()).toLogical(factor);
      const size = (await win.innerSize()).toLogical(factor);
      saveChatGeometry({ x: pos.x, y: pos.y, width: size.width, height: size.height });
    } catch { /* ignore transient failures */ }
  };

  const unMoved = await win.onMoved(persist);
  const unResized = await win.onResized(persist);
  return () => { unMoved(); unResized(); };
}
