/**
 * Inbox routing (#395) — client + pure logic for sending a Downloads-inbox
 * file to a destination surface. Explicit and user-controlled: the user picks
 * Brain / project / scheduler; Permagent never guesses where a file goes.
 *
 * Wire contract mirrors the daemon's `POST /api/inbox/{id}/route`
 * (crates/goose-server/src/routes/inbox.rs): `brain` needs no target and marks
 * the row `ingested`; `project` and `scheduler` are project-scoped and mark it
 * `routed` with the project stamped.
 */

import { apiFetch, type InboxFile } from '../../lib/api';
import type { Project } from '../projects/types';

export type RouteDestination = 'brain' | 'project' | 'scheduler';

/** Response of POST /api/inbox/{id}/route — the updated row + per-destination detail. */
export interface RouteInboxResponse {
  file: InboxFile;
  destination: string;
  /** Reader digest gist (brain; empty string when the file was visual-only). */
  summary: string | null;
  /** Project document id (project). */
  document_id: string | null;
  /** social_post card id (scheduler). */
  card_id: string | null;
}

/** project + scheduler routes land on a project; brain does not. */
export function needsProject(dest: RouteDestination): boolean {
  return dest === 'project' || dest === 'scheduler';
}

/** Routing controls render for any live row; only deleted rows are inert. */
export function canRoute(status: string): boolean {
  return status !== 'deleted';
}

/** Status chip text for the inbox table. */
export function statusLabel(status: string): string {
  switch (status) {
    case 'received':
      return 'New';
    case 'ingested':
      return 'In Brain';
    case 'routed':
      return 'Routed';
    case 'deleted':
      return 'Deleted';
    default:
      return status;
  }
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : `${s.slice(0, max - 1).trimEnd()}…`;
}

/** One human sentence for the per-row result line after a successful route. */
export function describeRouteResult(resp: RouteInboxResponse, projectName?: string): string {
  const target = projectName ?? 'the project';
  switch (resp.destination) {
    case 'brain':
      return resp.summary
        ? `Read into the Brain — “${truncate(resp.summary, 90)}”`
        : 'The Reader saw a visual file — nothing to read into the Brain';
    case 'project':
      return `Filed as a document in ${target}`;
    case 'scheduler':
      return `Drafted as a social post on ${target}’s board`;
    default:
      return `Routed to ${resp.destination}`;
  }
}

/** Send one inbox file to one destination. Throws on any non-2xx (apiFetch). */
export function routeInboxFile(
  id: string,
  destination: RouteDestination,
  projectId?: string,
): Promise<RouteInboxResponse> {
  return apiFetch<RouteInboxResponse>(`/api/inbox/${encodeURIComponent(id)}/route`, {
    method: 'POST',
    body: JSON.stringify({
      destination,
      ...(projectId ? { project_id: projectId } : {}),
    }),
  });
}

/** Projects the picker offers as routing targets (same list the app shows). */
export function fetchRoutableProjects(): Promise<Project[]> {
  return apiFetch<Project[]>('/api/projects');
}
