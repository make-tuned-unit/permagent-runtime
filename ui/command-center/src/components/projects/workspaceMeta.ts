/**
 * Project workspace metadata — the #472 residue (brief + custom links + summary
 * editing) built entirely on EXISTING endpoints:
 *
 *   GET   /api/projects/:id        — freshest copy before any metadata write
 *   PATCH /api/projects/:id        — description / siteUrl / repoUrl / metadataJson
 *
 * `brief` and `links` live inside `projects.metadata_json` (schema v26, landed
 * for #456's `build_command`) under the keys below — no schema bump, and #457's
 * publish-sequence config can share the same bag with its own keys.
 *
 * CONTRACT (routes/projects.rs `UpdateProjectRequest`): `metadataJson` is a
 * FULL-REPLACEMENT JSON object. Other features keep keys in this bag, so every
 * save here re-fetches the project and merges over the freshest copy, touching
 * only its own keys. `siteUrl` / `repoUrl` are double-Option on the wire —
 * an explicit JSON `null` clears them, absence leaves them unchanged.
 */

import { apiFetch } from '../../lib/api';
import type { Project } from './types';

/** metadata_json key for the long-form project brief (multi-line text). */
export const BRIEF_KEY = 'brief';
/** metadata_json key for social/other links beyond site_url/repo_url. */
export const LINKS_KEY = 'links';

export interface WorkspaceLink {
  label: string;
  url: string;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/** Long-form brief out of the metadata bag; '' when absent or malformed. */
export function readBrief(meta: unknown): string {
  if (!isRecord(meta)) return '';
  const v = meta[BRIEF_KEY];
  return typeof v === 'string' ? v : '';
}

/**
 * Custom links out of the metadata bag. Tolerant of malformed entries (the bag
 * is shared and agent-writable): non-arrays read as [], entries without a
 * string label+url — or with an empty url — are dropped, never thrown on.
 */
export function readLinks(meta: unknown): WorkspaceLink[] {
  if (!isRecord(meta)) return [];
  const v = meta[LINKS_KEY];
  if (!Array.isArray(v)) return [];
  const out: WorkspaceLink[] = [];
  for (const item of v) {
    if (
      isRecord(item) &&
      typeof item.label === 'string' &&
      typeof item.url === 'string' &&
      item.url.trim() !== ''
    ) {
      out.push({ label: item.label, url: item.url });
    }
  }
  return out;
}

/**
 * User-typed URL → storable URL. Trims; empty → null (meaning "clear");
 * scheme-less input gets https:// so `useBrowserNavigate` opens it correctly.
 * Existing schemes (https, mailto, …) pass through untouched.
 */
export function normalizeUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) return trimmed;
  return `https://${trimmed}`;
}

/**
 * Merge brief/links changes into an existing metadata bag, preserving every
 * foreign key (build_command, publish-sequence, …). Pure — does not mutate
 * the input. Empty brief / empty links remove their key entirely so the bag
 * never accretes dead entries.
 */
export function mergeWorkspaceMeta(
  meta: unknown,
  changes: { brief?: string; links?: WorkspaceLink[] },
): Record<string, unknown> {
  const base: Record<string, unknown> = isRecord(meta) ? { ...meta } : {};
  if (changes.brief !== undefined) {
    const brief = changes.brief.trim();
    if (brief) base[BRIEF_KEY] = brief;
    else delete base[BRIEF_KEY];
  }
  if (changes.links !== undefined) {
    const links = changes.links
      .map(l => ({ label: l.label.trim(), url: l.url.trim() }))
      .filter(l => l.url !== '');
    if (links.length > 0) base[LINKS_KEY] = links;
    else delete base[LINKS_KEY];
  }
  return base;
}

export interface SummarySave {
  description?: string;
  brief?: string;
  links?: WorkspaceLink[];
  /** null clears the field on the wire (double-Option PATCH semantics). */
  siteUrl?: string | null;
  repoUrl?: string | null;
}

/**
 * Persist Overview summary edits via the existing PATCH endpoint. When the
 * metadata bag is touched, the project is re-fetched first so the merge runs
 * over the freshest copy — a stale prop must never clobber another feature's
 * keys. (A concurrent write between the GET and the PATCH can still lose;
 * acceptable for a single-user local daemon, flagged in the PR.)
 */
export async function saveProjectSummary(
  projectId: string,
  changes: SummarySave,
): Promise<Project> {
  const id = encodeURIComponent(projectId);
  const body: Record<string, unknown> = {};
  if (changes.brief !== undefined || changes.links !== undefined) {
    const fresh = await apiFetch<Project>(`/api/projects/${id}`);
    body.metadataJson = mergeWorkspaceMeta(fresh.metadataJson, changes);
  }
  if (changes.description !== undefined) body.description = changes.description;
  if (changes.siteUrl !== undefined) body.siteUrl = changes.siteUrl;
  if (changes.repoUrl !== undefined) body.repoUrl = changes.repoUrl;
  return apiFetch<Project>(`/api/projects/${id}`, {
    method: 'PATCH',
    body: JSON.stringify(body),
  });
}
