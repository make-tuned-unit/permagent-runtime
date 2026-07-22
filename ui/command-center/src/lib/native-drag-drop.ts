/**
 * Bridge between Tauri's native drag-drop events and the React file handling pipeline.
 *
 * In Tauri, HTML5 drag-and-drop events don't fire for files dragged from Finder.
 * Tauri intercepts them at the native window level and exposes them via
 * onDragDropEvent on the webview window — a SINGLE window-level stream with no
 * per-DOM-element targeting. This module fans that one stream out to multiple
 * registered drop zones by hit-testing the event's position against each zone's
 * element bounds.
 *
 * WHY POSITION-SCOPED (see #550): the native drop stream is window-wide, so a
 * single global handler (the old design) captured EVERY file drop for the chat
 * — including drops meant for the Build-tab terminal pane, which could never
 * receive a file. Zones now declare the element they cover; a drop is routed to
 * the highest-priority zone whose bounds contain the drop point. This lets the
 * terminal pane claim drops over itself (#557) while the app-level chat zone
 * still catches drops everywhere else.
 *
 * The native listener is registered once (idempotent) and never torn down;
 * React components add/remove zones synchronously via registerDropZone(),
 * avoiding Strict Mode race conditions.
 */

/** Position carried by native enter/over/drop events (physical pixels). */
export interface NativeDropPosition {
  x: number;
  y: number;
}

/** Shape of the Tauri onDragDropEvent payload we consume. */
export type DragDropPayload =
  | { type: 'enter'; paths: string[]; position: NativeDropPosition }
  | { type: 'over'; position: NativeDropPosition }
  | { type: 'drop'; paths: string[]; position: NativeDropPosition }
  | { type: 'leave' };

/**
 * A drop target scoped to a DOM element. Registered via registerDropZone().
 */
export interface FileDropZone {
  /** Stable id — re-registering with the same id replaces the prior zone. */
  id: string;
  /**
   * Returns the element used to hit-test the drop position. Return `null` to
   * match anywhere (a window-wide fallback zone). Evaluated lazily on each
   * event so live layout (resized/hidden panes) is respected.
   */
  getElement: () => HTMLElement | null;
  /** Higher wins when zones overlap. Default 0. */
  priority?: number;
  onEnter?: () => void;
  onLeave?: () => void;
  /**
   * Called when a file drop lands inside this zone. Receives the raw
   * filesystem `paths` from the native event (what the terminal needs to
   * inject) plus a lazy `readFiles()` that reconstructs File objects with
   * bytes (what the chat needs to upload) — only pay the read cost if used.
   */
  onDrop: (paths: string[], readFiles: () => Promise<File[]>) => void | Promise<void>;
}

/** @deprecated back-compat shape for the old single-handler API. */
export interface DropHandlers {
  onEnter: () => void;
  onLeave: () => void;
  onDrop: (files: File[]) => void;
}

const zones: FileDropZone[] = [];
let activeZoneId: string | null = null;
let dragActive = false;
let listenerInitialized = false;

function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
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

async function readDroppedFiles(paths: string[]): Promise<File[]> {
  const files: File[] = [];
  for (const path of paths) {
    const file = await readDroppedFile(path);
    if (file) files.push(file);
  }
  return files;
}

/**
 * Pick the drop zone a native position falls into. Pure — exported for tests.
 *
 * The native position is in PHYSICAL pixels; element bounds are in CSS pixels,
 * so we divide by the device pixel ratio. Among all zones whose element
 * contains the point (a `null` element always matches, as a fallback), the
 * highest `priority` wins; ties break toward the most-recently-registered zone.
 */
export function pickDropZone(
  candidates: readonly FileDropZone[],
  position: NativeDropPosition,
  dpr = (typeof window !== 'undefined' && window.devicePixelRatio) || 1,
): FileDropZone | null {
  const x = position.x / dpr;
  const y = position.y / dpr;
  let best: FileDropZone | null = null;
  let bestPriority = -Infinity;
  for (const zone of candidates) {
    const el = zone.getElement();
    let contains: boolean;
    if (!el) {
      contains = true; // window-wide fallback
    } else {
      const r = el.getBoundingClientRect();
      contains =
        r.width > 0 && r.height > 0 &&
        x >= r.left && x <= r.right &&
        y >= r.top && y <= r.bottom;
    }
    if (!contains) continue;
    const p = zone.priority ?? 0;
    // `>=` lets a later registration win an equal-priority overlap.
    if (p >= bestPriority) {
      best = zone;
      bestPriority = p;
    }
  }
  return best;
}

