import { execFile } from "node:child_process";
import { existsSync, readdirSync, statSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

const SKIP_DIRS = new Set([
  ".git",
  "node_modules",
  "target",
  "dist",
  ".strix",
  ".next",
  "coverage",
]);

export function atQuery(
  input: string,
): { start: number; query: string } | null {
  if (input.startsWith("/") || input.startsWith("!")) return null;
  const m = /(?:^|[\s])@([^\s]*)$/.exec(input);
  if (!m) return null;
  const query = m[1] ?? "";
  const start = input.length - query.length - 1;
  return { start, query };
}

export function applyAtMention(
  input: string,
  start: number,
  file: string,
): string {
  const insert = /[\s"]/.test(file) ? `"${file.replace(/"/g, '\\"')}"` : file;
  return `${input.slice(0, start)}@${insert} `;
}

export function stripAtQuery(input: string): string {
  const q = atQuery(input);
  if (!q) return input;
  return input.slice(0, q.start).replace(/\s+$/, "");
}

export function scoreFile(file: string, query: string): number {
  if (!query) return 1;
  const q = query.toLowerCase();
  const full = file.toLowerCase();
  const base = (file.split("/").pop() ?? file).toLowerCase();
  if (base === q) return 400;
  if (base.startsWith(q)) return 300 - file.length / 100;
  if (base.includes(q)) return 200 - file.length / 100;
  if (full.includes(q)) return 100 - file.length / 100;
  let qi = 0;
  for (const ch of full) {
    if (ch === q[qi]) qi += 1;
    if (qi === q.length) return 50 - file.length / 100;
  }
  return 0;
}

export function fuzzyFiles(
  files: string[],
  query: string,
  limit = 8,
): string[] {
  const ranked = files
    .map((file) => ({ file, score: scoreFile(file, query) }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score || a.file.localeCompare(b.file));
  return ranked.slice(0, limit).map((x) => x.file);
}

export async function listProjectFiles(cwd: string): Promise<string[]> {
  try {
    const { stdout } = await execFileAsync(
      "git",
      ["ls-files", "-co", "--exclude-standard"],
      { cwd, timeout: 8000, maxBuffer: 2 * 1024 * 1024 },
    );
    return stdout.split("\n").filter(Boolean).slice(0, 4000);
  } catch {
    return walkFiles(cwd, cwd, 4000);
  }
}

function walkFiles(root: string, dir: string, budget: number): string[] {
  const out: string[] = [];
  let entries: string[] = [];
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries) {
    if (out.length >= budget) break;
    if (SKIP_DIRS.has(name)) continue;
    const full = path.join(dir, name);
    let st;
    try {
      st = statSync(full);
    } catch {
      continue;
    }
    if (st.isDirectory()) {
      out.push(...walkFiles(root, full, budget - out.length));
    } else {
      out.push(path.relative(root, full) || name);
    }
  }
  return out;
}

export function parseBang(
  text: string,
): { sendToModel: boolean; command: string } | null {
  const t = text.trim();
  if (t.startsWith("!!")) {
    const command = t.slice(2).trim();
    return command ? { sendToModel: false, command } : null;
  }
  if (t.startsWith("!") && t.length > 1 && t[1] !== "=") {
    const command = t.slice(1).trim();
    return command ? { sendToModel: true, command } : null;
  }
  return null;
}

export async function runShellCommand(
  command: string,
  cwd: string,
): Promise<{ ok: boolean; output: string }> {
  try {
    const { stdout, stderr } = await execFileAsync("sh", ["-c", command], {
      cwd,
      timeout: 30_000,
      maxBuffer: 64 * 1024,
    });
    const output = [stdout, stderr].filter(Boolean).join("\n").trim();
    return { ok: true, output: output.slice(0, 8000) };
  } catch (e: unknown) {
    const err = e as { stdout?: string; stderr?: string; message?: string };
    const output = [err.stdout, err.stderr, err.message]
      .filter(Boolean)
      .join("\n")
      .trim();
    return { ok: false, output: (output || String(e)).slice(0, 8000) };
  }
}

export function formatShellPrompt(
  command: string,
  output: string,
  ok: boolean,
): string {
  const body = output || "(no output)";
  return ok
    ? `$ ${command}\n\n${body}`
    : `$ ${command}\n\n${body}\n\n(command failed)`;
}

export async function copyToClipboard(text: string): Promise<void> {
  const { spawn } = await import("node:child_process");
  const platform = process.platform;
  const bin =
    platform === "darwin" ? "pbcopy" : platform === "win32" ? "clip" : "xclip";
  const args = platform === "linux" ? ["-selection", "clipboard"] : [];
  await new Promise<void>((resolve, reject) => {
    const child = spawn(bin, args);
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${bin} exited ${code}`));
    });
    child.stdin.end(text);
  });
}

export function lastAssistantText(
  turns: Array<{
    responseItems: Array<{
      itemType: string;
      content?: { type?: string; text?: string };
    }>;
  }>,
): string {
  for (let i = turns.length - 1; i >= 0; i--) {
    const chunks: string[] = [];
    for (const item of turns[i]!.responseItems) {
      if (
        item.itemType === "content_chunk" &&
        item.content?.type === "text" &&
        item.content.text
      ) {
        chunks.push(item.content.text);
      }
    }
    const text = chunks.join("").trim();
    if (text) return text;
  }
  return "";
}

export function formatTranscript(
  turns: Array<{
    userText: string;
    responseItems: Array<{
      itemType: string;
      content?: { type?: string; text?: string };
      title?: string;
    }>;
  }>,
): string {
  const parts: string[] = [];
  for (const turn of turns) {
    parts.push(`## User\n\n${turn.userText.trim()}\n`);
    const body: string[] = [];
    for (const item of turn.responseItems) {
      if (item.itemType === "tool_call" && item.title) {
        body.push(`- tool: ${item.title}`);
      } else if (
        item.itemType === "content_chunk" &&
        item.content?.type === "text" &&
        item.content.text
      ) {
        body.push(item.content.text);
      }
    }
    if (body.length) parts.push(`## Assistant\n\n${body.join("\n\n")}\n`);
  }
  return parts.join("\n") || "(empty session)";
}

export function writeTranscript(file: string, markdown: string): string {
  writeFileSync(file, markdown, "utf8");
  return file;
}

export function harnessFacts(cwd: string, home = os.homedir()): string[] {
  const facts: string[] = [];
  if (existsSync(path.join(cwd, "AGENTS.md"))) facts.push("AGENTS.md");
  else if (existsSync(path.join(cwd, "CLAUDE.md"))) facts.push("CLAUDE.md");
  const n = countSkills(cwd, home);
  if (n > 0) facts.push(`${n} skill${n === 1 ? "" : "s"}`);
  return facts;
}

function countSkills(cwd: string, home: string): number {
  const roots = [
    path.join(cwd, "skills"),
    path.join(cwd, ".agents", "skills"),
    path.join(home, ".agents", "skills"),
  ];
  let n = 0;
  for (const root of roots) {
    if (!existsSync(root)) continue;
    let entries: string[] = [];
    try {
      entries = readdirSync(root);
    } catch {
      continue;
    }
    for (const name of entries) {
      if (existsSync(path.join(root, name, "SKILL.md"))) n += 1;
    }
  }
  return n;
}

export function defaultExportPath(cwd: string): string {
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
  return path.join(cwd, `permagent-session-${stamp}.md`);
}
