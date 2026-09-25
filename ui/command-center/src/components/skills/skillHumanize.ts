/**
 * Readable names for skills that were saved from a repeated tool call.
 *
 * A saved skill's name and description come straight from the task log's
 * description of the call (`tool_task_description` in agents/agent.rs):
 * `"<tool>: <key>=<first string arg>"`, `"<tool>: <command>"` or `"<tool>"`,
 * already privacy-redacted, then slugged into the name
 * (`"orchestrator__goal_status:-card_id=[redacted]"`). The Learned section
 * rendered those verbatim — 37 cards titled "copy_to_clipboard:-te…".
 *
 * This derives a sentence from the tool + its one argument, and leaves a
 * skill the user named themselves alone.
 */

export interface HumanizableSkill {
  id: string;
  name: string;
  description?: string | null;
  toolUsed?: string | null;
  usageCount?: number;
}

export interface CallSignature {
  /** Full tool name as called, e.g. `orchestrator__goal_status`. */
  tool: string;
  /** Extension prefix (`orchestrator`), when the tool is namespaced. */
  extension: string | null;
  /** The operation (`goal_status`). */
  op: string;
  argKey: string | null;
  argValue: string | null;
  /** The argument was redacted before it was logged. */
  redacted: boolean;
}

const TOOL_TOKEN = /^([A-Za-z0-9][\w.-]*)(?::\s*([\s\S]*))?$/;
const REDACTED = /\[redacted\]/i;

function splitTool(tool: string): { extension: string | null; op: string } {
  const i = tool.indexOf('__');
  return i > 0 ? { extension: tool.slice(0, i), op: tool.slice(i + 2) } : { extension: null, op: tool };
}

/**
 * Parse a call-signature string. Accepts the description form
 * (`"write: path=/a/b"`) and the slugged name form (`"write:-path=/a/b"`).
 * Returns null for anything that reads as prose ("Summarize my downloads").
 */
export function parseCallSignature(text: string | null | undefined, toolUsed?: string | null): CallSignature | null {
  if (!text) return null;
  const m = TOOL_TOKEN.exec(text.trim());
  if (!m) return null;
  const tool = m[1];
  const rest = m[2]?.replace(/^-+/, '').trim() ?? null;
  const matchesToolUsed = !!toolUsed && tool.toLowerCase() === toolUsed.toLowerCase();
  // Tool names are lower-case identifiers; "Note: buy milk" is prose.
  const looksLikeTool = matchesToolUsed || (/^[a-z0-9][a-z0-9_.-]*$/.test(tool) && (rest !== null || tool.includes('_')));
  if (!looksLikeTool) return null;
  const { extension, op } = splitTool(matchesToolUsed ? toolUsed! : tool);
  let argKey: string | null = null;
  let argValue: string | null = rest || null;
  const kv = rest ? /^([A-Za-z_][\w-]*)=([\s\S]*)$/.exec(rest) : null;
  if (kv) { argKey = kv[1]; argValue = kv[2].trim() || null; }
  else if (rest) argKey = 'command';
  return {
    tool: matchesToolUsed ? toolUsed! : tool,
    extension, op, argKey, argValue,
    redacted: !!argValue && REDACTED.test(argValue),
  };
}

/** Plain-words verbs for the tools people actually repeat. Keyed by the operation. */
const VERBS: Record<string, string> = {
  copy_to_clipboard: 'Copy text to clipboard',
  goal_status: "Check a goal's status",
  goal_create: 'Create a goal',
  goal_update: 'Update a goal',
  write: 'Write a file',
  write_file: 'Write a file',
  read: 'Read a file',
  read_file: 'Read a file',
  edit: 'Edit a file',
  edit_file: 'Edit a file',
  text_editor: 'Edit a file',
  shell: 'Run a shell command',
  bash: 'Run a shell command',
  run_command: 'Run a shell command',
  delegate: 'Delegate a task',
  answer_decisions: 'Answer decisions',
  list_files: 'List files',
  ls: 'List files',
  glob: 'Find files',
  grep: 'Search file contents',
  search: 'Search',
  web_search: 'Search the web',
  fetch: 'Fetch a web page',
  screenshot: 'Take a screenshot',
  remember_memory: 'Save a memory',
  retrieve_memories: 'Recall memories',
  load_skill: 'Load a skill',
  todo_write: 'Update the to-do list',
};

const PATH_KEYS = new Set(['path', 'file', 'file_path', 'filepath', 'dir', 'directory', 'cwd', 'target']);

