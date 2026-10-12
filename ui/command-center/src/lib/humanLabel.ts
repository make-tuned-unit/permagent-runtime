/**
 * Readable titles for identifiers the runtime hands us as labels: event ids
 * (`config_changed`, `activity:terminal_session_started`), schedule ids
 * (`model_review`, `workspace-snapshot`), tool names (`ext__readWebpage`) and
 * logged tool calls with their arguments attached (`search: query=...`).
 *
 * The raw id never disappears: callers keep it for a Tooltip (or a secondary
 * line). This only decides what reads as the headline. Pure; never throws and
 * never returns an empty string for a non-empty input.
 */

/** Default length of the argument preview that may follow a headline. */
export const LABEL_PREVIEW_MAX = 48;

/** Collapse whitespace and cut to `max` characters with an ellipsis. */
export function clampPreview(text: string, max: number = LABEL_PREVIEW_MAX): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, Math.max(1, max - 1)).trimEnd()}…`;
}

function words(segment: string): string[] {
  return segment
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .split(/[_\-\s.]+/)
    .filter(Boolean)
    .map(w => (/^[A-Z0-9]{2,}$/.test(w) ? w : w.toLowerCase()));
}

/** `config_changed` → "Config changed"; `activity:terminal_session_started` →
 *  "Activity: terminal session started"; `ext__readWebpage` → "Read webpage". */
export function humanizeId(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return raw;
  // An `ext__tool` gateway prefix is the MCP gateway's naming, not the tool's.
  const noExt = trimmed.includes('__') ? trimmed.slice(trimmed.lastIndexOf('__') + 2) || trimmed : trimmed;
  const parts = noExt.split(':').map(words).filter(p => p.length > 0);
  if (parts.length === 0) return raw;
  const text = parts.map(p => p.join(' ')).join(': ');
  return text[0].toUpperCase() + text.slice(1);
}

/** Identifier head followed by an argument dump: `tool: key=value`, `tool key=value`. */
const CALL_WITH_ARGS = /^([A-Za-z][\w.-]*(?:__[\w.-]+)?)(?::\s+|\s+(?=\w+=))(.+)$/s;
/** A bare identifier: no spaces, at least one separator or inner capital. */
const BARE_ID = /^[A-Za-z0-9][\w.:-]*$/;

/**
 * A display label for anything that might be a raw id or call dump.
 * Prose ("Weekly Storage Report") passes through, only clamped. Returns the
 * raw text separately so callers can show it on hover.
 */
export function humanLabel(raw: string | null | undefined, previewMax: number = LABEL_PREVIEW_MAX): { label: string; raw: string; changed: boolean } {
  const text = (raw ?? '').trim();
  if (!text) return { label: '', raw: text, changed: false };
  let label: string;
  const call = CALL_WITH_ARGS.exec(text);
  if (call && /[_\-]|[a-z][A-Z]/.test(call[1])) {
    label = `${humanizeId(call[1])}: ${clampPreview(call[2], previewMax)}`;
  } else if (BARE_ID.test(text) && /[_\-:]|[a-z][A-Z]/.test(text)) {
    label = humanizeId(text);
  } else {
    label = clampPreview(text, Math.max(previewMax, 80));
  }
  return { label, raw: text, changed: label !== text };
}
