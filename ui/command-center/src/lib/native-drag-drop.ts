/**
 * Bridge between Tauri's native drag-drop events and the React file handling pipeline.
 *
 * In Tauri, HTML5 drag-and-drop events don't fire for files dragged from Finder.
 * Tauri intercepts them at the native window level and exposes them via
 * onDragDropEvent on the webview window. This module listens for those events
 * and converts native file paths into File objects the React layer can consume.
 */

export interface DropHandlers {
  onEnter: () => void;
  onLeave: () => void;
  onDrop: (files: File[]) => void;
}

let currentHandlers: DropHandlers | null = null;
let cleanupFn: (() => void) | null = null;

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
    return new File([bytes], filename, { type: mimeType });
  } catch (e) {
    console.warn('Failed to read dropped file:', path, e);
    return null;
  }
}

export async function registerDropHandler(handlers: DropHandlers): Promise<() => void> {
  // Always store handlers (DropZone will call these)
  currentHandlers = handlers;

  // Only set up native listener in Tauri context
  if (!isTauri()) {
    return () => { currentHandlers = null; };
  }

  // Clean up previous listener
  if (cleanupFn) {
    cleanupFn();
    cleanupFn = null;
  }

  try {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const webview = getCurrentWebviewWindow();

    const unlisten = await webview.onDragDropEvent(async (event) => {
      if (!currentHandlers) return;

      if (event.payload.type === 'enter') {
        currentHandlers.onEnter();
      } else if (event.payload.type === 'leave') {
        currentHandlers.onLeave();
      } else if (event.payload.type === 'drop') {
        currentHandlers.onLeave(); // Hide overlay immediately
        const paths: string[] = event.payload.paths;
        if (paths.length === 0) return;

        const files: File[] = [];
        for (const path of paths) {
          const file = await readDroppedFile(path);
          if (file) files.push(file);
        }
        if (files.length > 0) {
          currentHandlers.onDrop(files);
        }
      }
    });

    cleanupFn = unlisten;
    return () => {
      unlisten();
      cleanupFn = null;
      currentHandlers = null;
    };
  } catch (e) {
    console.warn('Failed to register Tauri drag-drop listener:', e);
    return () => { currentHandlers = null; };
  }
}