function sentence(snake: string): string {
  const words = snake.replace(/[_-]+/g, ' ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : snake;
}

/**
 * The one lookup this module does for a tool's plain-words verb. Kept as its
 * own function — not inlined into `toolVerb` — so it is the single call site
 * to redirect at merge time: dagk/ux-chat added `lib/toolLabel.ts`, a shared
 * tool-name humanizer, and this VERBS map duplicates part of that job. Once
 * both lanes land, replace the body with `toolLabel(op)` (or fold `VERBS`
 * into that module) here ONLY — nothing else in this file, or in
 * LearnedList.tsx, needs to change.
 */
function knownToolVerb(op: string): string | null {
  return VERBS[op.toLowerCase()] ?? null;
}

/** "Write a file" style base verb for a tool — the group heading. */
export function toolVerb(sig: Pick<CallSignature, 'op' | 'extension'>): string {
  const known = knownToolVerb(sig.op);
  if (known) return known;
  const base = sentence(sig.op);
  return sig.extension ? `${base} (${sentence(sig.extension).toLowerCase()})` : base;
}

/** The last two directories of a path: `/Users/j/dev/permagent-runtime/docs/x.md` → `permagent-runtime/docs`. */
function folderOf(path: string): string | null {
  const parts = path.replace(/\/+$/, '').split('/').filter(Boolean);
  if (parts.length < 2) return null;
  const dirs = parts.slice(0, -1);
  return dirs.slice(-2).join('/');
}

export interface HumanSkill {
  /** The readable title ("Write a file in permagent-runtime/docs"). */
  title: string;
  /** What distinguishes this pattern inside its group ("in permagent-runtime/docs"), or null. */
  variant: string | null;
  /** Group heading — the tool's verb, or the skill's own name when user-named. */
  groupLabel: string;
  /** Stable grouping key. */
  groupKey: string;
  /** The raw signature, for the detail view; null when the skill was user-named. */
  raw: string | null;
}

export function humanizeSkill(skill: HumanizableSkill): HumanSkill {
  const sig = parseCallSignature(skill.description, skill.toolUsed) ?? parseCallSignature(skill.name, skill.toolUsed);
  if (!sig) {
    return { title: skill.name, variant: null, groupLabel: skill.name, groupKey: `skill:${skill.id}`, raw: null };
  }
  const verb = toolVerb(sig);
  let variant: string | null = null;
  if (sig.argValue && !sig.redacted) {
    const key = sig.argKey?.toLowerCase() ?? '';
    if (PATH_KEYS.has(key) || (sig.argValue.includes('/') && key !== 'command')) {
      const folder = folderOf(sig.argValue);
      variant = folder ? `in ${folder}` : sig.argValue;
    } else if (key === 'command') {
      variant = `“${sig.argValue}”`;
    } else {
      variant = `${sig.argKey}: ${sig.argValue}`;
    }
  }
  return {
    title: variant ? `${verb} ${variant}` : verb,
    variant,
    groupLabel: verb,
    groupKey: `tool:${verb}`,
    raw: (skill.description?.trim() || skill.name),
  };
}

export interface SkillGroup<S extends HumanizableSkill> {
  key: string;
  label: string;
  items: Array<{ skill: S; human: HumanSkill }>;
  /** Sum of `usageCount` over the group. */
  matches: number;
}

/** Group by tool verb, busiest group first; within a group, busiest pattern first. */
export function groupSkills<S extends HumanizableSkill>(skills: S[]): SkillGroup<S>[] {
  const groups = new Map<string, SkillGroup<S>>();
  for (const skill of skills) {
    const human = humanizeSkill(skill);
    let g = groups.get(human.groupKey);
    if (!g) { g = { key: human.groupKey, label: human.groupLabel, items: [], matches: 0 }; groups.set(human.groupKey, g); }
    g.items.push({ skill, human });
    g.matches += skill.usageCount ?? 0;
  }
  const out = [...groups.values()];
  for (const g of out) g.items.sort((a, b) => (b.skill.usageCount ?? 0) - (a.skill.usageCount ?? 0));
  return out.sort((a, b) => b.matches - a.matches || b.items.length - a.items.length || a.label.localeCompare(b.label));
}

/**
 * What `usageCount` actually counts. It is `COUNT(*)` of `skill_executions`
 * (crates/goose/src/skills.rs), and those rows are written (a) each time the
 * agent loads the saved skill and (b) by the repetition sweep that runs after
 * every completed task, once per sweep while the same call pattern is still
 * recurring. So it measures how often the pattern keeps showing up — not
 * mastery, and not even distinct uses. The old MASTERED/TRUSTED/LEARNED tiers
 * read that number as competence.
 */
export function matchesLabel(n: number): string {
  return `matched ${n}×`;
}

export const MATCHES_HINT =
  'How many times this same call pattern was seen again after it was saved (or the skill was loaded). It measures repetition, not how well it works.';
