/**
 * A human label for a Recent row.
 *
 * `RecentSession.title` is the session's `name` passed through
 * `truncate(name, 40)` by dashboard.rs (or `Session <id>` when unnamed). A
 * session started from a tool call or a scripted run is named after the call's
 * arguments, so the name is an object literal, and after the cut it is not
 * even parseable: Home showed `{name: 'scroll_depth', depth: ...` as a title.
 *
 * The API carries nothing else to name the row by (id, title, state,
 * ended_at), so the label is recovered from the blob itself: the value of a
 * naming key (`name`, `tool`, `goal`, `title`, …) if one is there, else the
 * first quoted string value, else a plain "Untitled session". Only a title that
 * looks like a serialized object or array is touched; ordinary titles,
 * including ones that merely contain a brace, pass through unchanged.
 */

const NAMING_KEYS = ['name', 'tool', 'tool_name', 'goal', 'title', 'task', 'action', 'command', 'query'];

/** An object or array literal: `{…` / `[…` with a key or value right after. */
function looksSerialized(title: string): boolean {
  const t = title.trim();
  return /^\{\s*["']?[\w$-]+["']?\s*:/.test(t) || /^\[\s*[{["'\d]/.test(t);
}

/** `scroll_depth` / `scrollDepth` / `scroll-depth` → `Scroll depth`. */
function humanize(raw: string): string {
  const words = raw
    .replace(/([a-z\d])([A-Z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (!words) return '';
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** A string value, quoted or a bare identifier, for `key` in an object literal. */
function valueFor(blob: string, key: string): string | null {
  const re = new RegExp(`["']?\\b${key}\\b["']?\\s*:\\s*(?:"([^"]+)"|'([^']+)'|([A-Za-z_][\\w.-]*))`);
  const m = re.exec(blob);
  const v = m?.[1] ?? m?.[2] ?? m?.[3];
  if (!v || v === 'null' || v === 'undefined' || v === 'true' || v === 'false') return null;
  return v;
}

export const UNTITLED_SESSION = 'Untitled session';

export function recentTitle(title: string): string {
  const t = title.trim();
  if (!t) return UNTITLED_SESSION;
  if (!looksSerialized(t)) return t;
  for (const key of NAMING_KEYS) {
    const v = valueFor(t, key);
    if (v) return humanize(v) || UNTITLED_SESSION;
  }
  // No naming key: the first quoted string VALUE (after a colon, so a quoted
  // key is not mistaken for one). The cut may have left its closing quote
  // off, so an unterminated run up to the end counts too.
  const quoted = /:\s*(?:"([^"]{2,})"?|'([^']{2,})'?)/.exec(t);
  const first = quoted?.[1] ?? quoted?.[2];
  if (first) {
    const label = humanize(first.replace(/\.{3}$|…$/, ''));
    if (label) return label;
  }
  return UNTITLED_SESSION;
}
