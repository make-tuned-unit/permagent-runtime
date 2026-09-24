/**
 * Spend → By project: which rows are projects.
 *
 * The server rolls spend up by each session's working directory and labels a
 * row with the project's name when that directory is a project root, else with
 * the directory's basename (crates/goose/src/app_views.rs, build_spend_snapshot).
 * So a chat started from `/`, `/tmp` or the home folder showed up as projects
 * called "/", "tmp" and "j" (QA 2026-09-24) — the payload does not say which
 * rows are real projects.
 *
 * The UI knows the project roots (`GET /api/projects` → `rootPath`), so it
 * sorts the rows here: a directory that is a project root, or inside one,
 * stays a project row; everything else folds into one "No project" row that
 * keeps its per-directory breakdown. Without the roots (the read failed) the
 * rows are shown as the server sent them — guessing from labels would be worse.
 */

import type { ProjectSpend } from '../../lib/api';

export interface NoProjectSpend {
  costUsd: number;
  tokens: number;
  sessionCount: number;
  /** The folded directories, most expensive first. */
  dirs: ProjectSpend[];
}

export interface GroupedProjectSpend {
  projects: ProjectSpend[];
  noProject: NoProjectSpend | null;
}

function trimSlash(p: string): string {
  return p.length > 1 ? p.replace(/\/+$/, '') : p;
}

function isUnder(dir: string, root: string): boolean {
  const d = trimSlash(dir);
  const r = trimSlash(root);
  if (r === '/' || r === '') return false; // "/" as a root would swallow everything
  return d === r || d.startsWith(`${r}/`);
}

export function groupProjectSpend(
  rows: ProjectSpend[],
  projectRoots: readonly string[] | null,
): GroupedProjectSpend {
  if (projectRoots === null) return { projects: rows, noProject: null };
  const roots = projectRoots.filter(Boolean);
  const projects: ProjectSpend[] = [];
  const dirs: ProjectSpend[] = [];
  for (const row of rows) {
    if (roots.some(root => isUnder(row.path, root))) projects.push(row);
    else dirs.push(row);
  }
  if (dirs.length === 0) return { projects, noProject: null };
  dirs.sort((a, b) => b.costUsd - a.costUsd);
  return {
    projects,
    noProject: {
      costUsd: dirs.reduce((s, d) => s + d.costUsd, 0),
      tokens: dirs.reduce((s, d) => s + d.tokens, 0),
      sessionCount: dirs.reduce((s, d) => s + d.sessionCount, 0),
      dirs,
    },
  };
}
