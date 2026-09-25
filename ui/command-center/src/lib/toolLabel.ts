/**
 * R3 (dagK UX standard, 2026-09-25): a tool card showed the wire identifier
 * verbatim — `card_create`, `gmail__search`, `developer__shell` — which is
 * exactly the internal-jargon-in-user-copy the rule names. `toolLabel()` maps
 * the ~40 tools a user actually sees often to a plain-English label the
 * override was written by hand for (read off the tool registrations in
 * crates/goose/src/agents/platform_extensions/*.rs and the Gmail/bash cases
 * ToolResult.tsx already special-cased), and falls back to a mechanical
 * cleanup for everything else. The raw name never disappears — callers keep
 * it on hover/in the expanded detail (ToolResult.tsx) — this only changes
 * what reads as the headline.
 *
 * Exported from lib/ (not components/tool-results/) so Automate's Learned
 * list can reuse it for the same tools without importing a chat component.
 */

/** Hand-named because a mechanical split cannot recover "a card" from
 *  `card_create`, or that `gmail__search` means the Gmail extension rather
 *  than a literal double underscore. Extend this map before reaching for the
 *  fallback — a wrong guess reads worse than the raw name it replaced. */
const TOOL_LABELS: Record<string, string> = {
  // Cards / projects (project_manager.rs)
  card_create: 'Create a card',
  card_move: 'Move a card',
  card_update: 'Update a card',
  card_delete: 'Delete a card',
  card_list: 'List cards',
  card_due_list: 'List cards due soon',
  column_create: 'Add a column',
  column_delete: 'Delete a column',
  board_summary: 'Summarize the board',
  project_create: 'Create a project',
  project_update: 'Update a project',
  project_delete: 'Delete a project',
  project_list: 'List projects',
  project_resolve: 'Find a project',
  project_documents_list: 'List project documents',
  project_document_read: 'Read a project document',
  research_project_intel: 'Research a project',
  propose_project_intel: 'Propose project intel',
  dismiss_project_intel: 'Dismiss project intel',
  set_project_strategy: 'Set the project strategy',
  set_project_brand: 'Set the project brand',
  social_content_brief: 'Draft a social content brief',
  retry_social_media: 'Retry a social post',
  approve_social_post: 'Approve a social post',
  publisher_status: 'Check the publisher',
  connect_project_channel: 'Connect a channel',
  disconnect_project_channel: 'Disconnect a channel',
  project_launch: 'Launch the project',

  // Apps / desktop / browser
  navigate_app: 'Go to a tab',
  app_action: 'Act in the app',
  open_item: 'Open an item',
  copy_to_clipboard: 'Copy to clipboard',
  observe_app: 'Check app state',
  create_app_content: 'Draft a new app',
  update_app_content: 'Update an app',
  list_apps: 'List apps',
  create_app: 'Create an app',
  iterate_app: 'Improve an app',
  delete_app: 'Delete an app',
  read_browser_content: 'Read the browser page',
  open_website: 'Open a website',
  wake_desktop_app: 'Open the desktop app',
  read_webpage: 'Fetch a web page',
  get_page_snapshot: 'List page elements',
  act_on_page: 'Act on the page',

  // Developer
  read: 'Read a file',
  write: 'Write a file',
  edit: 'Edit a file',
  shell: 'Run a shell command',
  tree: 'List a directory',
  search: 'Search the code',
  verify: 'Verify the build',
  run_command: 'Run a shell command',
  execute_bash: 'Run a shell command',
  execute_typescript: 'Run TypeScript',

  // Council / decisions / memory
  council_status: 'Check the Council',
  council_convene: 'Convene the Council',
  council_report: 'Read the Council report',
  list_pending_decisions: "Read the Decision Inbox",
  search_memory: 'Search memory',
  get_memory: 'Load a memory',
  describe_memory: 'Describe a memory',
  read_dashboard: 'Read the dashboard',

  // Finance
  research_ticker: 'Look up a ticker',
  company_fundamentals: 'Read company fundamentals',
  finance_board: 'Read the Finance tab',
  finance_watchlist_add: 'Add to the watchlist',
  finance_watchlist_remove: 'Remove from the watchlist',
  picker_status: 'Check the stock scanner',
  polybot_status: 'Check Polybot',

  // Forecaster
  forecaster_series: 'List forecast series',
  forecaster_forecast: 'Run a forecast',
  forecaster_brief: 'Read the market brief',

  // Models
  list_models: 'List local models',
  propose_model_upgrade: 'Propose a model upgrade',

  // Gmail (external MCP extension; ToolResult.tsx already special-cases the
  // double-underscore and single-underscore spellings for the result body)
  gmail__search: 'Search Gmail',
  gmail_search: 'Search Gmail',
  gmail__read: 'Read a Gmail message',
  gmail_read: 'Read a Gmail message',

  // File reading (already special-cased for the result body in ToolResult.tsx)
  file_read: 'Read a file',
  read_file: 'Read a file',
  read_attachment: 'Read an attachment',
  attachment_read: 'Read an attachment',
};

