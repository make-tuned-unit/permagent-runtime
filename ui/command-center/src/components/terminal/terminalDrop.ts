/**
 * Formatting for drop-to-CC-terminal (#557).
 *
 * When a file is dropped on the Build-tab terminal pane, its filesystem path is
 * injected into the running session's PTY as if typed — e.g. a live Claude Code
 * session receives the path in its input. Paths must be shell-safe: a space or
 * shell metacharacter would otherwise split the argument or be interpreted.
 */

/** Characters that are always safe unquoted in a POSIX shell word. */
const SHELL_SAFE = /^[A-Za-z0-9_@%+=:,./-]+$/;

/**
 * Quote a single path for injection into a shell input line. Safe words pass
 * through untouched; anything else is single-quoted with embedded single quotes
 * escaped the POSIX way (`'\''`), which is robust against spaces, `$`, `"`,
 * backticks, globs, and newlines.
 */
export function shellQuotePath(path: string): string {
  if (SHELL_SAFE.test(path)) return path;
  return `'${path.replace(/'/g, `'\\''`)}'`;
}

/**
 * Build the string to write into the PTY for a set of dropped paths. Multiple
 * paths are space-separated; a trailing space is appended so the cursor sits
 * ready for the next token (and no newline is sent — the drop injects the path,
 * it does not submit the prompt). Returns '' for an empty drop.
 */
export function formatDroppedPathsForInput(paths: readonly string[]): string {
  const cleaned = paths.filter(p => p && p.length > 0);
  if (cleaned.length === 0) return '';
  return cleaned.map(shellQuotePath).join(' ') + ' ';
}

/** Minimal terminal-tab shape needed to resolve a drop injection. */
export interface DropInjectionTab {
  id: string;
  sessionId: string | null;
}

/** What to write, and into which PTY session, for a terminal-pane file drop. */
export interface PtyInjection {
  sessionId: string;
  data: string;
}

/**
 * Resolve a terminal-pane drop to a concrete PTY write: the ACTIVE tab's live
 * session receives the formatted paths. Returns null when the active tab has no
 * spawned session yet, or when there is nothing to inject — the drop is a no-op
 * rather than an error in those cases.
 */
export function resolvePtyInjection(
  tabs: readonly DropInjectionTab[],
  activeTabId: string,
  paths: readonly string[],
): PtyInjection | null {
  const active = tabs.find(t => t.id === activeTabId);
  if (!active?.sessionId) return null;
  const data = formatDroppedPathsForInput(paths);
  if (!data) return null;
  return { sessionId: active.sessionId, data };
}
