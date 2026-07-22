import type { RefObject } from 'react';
import { useCallback, useEffect, useRef } from 'react';

/** Inputs inside a pane own Tab for completion/focus navigation. */
export function isPaneInputFocused(pane: HTMLElement, activeElement: Element | null): boolean {
  if (!activeElement || !pane.contains(activeElement)) return false;

  const element = activeElement as HTMLElement;
  return element.matches('input, textarea, select, [contenteditable="true"], [role="textbox"]');
}

export function nextPaneTabId(
  tabIds: readonly string[],
  activeTabId: string,
  backwards = false,
): string | null {
  if (tabIds.length < 2) return null;
  const activeIndex = tabIds.indexOf(activeTabId);
  const start = activeIndex < 0 ? 0 : activeIndex;
  const offset = backwards ? -1 : 1;
  return tabIds[(start + offset + tabIds.length) % tabIds.length];
}

export interface PaneTabKeydownState {
  paneSelected: boolean;
  pane: HTMLElement;
  activeElement: Element | null;
}

let selectedPaneId: string | null = null;

/** Pure decision seam for the two states pinned by #258. */
export function shouldCyclePaneTabs(event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey'>, state: PaneTabKeydownState): boolean {
  return event.key === 'Tab'
    && !event.metaKey
    && !event.ctrlKey
    && !event.altKey
    && state.paneSelected
    && !isPaneInputFocused(state.pane, state.activeElement);
}

/**
 * Tracks selection independently from input focus. Clicking anywhere in this
 * pane selects it; focusing its terminal/address input still leaves Tab alone.
 */
export function usePaneTabCycling(
  paneId: 'terminal' | 'browser',
  paneRef: RefObject<HTMLElement>,
  cycle: (backwards: boolean) => void,
) {
  const selectedRef = useRef(false);

  const selectPane = useCallback(() => {
    selectedPaneId = paneId;
    selectedRef.current = true;
  }, [paneId]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const pane = paneRef.current;
      if (!pane) return;
      if (pane.contains(event.target as Node)) {
        selectedPaneId = paneId;
        selectedRef.current = true;
      } else if (selectedPaneId === paneId) {
        selectedPaneId = null;
        selectedRef.current = false;
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      const pane = paneRef.current;
      if (!pane || !shouldCyclePaneTabs(event, {
        paneSelected: selectedRef.current && selectedPaneId === paneId,
        pane,
        activeElement: document.activeElement,
      })) return;

      event.preventDefault();
      cycle(event.shiftKey);
    };

    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [cycle, paneId, paneRef]);

  return selectPane;
}
