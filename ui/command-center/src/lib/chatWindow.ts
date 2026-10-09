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
import { Effect, EffectState } from '@tauri-apps/api/window';
import { useCommandCenter } from './store';
import { CHAT_DRAFT_HANDOFF, CHAT_DRAFT_HANDOFF_ACK, encodeDraftFiles } from './chatDraftHandoff';
import { peekChatSetupIntentRecord } from './wizardIntent';
import {
  applyWindowTransparency, NATIVE_WINDOW_IS_OPAQUE, OVERLAY_TITLEBAR, reinsetTrafficLights,
} from './windowChrome';

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
let creatingChatWindow: Promise<WebviewWindowType> | null = null;

export function createChatWindow(appTheme: string): Promise<WebviewWindowType> {
  if (creatingChatWindow) return creatingChatWindow;
  const pending = createChatWindowOnce(appTheme);
  creatingChatWindow = pending;
  const clear = () => { if (creatingChatWindow === pending) creatingChatWindow = null; };
  pending.then(clear, clear);
  return pending;
}

async function createChatWindowOnce(appTheme: string): Promise<WebviewWindowType> {
  // Freeze the composer before any file encoding or native imports can yield.
  // Its store copy remains intact if any part of the handoff fails.
  const dockWasOpen = useCommandCenter.getState().chatDockOpen;
  useCommandCenter.setState({ chatWindowOpen: true, chatDockOpen: false });
  let chat: WebviewWindowType | undefined;
  let unready: (() => void) | undefined;
  let unack: (() => void) | undefined;
  try {
  const { WebviewWindow } = await import('@tauri-apps/api/webviewWindow');
  const draft = useCommandCenter.getState().chatDraft;
  // Always use a nonce: a Devices request can arrive while this window is
  // opening, after the initial snapshot but before its listener is ready.
  const handoffId = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
  // The dock is already frozen. A failed read restores it with its File
  // objects still in the main window's store.
  const handoffFiles = await encodeDraftFiles(draft.files);
  const { emitTo, listen } = await import('@tauri-apps/api/event');
  let readyResolve: (() => void) | undefined;
  let ackResolve: (() => void) | undefined;
  const ready = new Promise<void>(resolve => { readyResolve = resolve; });
  const ack = new Promise<void>(resolve => { ackResolve = resolve; });
  const bounded = (work: Promise<void>, label: string) => new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${label} timed out`)), 8000);
    work.then(() => { clearTimeout(timer); resolve(); }, error => { clearTimeout(timer); reject(error); });
  });
  unready = await listen<{ draft_handoff: string | null }>('chat_ready', event => {
    if (event.payload?.draft_handoff === handoffId) readyResolve?.();
  });
  unack = await listen<{ id: string }>(CHAT_DRAFT_HANDOFF_ACK, event => {
    if (event.payload?.id === handoffId) ackResolve?.();
  });
  const saved = loadChatGeometry();
  const placement = saved
    ? { x: saved.x, y: saved.y, width: saved.width, height: saved.height }
    : { ...DEFAULT_SIZE, center: true };

  // Hand the CURRENT session id to the chat window explicitly. It used to be
  // discovered via localStorage, but on macOS WKWebView storage does not
  // reliably cross windows — so the popped-out chat would find nothing, mint a
  // fresh session, and replay Henry's greeting over the conversation the user
  // was already having in the dock. Passing it in the URL makes the handoff
  // deterministic regardless of storage partitioning.
  // Make the handoff TOTAL: with no current session the URL carried nothing
  // and the popped-out window minted a fresh session — a genuinely new
  // conversation, hence a genuine (repeated) greeting. This bit voice-only
  // users every time: their turns live in a session the dock never adopted,
  // so chatSessionId was null exactly when a conversation was in progress.
  let currentSession = useCommandCenter.getState().chatSessionId;
  if (!currentSession) {
    try {
      currentSession = await useCommandCenter.getState().ensureSession();
    } catch { /* fall through: the window will mint one as a last resort */ }
  }
  const params = new URLSearchParams({ view: 'chat' });
  if (currentSession) params.set('session', currentSession);
  params.set('draft_handoff', handoffId);
  const url = `index.html?${params.toString()}`;

  // The same chrome the main window wears: an overlay titlebar over our own
  // HTML, and the native material behind it. Runtime windows need both halves
  // applied AFTER creation — the builder silently drops `trafficLightPosition`
  // and does not carry transparency to the webview (see windowChrome.ts).
    chat = new WebviewWindow('chat', {
    url,
    title: 'Permagent Chat',
    ...MIN_SIZE,
    ...OVERLAY_TITLEBAR,
    decorations: true,
    resizable: true,
    focus: true,
    transparent: !NATIVE_WINDOW_IS_OPAQUE,
    windowEffects: NATIVE_WINDOW_IS_OPAQUE
      ? undefined
      : { effects: [Effect.Sidebar], state: EffectState.FollowsWindowActiveState },
    // The in-app browser is a native child webview of the MAIN window. Native
    // child webviews always paint above the main window's HTML, and they ride
    // the main window's z-order — so the moment the user clicks the browser,
    // the main window (and its browser webview) comes forward and paints over
    // any overlapping part of the chat window, clipping it at the browser's
    // edge. `focus` alone only front-orders once and is lost on that next
    // click.
    //
    // The chat window is an INDEPENDENT top-level window — NOT a child of main
    // (`parent`/addChildWindow would couple it to main: it would move with the
    // main window and could no longer fullscreen or tile into split-screen,
    // see #477) and NOT global `alwaysOnTop` (that floated it above ALL apps,
    // see #461). Instead, the main window re-asserts the chat window's stacking
    // just above itself via the native `raise_chat_above_main` command on every
    // main-window focus (see ChatLauncher + main.rs). That keeps the chat above
    // the main window's browser child-webview WITHOUT coupling movement or
    // floating above other apps.
    theme: appTheme === 'silver' ? 'light' : 'dark',
    ...placement,
    });
    void chat.once('tauri://created', () => {
      void reinsetTrafficLights('chat');
      void applyWindowTransparency('chat');
    });
    await bounded(ready, 'Chat draft handoff readiness');
    await emitTo('chat', CHAT_DRAFT_HANDOFF, {
      id: handoffId, text: draft.text, files: handoffFiles,
      pendingSetup: peekChatSetupIntentRecord(),
    });
    await bounded(ack, 'Chat draft handoff acknowledgement');
    return chat;
  } catch (error) {
    try { await chat?.close(); } catch { /* the window may not have been created */ }
    useCommandCenter.setState({ chatWindowOpen: false, chatDockOpen: dockWasOpen });
    throw error;
  } finally {
    unready?.();
    unack?.();
  }
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
