/**
 * Stack organizer (#512) — wire helpers + grouping for the StackPanel.
 *
 * Talks to the daemon's `/api/projects/{id}/stack` endpoints (mounted in
 * routes/projects.rs). Requests are camelCase; responses are snake_case
 * (`StackEntry` in types.ts matches the wire exactly).
 *
 * REFERENCE-ONLY, BY DESIGN: these helpers carry a service name, the login
 * identity (email/handle) used for it, notes, and a dashboard URL — never a
 * password or secret. The backend has no field for one and 422s unknown keys,
 * so nothing here may ever grow one either.
 */

import { apiFetch } from '../../lib/api';
import {
  STACK_CATEGORIES,
  type StackCategory,
  type StackEntry,
} from './types';

/** Draft fields for creating a stack entry (camelCase request keys). */
export interface StackEntryDraft {
  serviceName: string;
  category: StackCategory;
  identity?: string;
  notes?: string;
  dashboardUrl?: string;
}

/**
 * Patch fields for editing a stack entry. `identity` / `dashboardUrl` support
 * an explicit `null` to clear (double-Option on the backend); omit a field to
 * leave it unchanged.
 */
export interface StackEntryPatch {
  serviceName?: string;
  category?: StackCategory;
  identity?: string | null;
  notes?: string;
  dashboardUrl?: string | null;
}

const stackPath = (projectId: string) =>
  `/api/projects/${encodeURIComponent(projectId)}/stack`;

/** List a project's stack entries (backend returns them grouped by category). */
export function listStackEntries(projectId: string): Promise<StackEntry[]> {
  return apiFetch<StackEntry[]>(stackPath(projectId));
}

/** Add a stack entry; returns the saved row. */
export function createStackEntry(
  projectId: string,
  draft: StackEntryDraft,
): Promise<StackEntry> {
  return apiFetch<StackEntry>(stackPath(projectId), {
    method: 'POST',
    body: JSON.stringify(draft),
  });
}

/** Edit a stack entry; returns the updated row. */
export function updateStackEntry(
  projectId: string,
  entryId: string,
  patch: StackEntryPatch,
): Promise<StackEntry> {
  return apiFetch<StackEntry>(
    `${stackPath(projectId)}/${encodeURIComponent(entryId)}`,
    { method: 'PATCH', body: JSON.stringify(patch) },
  );
}

/** Remove a stack entry. Resolves on 200; apiFetch throws on error statuses. */
export async function deleteStackEntry(
  projectId: string,
  entryId: string,
): Promise<void> {
  await apiFetch<void>(
    `${stackPath(projectId)}/${encodeURIComponent(entryId)}`,
    { method: 'DELETE' },
  );
}

/**
 * Group entries for display: `STACK_CATEGORIES` display order (not the
 * backend's alphabetical order), empty categories omitted. Tolerant of an
 * unknown category from a newer backend — those bucket under "other" rather
 * than vanishing.
 */
export function groupByCategory(
  entries: StackEntry[],
): { category: StackCategory; entries: StackEntry[] }[] {
  const buckets = new Map<StackCategory, StackEntry[]>(
    STACK_CATEGORIES.map(c => [c, []]),
  );
  for (const entry of entries) {
    const cat = (STACK_CATEGORIES as readonly string[]).includes(entry.category)
      ? entry.category
      : 'other';
    buckets.get(cat)!.push(entry);
  }
  return STACK_CATEGORIES
    .filter(c => buckets.get(c)!.length > 0)
    .map(c => ({ category: c, entries: buckets.get(c)! }));
}
