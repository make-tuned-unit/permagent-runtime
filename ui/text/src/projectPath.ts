import os from "node:os";
import path from "node:path";

/** Shorten an absolute path the way Cursor's CLI does: `~/Documents/dev/foo`. */
export function formatHomePath(cwd: string, home = os.homedir()): string {
  const resolved = path.resolve(cwd);
  const resolvedHome = path.resolve(home);
  if (resolved === resolvedHome) return "~";
  const prefix = resolvedHome + path.sep;
  if (resolved.startsWith(prefix)) {
    return "~/" + resolved.slice(prefix.length).split(path.sep).join("/");
  }
  return resolved.split(path.sep).join("/");
}

export function projectFolderName(cwd: string): string {
  const resolved = path.resolve(cwd);
  return path.basename(resolved) || resolved;
}
