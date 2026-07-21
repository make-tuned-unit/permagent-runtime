/**
 * Roadmap editing client (#251) + per-goal auto-approve (#252).
 *
 * Thin, typed wrappers over the daemon's post-creation roadmap endpoints:
 *   POST /api/projects/{pid}/roadmap/goals                        — insert a goal
 *   PUT  /api/projects/{pid}/roadmap/goals/{cid}/dependencies     — set depends_on (validated)
 *   POST /api/projects/{pid}/roadmap/goals/{cid}/remove           — splice out + cancel
 *
 * All dependency-graph validation (cycles, dangling ids, editable states)
 * lives in the daemon's goal-transition guard — these calls surface its
 * actionable error messages verbatim.
 */

import { apiFetch } from './api';

/** The card shape the roadmap endpoints return (mirrors CardResponse). */
export interface RoadmapCard {
  id: string;
  projectId: string;
  cardType: string;
  title: string;
  description: string;
  columnId: string;
  position: number;
  assignedTo: string | null;
  metadataJson: Record<string, unknown> | null;
  createdAt: string;
  updatedAt: string;
}

export interface InsertRoadmapGoalInput {
  title: string;
  description?: string;
  acceptanceCriteria?: string[];
  tags?: string[];
  dependsOn?: string[];
}

export interface RemoveRoadmapGoalResult {
  removed: boolean;
  cancelled: boolean;
  rewiredDependents: number;
}

const enc = encodeURIComponent;

/** Insert a goal into an existing roadmap. Dependency-satisfied inserts (no
 *  deps, or all deps Complete) land in Ready; others wait in Triage. */
export function insertRoadmapGoal(
  projectId: string,
  input: InsertRoadmapGoalInput,
): Promise<RoadmapCard> {
  return apiFetch<RoadmapCard>(`/api/projects/${enc(projectId)}/roadmap/goals`, {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

/** Replace a goal's dependency set. The daemon re-validates the full project
 *  graph (no cycles / dangling ids) before writing, and promotes the goal if
 *  its new dependencies are already satisfied. */
export function setGoalDependencies(
  projectId: string,
  cardId: string,
  dependsOn: string[],
): Promise<RoadmapCard> {
  return apiFetch<RoadmapCard>(
    `/api/projects/${enc(projectId)}/roadmap/goals/${enc(cardId)}/dependencies`,
    { method: 'PUT', body: JSON.stringify({ dependsOn }) },
  );
}

/** Remove a goal from its roadmap: dependents are rewired onto the removed
 *  goal's own dependencies, then a non-terminal goal is cancelled (worker
 *  killed, open decisions superseded). */
export function removeRoadmapGoal(
  projectId: string,
  cardId: string,
): Promise<RemoveRoadmapGoalResult> {
  return apiFetch<RemoveRoadmapGoalResult>(
    `/api/projects/${enc(projectId)}/roadmap/goals/${enc(cardId)}/remove`,
    { method: 'POST' },
  );
}

/** Per-goal auto-approve opt-in (#252): when enabled, a VERIFIED PASS from
 *  the L2 verifier auto-approves the goal's Tier-1 Review (henry-policy);
 *  a FAIL/Uncertain verdict — or a Tier-2 approval dial — still holds in
 *  Review. Default remains Review-required. */
export function setGoalAutoApprove(
  projectId: string,
  cardId: string,
  enabled: boolean,
): Promise<RoadmapCard> {
  return apiFetch<RoadmapCard>(
    `/api/projects/${enc(projectId)}/cards/${enc(cardId)}/auto-approve`,
    { method: 'POST', body: JSON.stringify({ enabled }) },
  );
}