function setActiveZone(next: FileDropZone | null): void {
  if ((next?.id ?? null) === activeZoneId) return;
  const prev = zones.find(z => z.id === activeZoneId);
  prev?.onLeave?.();
  activeZoneId = next?.id ?? null;
  next?.onEnter?.();
}

/**
 * Core router — drives zone enter/leave/drop from a native payload. Exported so
 * the routing behaviour can be unit-tested without a live Tauri window.
 */
export async function handleDragDropPayload(payload: DragDropPayload): Promise<void> {
  switch (payload.type) {
    case 'enter': {
      // Internal HTML5 card drags fire this with empty paths — ignore so we
      // don't flash a file-drop overlay for a card being dragged in-app.
      if (!payload.paths || payload.paths.length === 0) return;
      dragActive = true;
      setActiveZone(pickDropZone(zones, payload.position));
      break;
    }
    case 'over': {
      if (!dragActive) return;
      // Update the active zone as the cursor moves between panes mid-drag.
      setActiveZone(pickDropZone(zones, payload.position));
      break;
    }
    case 'leave': {
      dragActive = false;
      setActiveZone(null);
      break;
    }
    case 'drop': {
      dragActive = false;
      const paths = payload.paths ?? [];
      const target = paths.length > 0 ? pickDropZone(zones, payload.position) : null;
      setActiveZone(null); // hide overlays immediately
      if (!target || paths.length === 0) return;
      await target.onDrop(paths, () => readDroppedFiles(paths));
      break;
    }
  }
}

/** Initialize the singleton native drag-drop listener. Called once, idempotent. */
async function ensureNativeListener(): Promise<void> {
  if (listenerInitialized || !isTauri()) return;
  listenerInitialized = true;

  try {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    const webview = getCurrentWebviewWindow();
    await webview.onDragDropEvent((event) => {
      void handleDragDropPayload(event.payload as DragDropPayload);
    });
    console.log('[drag-drop] native listener registered successfully');
  } catch (e) {
    console.error('[drag-drop] failed to register native listener:', e);
    listenerInitialized = false;
  }
}

/**
 * Register a position-scoped drop zone. Returns an unregister function.
 * Re-registering an existing id replaces it (and clears it as the active zone
 * if it was one), so a component can safely re-run this on ref/dep changes.
 */
export function registerDropZone(zone: FileDropZone): () => void {
  const existing = zones.findIndex(z => z.id === zone.id);
  if (existing !== -1) {
    zones.splice(existing, 1);
  }
  zones.push(zone);
  if (isTauri()) ensureNativeListener();
  return () => {
    const idx = zones.findIndex(z => z.id === zone.id);
    if (idx !== -1) zones.splice(idx, 1);
    if (activeZoneId === zone.id) activeZoneId = null;
  };
}

const LEGACY_ZONE_ID = 'legacy-window-fallback';
let legacyUnregister: (() => void) | null = null;

/**
 * @deprecated Prefer registerDropZone() with an explicit element. Kept for the
 * app-level chat DropZone: registers a window-wide (element-null) fallback zone
 * at priority 0, so any more-specific pane zone (e.g. the terminal) wins over
 * its own bounds while the chat still catches drops everywhere else. Pass
 * `null` to unregister.
 */
export function setDropHandlers(handlers: DropHandlers | null): void {
  legacyUnregister?.();
  legacyUnregister = null;
  if (!handlers) return;
  legacyUnregister = registerDropZone({
    id: LEGACY_ZONE_ID,
    getElement: () => null,
    priority: 0,
    onEnter: handlers.onEnter,
    onLeave: handlers.onLeave,
    onDrop: async (_paths, readFiles) => {
      const files = await readFiles();
      if (files.length > 0) handlers.onDrop(files);
    },
  });
}

/**
 * @deprecated Use registerDropZone() instead. Kept for backward compatibility.
 */
export async function registerDropHandler(handlers: DropHandlers): Promise<() => void> {
  setDropHandlers(handlers);
  return () => setDropHandlers(null);
}

/** Test-only: clear all registered zones and reset router state. */
export function __resetDropZonesForTest(): void {
  zones.length = 0;
  activeZoneId = null;
  dragActive = false;
  legacyUnregister = null;
}
