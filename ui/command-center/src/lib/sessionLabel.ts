/**
 * The ONE place a session gets a human label, and the one place a session is
 * told apart as a conversation, an agent run or a system job.
 *
 * Every surface that names a session (History, Home's Recent, the chat's
 * session picker, search) goes through `sessionDisplayLabel`. They used to
 * print `session.name` raw, and the names the daemon had stored over time
 * were not all titles (QA walk 2026-09-24):
 *
 *   {name: 'scroll_depth', depth: 25|50|75|100, event_id: '{event-id}'}
 *                         — a code literal the old namer lifted from a prompt
 *   "Merged text content" — a quoted phrase from deep inside a health prompt
 *   Desktop — no battery  — a template fragment of a scheduled snapshot
 *   ATTENTION : CI failing on
 *   Human-Readable Summary, CLI Session, New Chat, Session <id>
 *                         — placeholders standing in for "no title yet"
 *
 * The daemon's namer (providers/base.rs `extract_short_title`) was fixed for
 * NEW sessions in 0bb7791a, but every session named before that keeps its
 * junk name. So the label is re-derived here: a name the user typed always
 * wins; otherwise the first user message is titled with the same rules the
 * daemon now uses, and the stored name is only a repaired fallback. A
 * scheduled run is named after its job (recipe title / schedule id) — its
 * prompt is a template, and a line of a template is not a title.
 */

/** What a label or a classification may be read from. All optional: the list
 *  API, the dashboard and the chat each carry a different subset, and a field
 *  an older daemon does not send is simply absent. */
export interface SessionLabelSource {
  name?: string | null;
  /** True when a person typed the name — it is shown as typed (repaired only
   *  for wrapping quotes), never re-derived over. */
  userSetName?: boolean | null;
  /** `user` | `scheduled` | `sub_agent` | `hidden` | `terminal` | `gateway` | `acp`. */
  sessionType?: string | null;
  firstUserText?: string | null;
  recipeTitle?: string | null;
  scheduleId?: string | null;
  goalId?: string | null;
  goalTitle?: string | null;
  projectName?: string | null;
}

export const UNTITLED_SESSION = 'Untitled session';

/**
 * Names that mean "nothing yet". Stored by session creation
 * (`New Chat`, the CLI's `CLI Session`, the namer's `New Session`), by the
 * dashboard's own fallback (`Session <id>`), or lifted by the old namer out of
 * a system prompt (`Merged text content`, `Human-Readable Summary`).
 */
const PLACEHOLDER_NAMES = new Set([
  'new chat',
  'new session',
  'cli session',
  'untitled',
  'untitled session',
  'merged text content',
  'merged consecutive user messages',
  'human-readable summary',
  'human readable summary',
]);

/** `Session 20260924_12` / `Session 3f2a…` — the dashboard's no-name fallback. */
const SESSION_ID_PLACEHOLDER = /^session\s+[\w-]+$/i;

/** A shouted prefix a system prompt uses to get a model's attention. It is not
 *  part of what the session is about. */
const SHOUTED_PREFIX = /^(?:attention|important|note|urgent|warning)\s*:\s*/i;

// ── object-literal names ────────────────────────────────────────────────────

const NAMING_KEYS = ['name', 'tool', 'tool_name', 'goal', 'title', 'task', 'action', 'command', 'query'];

