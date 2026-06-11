// W4 atmosphere — WORLD_VIEW_BIBLE.md §8 item 2.
// All workspaces mount simultaneously in WorkspaceRenderer (hidden ones get
// `display: none`), so the World canvas would otherwise keep rendering behind
// other tabs and also mounts at zero size before layout settles
// (GL_INVALID_FRAMEBUFFER_OPERATION spam). This hook reports whether the
// container actually has layout (non-zero box) so WorldView can gate the
// Canvas frameloop ('always' ↔ 'never').
//
// ResizeObserver is used instead of a store read because it is per-instance
// correct: it answers "is THIS WorldView's container visible" even if several
// workspaces contain a world panel, and it also covers the settings overlay
// and the pre-layout zero-size mount for free (display:none → 0×0 box).

import { useEffect, useState } from 'react';
import type { RefObject } from 'react';

export function useWorldVisibility(ref: RefObject<HTMLElement | null>): boolean {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[entries.length - 1].contentRect;
      setVisible(rect.width > 1 && rect.height > 1);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);

  return visible;
}
