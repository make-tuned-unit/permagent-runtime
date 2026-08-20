/**
 * PTY grid discipline.
 *
 * Claude Code (and any TUI) paints with CUP/CUU against the size we
 * advertised via TIOCGWINSZ. If that size is not the grid xterm is showing,
 * the TUI's status line lands on the prompt — the 2026-08-19 report of
 * "Press up to edit queued messages" rendering on top of the input.
 *
 * This module never touches PTY bytes. The stream contract stays verbatim
 * (#573 / #239): we keep the two grids equal so the TUI's cursor addressing
 * matches the cells we display.
 *
 * Two ways they diverge in this pane:
 *
 *  1. FitAddon on a 0-box. Tabs stay mounted behind `display:none`, and the
 *     workspace itself is `display:none` while another view is forward.
 *     FitAddon floors that to 2×1 (its documented minimum) and `resize_pty`
 *     told the live TUI to paint in one row.
 *  2. JetBrains Mono loads with `font-display: swap` after the first measure.
 *     Cell metrics stay on the fallback font; glyphs render in the real one.
 *     Rows are then shorter than the ink, so adjacent TUI lines occupy the
 *     same pixels. Remeasuring and fitting after `document.fonts` is ready
 *     is the public way to make CharSizeService catch up.
 */

export const FALLBACK_PTY_GRID = { cols: 80, rows: 24 } as const;

export function containerCanFit(
  el: { offsetWidth: number; offsetHeight: number } | null | undefined,
): boolean {
  return !!el && el.offsetWidth > 0 && el.offsetHeight > 0;
}

/**
 * The size we are willing to send to `resize_pty` / `spawn_pty_session`.
 *
 * FitAddon floors a zero box to 2×1. Advertising that is how a status line
 * is painted on top of the prompt. Reject it; the caller falls back to the
 * last good size (or {@link FALLBACK_PTY_GRID} at spawn).
 */
export function advertisedGrid(term: { cols: number; rows: number }): { cols: number; rows: number } | null {
  if (term.cols < 2 || term.rows < 2) return null;
  return { cols: term.cols, rows: term.rows };
}

export function fitVisibleTerminal(
  fitAddon: { fit: () => void } | null | undefined,
  el: { offsetWidth: number; offsetHeight: number } | null | undefined,
): boolean {
  if (!fitAddon || !containerCanFit(el)) return false;
  try {
    fitAddon.fit();
    return true;
  } catch {
    return false;
  }
}

/**
 * Force xterm to remeasure after a webfont swap. Re-assigning `fontFamily`
 * is the public API; a no-op set still invalidates CharSizeService. We never
 * inject characters into the buffer.
 */
export function remeasureXterm(term: { options: { fontFamily?: string } }): void {
  const family = term.options.fontFamily;
  if (family === undefined) return;
  term.options.fontFamily = family;
}

/**
 * Run `cb` once fonts are ready, and again if a later face finishes loading
 * (the swap of JetBrains Mono). Cancelled on unmount so a late `fonts.ready`
 * cannot fit a disposed terminal.
 */
export function subscribeTerminalFonts(cb: () => void): () => void {
  const fonts = typeof document !== 'undefined' ? document.fonts : undefined;
  if (!fonts) return () => {};
  let alive = true;
  const run = () => {
    if (alive) cb();
  };
  void fonts.ready.then(run);
  const target = fonts as unknown as {
    addEventListener?: (type: string, listener: () => void) => void;
    removeEventListener?: (type: string, listener: () => void) => void;
  };
  target.addEventListener?.('loadingdone', run);
  return () => {
    alive = false;
    target.removeEventListener?.('loadingdone', run);
  };
}