/** An object or array literal: `{…` / `[…` with a key or value right after. */
function looksSerialized(title: string): boolean {
  const t = title.trim();
  return /^\{\s*["']?[\w$-]+["']?\s*:/.test(t) || /^\[\s*[{["'\d]/.test(t);
}

/** `scroll_depth` / `scrollDepth` / `scroll-depth` → `Scroll depth`. */
export function humanize(raw: string): string {
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

/** The label an object-literal name can be recovered to, or null. */
function labelFromLiteral(t: string): string | null {
  for (const key of NAMING_KEYS) {
    const v = valueFor(t, key);
    if (v) return humanize(v) || null;
  }
  // No naming key: the first quoted string VALUE (after a colon, so a quoted
  // key is not mistaken for one). A cut name may have lost its closing quote,
  // so an unterminated run up to the end counts too.
  const quoted = /:\s*(?:"([^"]{2,})"?|'([^']{2,})'?)/.exec(t);
  const first = quoted?.[1] ?? quoted?.[2];
  if (first) return humanize(first.replace(/\.{3}$|…$/, '')) || null;
  return null;
}

// ── repair ──────────────────────────────────────────────────────────────────

function stripWrappingQuotes(s: string): string {
  let t = s.trim();
  for (;;) {
    const a = t.charAt(0);
    const b = t.charAt(t.length - 1);
    const pair = (a === b && (a === '"' || a === "'" || a === '`'))
      || (a === '“' && b === '”') || (a === '‘' && b === '’');
    if (t.length < 2 || !pair) return t;
    t = t.slice(1, -1).trim();
  }
}

function isPlaceholder(t: string): boolean {
  return PLACEHOLDER_NAMES.has(t.toLowerCase()) || SESSION_ID_PLACEHOLDER.test(t);
}

/**
 * A stored name made presentable, or `null` when it is not a title at all
 * (blank, a placeholder, a code literal with nothing to recover).
 */
export function repairName(raw: string | null | undefined): string | null {
  if (!raw) return null;
  let t = stripWrappingQuotes(raw.replace(/\s+/g, ' '));
  if (!t) return null;
  if (looksSerialized(t)) return labelFromLiteral(t);
  // Markdown emphasis and a shouted prefix are prompt furniture.
  t = t.replace(/\*\*|__/g, '').replace(/\s+:/g, ':').replace(SHOUTED_PREFIX, '').trim();
  t = stripWrappingQuotes(t);
  if (!t || isPlaceholder(t)) return null;
  return t;
}

// ── a title from the first message (mirrors providers/base.rs) ──────────────

/** The default cap for a compact surface (Home Recent, the chat session
 *  picker) — one line, mirrors the Rust namer's own limit so an old,
 *  client-repaired title and a new daemon-named one read the same length. A
 *  roomier surface (History) passes a bigger `maxChars` — up to `Infinity` —
 *  so the full label survives instead of being cut with nothing on screen to
 *  justify it; the caller wraps it in CSS instead (2-line clamp) and shows the
 *  untruncated text on hover only if it actually clamped. */
export const TITLE_MAX_CHARS = 80;

function isHeading(line: string): boolean {
  if (!line.startsWith('#')) return false;
  const rest = line.replace(/^#+/, '');
  return rest === '' || rest.startsWith(' ');
}

function isBoilerplate(line: string): boolean {
  const lower = line.toLowerCase();
  return isHeading(line)
    || lower.startsWith('you are ')
    || lower.startsWith('working directory:')
    || lower.startsWith('```')
    || /^[-=*_]+$/.test(line);
}

function cutAtFirstSentence(s: string): string {
  const abbreviations = ['e.g.', 'i.e.', 'etc.', 'vs.', 'mr.', 'mrs.', 'dr.'];
  const re = /[.!?](?=\s)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(s))) {
    const head = s.slice(0, m.index + 1);
    const words = head.split(/\s+/).filter(Boolean);
    const last = words[words.length - 1] ?? '';
    const stem = last.replace(/[.!?]+$/, '');
    if (m[0] === '.' && (abbreviations.includes(last.toLowerCase()) || stem.length <= 1 || /^\d+$/.test(stem))) continue;
    if (words.length < 2) continue;
    return m[0] === '.' ? s.slice(0, m.index) : head;
  }
  return s;
}

function capChars(s: string, max: number): string {
  const chars = Array.from(s);
  if (chars.length <= max) return s;
  const prefix = chars.slice(0, max).join('');
  const ws = prefix.search(/\s\S*$/);
  const cut = ws > 0 ? prefix.slice(0, ws) : prefix;
  return `${cut.replace(/[\s,:;\-–—→([]+$/, '')}…`;
}

