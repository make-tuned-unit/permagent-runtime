/**
 * Bridge between Tauri's native drag-drop events and the React file handling pipeline.
 *
 * In Tauri, HTML5 drag-and-drop events don't fire for files dragged from Finder.
 * Tauri intercepts them at the native window level and exposes them via
 * onDragDropEvent on the webview window. This module listens for those events
 * and converts native file paths into File objects the React layer can consume.
 *
 * Uses a singleton listener pattern: the native event listener is registered
 * once and never unregistered. React components swap handlers synchronously
 * via setDropHandlers(), avoiding Strict Mode race conditions.
 */

export interface DropHandlers {
  onEnter: () => void;
  onLeave: () => void;
  onDrop: (files: File[]) => void;
}

let currentHandlers: DropHandlers | null = null;
let listenerInitialized = false;

function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

async function readDroppedFile(path: string): Promise<File | null> {
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const [filename, mimeType, b64Data] = await invoke<[string, string, string]>(
      'read_dropped_file',
      { path },
    );
    const bytes = Uint8Array.from(atob(b64Data), c => c.charCodeAt(0));
    console.log(`[drag-drop] file read success: ${filename}, mime=${mimeType}, bytes=${bytes.length}`);
    return new File([bytes], filename, { type: mimeType });
  } catch (e) {
    console.error('[drag-drop] file read failure:', path, e);
    return null;
  }
}

/** Initialize the singleton native drag-drop listener. Called once, idempotent. */
async function ensureNativeListener(): Promise<void> {
  if (listenerInitialized || !isTauri()) return;
  listenerInitialized = true;

  try {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const webview = getCurrentWebviewWindow();

    await webview.onDragDropEvent(async (event) => {
      const handlers = currentHandlers;
      if (!handlers) {
        console.log('[drag-drop] event received but no handlers registered, type:', event.payload.type);
        return;
      }

      if (event.payload.type === 'enter') {
        const paths: string[] = (event.payload as { type: string; paths: string[] }).paths;
        console.log('[drag-drop] enter, paths:', paths);
        handlers.onEnter();
      } else if (event.payload.type === 'leave') {
        console.log('[drag-drop] leave');
        handlers.onLeave();
      } else if (event.payload.type === 'drop') {
        const paths: string[] = (event.payload as { type: string; paths: string[] }).paths;
        console.log('[drag-drop] drop, paths:', paths, 'count:', paths.length);
        handlers.onLeave(); // Hide overlay immediately

        if (paths.length === 0) return;

        const files: File[] = [];
        for (const path of paths) {
          const file = await readDroppedFile(path);
          if (file) files.push(file);
        }
        console.log('[drag-drop] files ready:', files.length, files.map(f => `${f.name}(${f.type})`));
        if (files.length > 0) {
          handlers.onDrop(files);
        }
      } else if (event.payload.type === 'over') {
        // Position update — ignore silently
      } else {
        console.log('[drag-drop] unknown event type:', (event.payload as { type: string }).type);
      }
    });

    console.log('[drag-drop] native listener registered successfully');
  } catch (e) {
    console.error('[drag-drop] failed to register native listener:', e);
    listenerInitialized = false;
  }
}

/**
 * Set (or clear) the current drop handlers. Synchronous — no race conditions.
 * Call with `null` to unregister.
 */
export function setDropHandlers(handlers: DropHandlers | null): void {
  currentHandlers = handlers;
  if (handlers && isTauri()) {
    ensureNativeListener();
  }
}

/**
 * @deprecated Use setDropHandlers() instead. Kept for backward compatibility.
 */
export async function registerDropHandler(handlers: DropHandlers): Promise<() => void> {
  setDropHandlers(handlers);
  return () => setDropHandlers(null);
}
