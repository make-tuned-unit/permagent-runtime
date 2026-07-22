/**
 * Per-project publish sequence (#457) — the ordered post-push steps a project
 * needs before a change is actually LIVE (e.g. seed the prod DB, then
 * `vercel --prod` so static pages regenerate). A git push runs none of them,
 * so for these projects "pushed" must never be read as "live".
 *
 * Storage: `projects.metadata_json.publish_sequence` (the schema-v26 bag that
 * already hosts `build_command`), canonical entry shape:
 *
 *   { "order": 1, "command": "npx tsx scripts/reseed-threads.ts", "timeout_secs": 300 }
 *
 * Built entirely on EXISTING endpoints:
 *   GET   /api/projects/:id   — freshest copy before any metadata write
 *   PATCH /api/projects/:id   — `metadataJson` (full-replacement JSON object)
 *
 * CONTRACT (routes/projects.rs `UpdateProjectRequest`): `metadataJson` is a
 * FULL-REPLACEMENT object and the bag is shared (build_command #456, the
 * Overview's brief/links #472, …), so every save here re-fetches the project
 * and merges over the freshest copy, touching ONLY its own key — the same
 * foreign-key-preserving discipline as workspaceMeta.ts (#472/PR #794).
 *
 * The reader mirrors the tolerant Rust parser
 * (`platform_extensions/publish_sequence.rs`): bare-string entries are
 * commands, malformed entries are dropped, explicit `order` wins over array
 * position.
 */

import { apiFetch } from '../../lib/api';
import type { Project } from './types';

/** metadata_json key for the ordered post-push publish steps. */
export const PUBLISH_SEQUENCE_KEY = 'publish_sequence';

export interface PublishStep {
  command: string;
  /** Optional per-step timeout in seconds. */
  timeoutSecs?: number;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

/**
 * Publish steps out of the metadata bag, in publish order; [] when absent or
 * malformed. Tolerant (the bag is shared and agent-writable): non-arrays read
 * as [], blank/malformed entries are dropped, explicit `order` sorts first
 * (array-index tiebreak, matching the Rust parser).
 */
export function readPublishSequence(meta: unknown): PublishStep[] {
  if (!isRecord(meta)) return [];
  const raw = meta[PUBLISH_SEQUENCE_KEY];
  if (!Array.isArray(raw)) return [];

  const keyed: { order: number; idx: number; step: PublishStep }[] = [];
  raw.forEach((entry, idx) => {
    let command = '';
    let order: number | undefined;
    let timeoutSecs: number | undefined;
    if (typeof entry === 'string') {
      command = entry.trim();
    } else if (isRecord(entry)) {
      command = typeof entry.command === 'string' ? entry.command.trim() : '';
      if (typeof entry.order === 'number' && Number.isFinite(entry.order)) {
        order = entry.order;
      }
      if (
        typeof entry.timeout_secs === 'number' &&
        Number.isFinite(entry.timeout_secs) &&
        entry.timeout_secs > 0
      ) {
        timeoutSecs = entry.timeout_secs;
      }
    }
    if (command === '') return;
    keyed.push({ order: order ?? idx, idx, step: { command, timeoutSecs } });
  });

  keyed.sort((a, b) => (a.order !== b.order ? a.order - b.order : a.idx - b.idx));
  return keyed.map(k => k.step);
}

/**
 * Merge a publish-sequence edit into an existing metadata bag, preserving
 * every foreign key (build_command, brief, links, …). Pure — does not mutate
 * the input. Steps are normalized to the canonical shape with `order` 1..N;
 * blank commands are dropped; an empty sequence removes the key entirely so
 * the bag never accretes dead entries.
 */
export function mergePublishSequence(
  meta: unknown,
  steps: PublishStep[],
): Record<string, unknown> {
  const base: Record<string, unknown> = isRecord(meta) ? { ...meta } : {};
  const cleaned = steps
    .map(s => ({ command: s.command.trim(), timeoutSecs: s.timeoutSecs }))
    .filter(s => s.command !== '');
  if (cleaned.length === 0) {
    delete base[PUBLISH_SEQUENCE_KEY];
    return base;
  }
  base[PUBLISH_SEQUENCE_KEY] = cleaned.map((s, i) => ({
    order: i + 1,
    command: s.command,
    ...(s.timeoutSecs !== undefined && Number.isFinite(s.timeoutSecs) && s.timeoutSecs > 0
      ? { timeout_secs: Math.floor(s.timeoutSecs) }
      : {}),
  }));
  return base;
}

/**
 * Persist a publish-sequence edit via the existing PATCH endpoint. The project
 * is re-fetched first so the merge runs over the freshest copy — a stale prop
 * must never clobber another feature's keys. (A concurrent write between the
 * GET and the PATCH can still lose; acceptable for a single-user local
 * daemon, same trade-off as workspaceMeta.ts.)
 */
export async function savePublishSequence(
  projectId: string,
  steps: PublishStep[],
): Promise<Project> {
  const id = encodeURIComponent(projectId);
  const fresh = await apiFetch<Project>(`/api/projects/${id}`);
  const metadataJson = mergePublishSequence(fresh.metadataJson, steps);
  return apiFetch<Project>(`/api/projects/${id}`, {
    method: 'PATCH',
    body: JSON.stringify({ metadataJson }),
  });
}