function shortenLine(line: string, maxChars: number): string {
  let s = line.trim().replace(/^[-*+]\s+/, '').replace(/^\d+[.)]\s+/, '');
  s = stripWrappingQuotes(s);
  // Cutting at the first sentence end is a content rule (a title is one
  // thought, not the paragraph after it) and applies at every length; only
  // the hard character cap is what a roomier surface opts out of.
  return capChars(cutAtFirstSentence(s), maxChars);
}

/** Starts with a letter or digit, has letters, and is mostly alphanumeric —
 *  never a brace/punctuation-heavy code literal. */
function isMeaningful(title: string): boolean {
  if (!/^[\p{L}\p{N}]/u.test(title)) return false;
  const letters = (title.match(/\p{L}/gu) ?? []).length;
  const visible = title.replace(/\s/g, '').length;
  const alnum = (title.match(/[\p{L}\p{N}]/gu) ?? []).length;
  return letters >= 2 && alnum * 10 >= visible * 6;
}

/**
 * A short title from (possibly long, multi-line) user text: the first line of
 * a `## Task` section or a `Goal:` / `Task:` line; otherwise the first line
 * that is not a "You are …" preamble, a working directory or a heading. Each
 * candidate is cut at its first sentence end and, unless the caller passes a
 * bigger `maxChars` (History passes `Infinity`), at `TITLE_MAX_CHARS`.
 */
