/**
 * World W2 — layers inside ONE place panel, and their Escape order.
 *
 * Escape closes the topmost layer of the panel only (confirm → composer →
 * panel) and hands focus back to the control that opened that layer. The
 * stack lives in React context on the panel root and the key is handled by
 * the panel's own onKeyDown — no window or document listener is installed.
 */
import { createContext, useContext, useEffect, useLayoutEffect, useRef, type RefObject } from 'react';

interface Layer {
  id: number;
  close: () => void;
  trigger: RefObject<HTMLElement | null>;
}

export interface LayerStack {
  push(layer: Omit<Layer, 'id'>): () => void;
  /** Close the topmost layer. Returns false when only the panel itself is left. */
  closeTop(): boolean;
  depth(): number;
}

export function createLayerStack(): LayerStack {
  let next = 0;
  const layers: Layer[] = [];
  return {
    push(layer) {
      const entry = { ...layer, id: next += 1 };
      layers.push(entry);
      return () => {
        const i = layers.findIndex((l) => l.id === entry.id);
        if (i >= 0) layers.splice(i, 1);
      };
    },
    closeTop() {
      const top = layers.pop();
      if (!top) return false;
      top.close();
      return true;
    },
    depth: () => layers.length,
  };
}

export const LayerContext = createContext<LayerStack | null>(null);

/**
 * Register a layer while `open`. When it closes — by Escape or by its own
 * buttons — focus returns to `trigger` if focus was inside the layer or lost.
 */
export function usePanelLayer(open: boolean, close: () => void, trigger: RefObject<HTMLElement | null>, container: RefObject<HTMLElement | null>) {
  const stack = useContext(LayerContext);
  const closeRef = useRef(close);
  closeRef.current = close;
  useEffect(() => {
    if (!open || !stack) return;
    return stack.push({ close: () => closeRef.current(), trigger });
  }, [open, stack, trigger]);
  const wasOpen = useRef(open);
  // Layout effect: runs before the browser paints the closed state, while the
  // layer's DOM is already gone, so focus never lands on <body> visibly.
  useLayoutEffect(() => {
    if (wasOpen.current && !open) {
      const active = typeof document !== 'undefined' ? document.activeElement : null;
      const lost = !active || active === document.body || !active.isConnected || (container.current?.contains(active) ?? false);
      if (lost) trigger.current?.focus({ preventScroll: true });
    }
    wasOpen.current = open;
  }, [open, trigger, container]);
}
