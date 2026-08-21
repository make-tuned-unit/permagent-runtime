import { execFile } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export interface SlashCommandDef {
  name: string;
  aliases: string[];
  summary: string;
  usage: string;
  keys?: string;
}

export const SLASH_COMMANDS: SlashCommandDef[] = [
  {
    name: "model",
    aliases: [],
    summary: "Switch the model (picker, or /model <name>)",
    usage: "/model [name]",
    keys: "^M",
  },
  {
    name: "mode",
    aliases: ["trust"],
    summary: "Permission mode: auto, ask, chat (shift+tab cycles)",
    usage: "/mode [auto|ask|chat]",
    keys: "⇧⇥",
  },
  {
    name: "autonomous",
    aliases: ["auto"],
    summary: "Keep working after each turn until a cap or gate",
    usage: "/autonomous on [turns] | off | status | gate <cmd>",
  },
  {
    name: "provider",
    aliases: ["providers"],
    summary: "Switch the provider",
    usage: "/provider",
    keys: "^P",
  },
  {
    name: "extensions",
    aliases: ["mcp", "exts"],
    summary: "Manage extensions / MCP servers",
    usage: "/extensions",
    keys: "^E",
  },
  {
    name: "help",
    aliases: ["?", "keys", "keymap", "shortcuts"],
    summary: "Show slash commands and keybindings",
    usage: "/help",
  },
  {
    name: "status",
    aliases: ["session"],
    summary: "Show project, model, session, and usage",
    usage: "/status",
  },
  {
    name: "usage",
    aliases: [],
    summary: "Token estimate, mode, and autonomous caps",
    usage: "/usage",
  },
  {
    name: "copy",
    aliases: [],
    summary: "Copy the last assistant message",
    usage: "/copy",
  },
  {
    name: "export",
    aliases: [],
    summary: "Write this session to a markdown file",
    usage: "/export [file]",
  },
  {
    name: "clear",
    aliases: ["new", "reset"],
    summary: "Start a new conversation",
    usage: "/clear",
  },
  {
    name: "compact",
    aliases: [],
    summary: "Summarize history to free context",
    usage: "/compact",
  },
  {
    name: "diff",
    aliases: [],
    summary: "Show uncommitted git changes",
    usage: "/diff",
  },
  {
    name: "cd",
    aliases: [],
    summary: "Change the session working directory",
    usage: "/cd <path>",
  },
  {
    name: "config",
    aliases: ["settings"],
    summary: "Open provider and model settings",
    usage: "/config",
  },
  {
    name: "quit",
    aliases: ["exit", "q"],
    summary: "Exit the TUI",
    usage: "/quit",
    keys: "^C",
  },
];

export const KEYBINDINGS: { keys: string; summary: string }[] = [
  { keys: "/", summary: "Open slash commands" },
  { keys: "tab", summary: "Complete slash command, @ file, or expand tools" },
  { keys: "esc", summary: "Hard-stop: cancel the turn, drop the queue, disable autonomous" },
  { keys: "ctrl+c", summary: "Hard-stop, or quit when idle" },
  { keys: "ctrl+d", summary: "Quit (same as ctrl+c)" },
  { keys: "enter", summary: "Send, or steer (cancel remaining work) while busy" },
  { keys: "shift+enter", summary: "Insert a newline (ctrl+enter also works)" },
  { keys: "alt+enter", summary: "Queue a follow-up for after this turn" },
  { keys: "alt+↑", summary: "Pull the last queued message back to the composer" },
  { keys: "@", summary: "Mention a project file" },
  { keys: "!cmd", summary: "Run a shell command and send the output" },
  { keys: "!!cmd", summary: "Run a shell command without sending" },
  { keys: "shift+tab", summary: "Cycle mode auto → ask → chat (ctrl+tab / cmd+tab too)" },
  { keys: "ctrl+l", summary: "Jump to the latest output" },
  { keys: "ctrl+o", summary: "Collapse or expand tool output" },
  { keys: "ctrl+m", summary: "Switch model" },
  { keys: "ctrl+p", summary: "Switch provider" },
  { keys: "ctrl+e", summary: "Extensions / MCP" },
  { keys: "shift+↑↓", summary: "Browse previous turns" },
];

/** True while the composer is a slash stem (`/` or `/mod`), not `/model opus`. */
export function isSlashMenuOpen(input: string): boolean {
  return /^\/[^\s]*$/.test(input);
}

export function slashStem(input: string): string {
  if (!isSlashMenuOpen(input)) return "";
  return input.slice(1).toLowerCase();
}

export function parseSlashInput(
  text: string,
): { name: string; args: string } | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("/") || trimmed.includes("\n")) return null;
  const rest = trimmed.slice(1);
  const space = rest.search(/\s/);
  if (space === -1) {
    return { name: rest.toLowerCase(), args: "" };
  }
  return {
    name: rest.slice(0, space).toLowerCase(),
    args: rest.slice(space).trim(),
  };
}

export function commandNames(def: SlashCommandDef): string[] {
  return [def.name, ...def.aliases];
}

export function resolveSlashCommand(name: string): SlashCommandDef | undefined {
  const needle = name.toLowerCase();
  return SLASH_COMMANDS.find((c) => commandNames(c).includes(needle));
}

export function filterSlashCommands(stem: string): SlashCommandDef[] {
  const q = stem.toLowerCase();
  if (!q) return SLASH_COMMANDS;
  return SLASH_COMMANDS.filter((c) =>
    commandNames(c).some((n) => n.startsWith(q) || n.includes(q)),
  );
}

export function resolveUserPath(
  input: string,
  cwd: string,
  home = os.homedir(),
): string {
  const trimmed = input.trim();
  if (trimmed === "~") return home;
  if (trimmed.startsWith("~/")) return path.join(home, trimmed.slice(2));
  if (path.isAbsolute(trimmed)) return trimmed;
  return path.resolve(cwd, trimmed);
}

export async function gitDiffSummary(cwd: string): Promise<string> {
  try {
    const { stdout: status } = await execFileAsync("git", ["status", "-sb"], {
      cwd,
      timeout: 8000,
    });
    const { stdout: stat } = await execFileAsync(
      "git",
      ["diff", "--stat", "HEAD"],
      { cwd, timeout: 8000 },
    );
    const body = [status.trim(), stat.trim()].filter(Boolean).join("\n\n");
    return body || "Working tree is clean.";
  } catch (e: unknown) {
    const err = e as { stderr?: string; message?: string };
    const msg = String(err.stderr || err.message || e);
    if (/not a git repository/i.test(msg)) return "Not a git repository.";
    return `git failed: ${msg.trim()}`;
  }
}

export function formatHelpText(): string {
  const cmds = SLASH_COMMANDS.map((c) => {
    const keys = c.keys ? `  ${c.keys}` : "";
    return `  /${c.name.padEnd(12)} ${c.summary}${keys}`;
  }).join("\n");
  const keys = KEYBINDINGS.map(
    (k) => `  ${k.keys.padEnd(12)} ${k.summary}`,
  ).join("\n");
  return `Commands\n${cmds}\n\nKeys\n${keys}`;
}