/** Verbs that appear as the LAST segment of an unmapped `noun_verb` tool name
 *  often enough to be worth moving to the front — Permagent's own tool naming
 *  convention (`card_create`, `column_delete`, `project_resolve`, …) puts the
 *  object first and the verb last, which is backwards from an English
 *  sentence. Not exhaustive: a name outside this set is just sentence-cased
 *  left to right, which is still better than the raw identifier. */
const TRAILING_VERBS = new Set([
  'create', 'update', 'delete', 'remove', 'add', 'list', 'read', 'write', 'edit',
  'search', 'get', 'set', 'resolve', 'start', 'stop', 'pause', 'scan', 'close',
  'recategorize', 'launch', 'connect', 'disconnect', 'dismiss', 'propose',
  'retry', 'approve', 'open', 'load', 'describe', 'forecast', 'bind',
]);

function capitalize(word: string): string {
  return word ? word[0].toUpperCase() + word.slice(1) : word;
}

/** The mechanical fallback: strip an `ext__tool` prefix (the MCP gateway's own
 *  naming, not the tool's), split snake_case, and read it as a sentence,
 *  moving a known trailing verb to the front. Never throws, never returns
 *  the empty string — an empty or unsplittable name falls back to itself. */
export function genericToolLabel(name: string): string {
  if (!name) return name;
  const afterExt = name.includes('__') ? name.slice(name.lastIndexOf('__') + 2) : name;
  const words = afterExt.split(/[_-]+/).filter(Boolean).map(w => w.toLowerCase());
  if (words.length === 0) return name;

  const last = words[words.length - 1];
  const ordered = words.length > 1 && TRAILING_VERBS.has(last)
    ? [last, ...words.slice(0, -1)]
    : words;

  return ordered.map((w, i) => (i === 0 ? capitalize(w) : w)).join(' ');
}

/** The label a tool card shows as its headline. Always succeeds; the raw
 *  `name` stays available to the caller for a tooltip or an expanded-detail
 *  line — this function only ever returns a display string, never the raw
 *  name dressed up as one unless the fallback had nothing better to do.
 *
 *  Tries the exact name first, then the name with an `ext__` gateway prefix
 *  stripped — `developer__shell` should read the same as the bare `shell`
 *  the override map already names, not fall past it into the mechanical
 *  fallback just because an extension prefixed it. */
export function toolLabel(name: string): string {
  if (TOOL_LABELS[name]) return TOOL_LABELS[name];
  if (name.includes('__')) {
    const stripped = name.slice(name.lastIndexOf('__') + 2);
    if (TOOL_LABELS[stripped]) return TOOL_LABELS[stripped];
  }
  return genericToolLabel(name);
}