export function titleFromMessage(text: string | null | undefined, maxChars: number = TITLE_MAX_CHARS): string | null {
  if (!text) return null;
  const stripped = text.replace(/<([a-zA-Z][\w]*)[^>]*>[\s\S]*?<\/[a-zA-Z][\w]*>/g, '').replace(/<\/?[a-zA-Z][\w]*[^>]*>/g, '');
  const lines = stripped.split('\n').map(l => l.split(/\s+/).filter(Boolean).join(' '));
  const candidates: string[] = [];
  const task = lines.findIndex(l => isHeading(l) && /^task:?$/i.test(l.replace(/^#+/, '').trim()));
  if (task >= 0) {
    for (const l of lines.slice(task + 1)) {
      if (isHeading(l)) break;
      if (l) candidates.push(l);
    }
  }
  lines.forEach((l, i) => {
    const m = /^(?:goal|task):\s*(.*)$/i.exec(l);
    if (!m) return;
    if (m[1]) candidates.push(m[1]);
    else {
      const next = lines.slice(i + 1).find(Boolean);
      if (next && !isHeading(next)) candidates.push(next);
    }
  });
  candidates.push(...lines.filter(l => l && !isBoilerplate(l)));
  for (const c of candidates) {
    const t = shortenLine(c, maxChars);
    if (isMeaningful(t) && !isPlaceholder(t)) return t;
  }
  return null;
}

// ── classification ──────────────────────────────────────────────────────────

export type SessionKind = 'conversation' | 'agent_run' | 'system';

/** The coding-harness recipe (tool_inspection.rs `CODING_HARNESS_RECIPE_TITLE`). */
const CODING_HARNESS_RECIPE = 'permagent coding harness';

/** Liveness probes and smoke checks: "Reply with exactly the word SHIPPED and
 *  nothing else. Do not call any tools." — a machine checking the pipe, not a
 *  person asking something. */
const PROBE = /^(?:reply|respond|answer|say)\b[^\n]{0,40}\b(?:exactly|only)\b/i;
const NO_TOOLS_PROBE = /\bdo not (?:call|use) any tools\b/i;

/** The Grow → coding-agent directive (grow/codingAgentDirective.ts). */
const CODING_DIRECTIVE = /^you are a coding agent\b/i;
const DIRECTIVE_PROJECT = /working in the repository for ["“]([^"”]+)["”]/i;

function isProbe(text: string | null | undefined): boolean {
  const t = text?.trim();
  if (!t) return false;
  return PROBE.test(t) || (t.length < 240 && NO_TOOLS_PROBE.test(t));
}

/**
 * Conversation (a person talking to the agent), agent run (a coding worker or
 * goal worker doing a job) or system (scheduled jobs, probes, hidden helpers).
 * Real fields decide first — session type, schedule, goal link, recipe — and
 * the first message's shape only decides when none of them do.
 */
export function classifySession(s: SessionLabelSource): SessionKind {
  const type = (s.sessionType ?? '').toLowerCase();
  if (type === 'scheduled' || s.scheduleId || type === 'hidden') return 'system';
  const first = s.firstUserText ?? null;
  if (isProbe(first) || (!first && isProbe(s.name))) return 'system';
  if (type === 'sub_agent' || type === 'acp' || s.goalId) return 'agent_run';
  if ((s.recipeTitle ?? '').trim().toLowerCase() === CODING_HARNESS_RECIPE) return 'agent_run';
  if (CODING_DIRECTIVE.test((first ?? s.name ?? '').trim())) return 'agent_run';
  return 'conversation';
}

/** The project an agent run belongs to: the daemon's own link, else the name
 *  the coding directive spells out. */
export function sessionProjectName(s: SessionLabelSource): string | null {
  const own = s.projectName?.trim();
  if (own) return own;
  const m = DIRECTIVE_PROJECT.exec(s.firstUserText ?? s.name ?? '');
  return m?.[1]?.trim() || null;
}

// ── the label ───────────────────────────────────────────────────────────────

export interface LabelOptions {
  /** The hard character cap for a label derived from the first message.
   *  Defaults to `TITLE_MAX_CHARS` (one line, for Home Recent and the chat
   *  session picker). History passes `Infinity`: the row is roomy enough to
   *  show the whole label, wrapped with CSS line-clamp instead of cut with
   *  free space still on the row (R3). A name the user actually typed, or a
   *  repaired stored name, is never capped by this option — only a title
   *  derived from the first message is. */
  maxChars?: number;
}

/** A human label for any session, from whatever the caller has. Never empty,
 *  never a literal, never a placeholder. */
export function sessionDisplayLabel(s: SessionLabelSource, opts?: LabelOptions): string {
  const name = repairName(s.name);
  if (s.userSetName && name) return name;

  const type = (s.sessionType ?? '').toLowerCase();
  if (type === 'scheduled' || s.scheduleId) {
    const job = repairName(s.recipeTitle) ?? (s.scheduleId ? humanize(s.scheduleId) : null);
    if (job) return job;
  }

  const fromMessage = titleFromMessage(s.firstUserText, opts?.maxChars);
  return fromMessage
    ?? name
    ?? repairName(s.goalTitle)
    ?? repairName(s.recipeTitle)
    ?? sessionProjectName(s)
    ?? UNTITLED_SESSION;
}

/** The text a search box matches: the label and the first message. */
export function sessionSearchText(s: SessionLabelSource): string {
  return [sessionDisplayLabel(s), s.name ?? '', s.firstUserText ?? '', sessionProjectName(s) ?? '', s.goalTitle ?? '']
    .join('\n')
    .toLowerCase();
}

/** A session record as the daemon's APIs spell it (snake_case). */
export interface SessionRecordLike {
  name?: string | null;
  user_set_name?: boolean | null;
  session_type?: string | null;
  first_user_text?: string | null;
  recipe_title?: string | null;
  schedule_id?: string | null;
  goal_id?: string | null;
  goal_title?: string | null;
  project_name?: string | null;
}

/** Read a daemon session record as a label source. */
export function labelSource(r: SessionRecordLike): SessionLabelSource {
  return {
    name: r.name,
    userSetName: r.user_set_name,
    sessionType: r.session_type,
    firstUserText: r.first_user_text,
    recipeTitle: r.recipe_title,
    scheduleId: r.schedule_id,
    goalId: r.goal_id,
    goalTitle: r.goal_title,
    projectName: r.project_name,
  };
}
