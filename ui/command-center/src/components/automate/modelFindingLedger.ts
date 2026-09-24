import { apiFetch } from '../../lib/api';

// W1b Model Review findings: the wire shapes of the one findings ledger
// (GET /api/model-review/findings, POST /api/model-review/findings/actions),
// plus the pure grouping and label helpers the view renders from.
//
// Nothing here touches storage cleanup: model findings are dismissed, snoozed
// or reopened, never trashed or kept (Astra W1, ruling 13).

export type ModelFindingState = 'open' | 'deferred' | 'dismissed';

export type ModelFindingTier =
  | 'update_available'
  | 'candidate_worth_evaluating'
  | 'validated_switch_recommendation'
  | 'policy_blocked'
  | 'upstream_unknown';

export interface ModelFindingDiscovery {
  source: string;
  snapshot_taken_at: string | null;
  incumbent_identity: string;
  candidate_identity: string | null;
  quantization?: string | null;
  candidate_quantization?: string | null;
  compatibility: string;
  size?: number | null;
  candidate_size?: number | null;
  uncertainty: string;
  stale: boolean;
}

export interface ModelFinding {
  consumer: string;
  tier: ModelFindingTier | string;
  action: string;
  discovery: ModelFindingDiscovery;
}

export interface ModelFindingRow {
  /** Opaque. Never parsed; sent back verbatim. */
  key: string;
  report_id: string;
  /** Effective state: a lapsed deferral already reads "open". */
  state: ModelFindingState;
  defer_until: string | null;
  decision_id: string | null;
  updated_at: string | null;
  finding: ModelFinding;
}

export type ModelFindingAction = 'dismiss' | 'defer' | 'reopen';

export interface ModelFindingActionResponse {
  updated: string[];
  unknown: string[];
}

export const MODEL_FINDINGS_PATH = '/api/model-review/findings';
export const MODEL_FINDING_ACTIONS_PATH = '/api/model-review/findings/actions';

export function listModelFindings(): Promise<ModelFindingRow[]> {
  return apiFetch<ModelFindingRow[]>(MODEL_FINDINGS_PATH);
}

/** One endpoint for a single row and for a bulk selection. */
export function actOnModelFindings(
  keys: string[],
  action: ModelFindingAction,
  deferUntil?: string,
): Promise<ModelFindingActionResponse> {
  const body: Record<string, unknown> = { keys, action };
  if (action === 'defer') body.defer_until = deferUntil;
  return apiFetch<ModelFindingActionResponse>(MODEL_FINDING_ACTIONS_PATH, {
    method: 'POST',
    body: JSON.stringify(body),
  });
}

export const TIER_LABEL: Record<ModelFindingTier, string> = {
  update_available: 'Update available',
  candidate_worth_evaluating: 'Worth evaluating',
  validated_switch_recommendation: 'Validated switch recommendation',
  policy_blocked: 'Blocked by policy',
  upstream_unknown: 'Upstream unknown',
};

/** A tier this build does not know is shown as-is rather than hidden. */
export function tierLabel(tier: string): string {
  return (TIER_LABEL as Record<string, string>)[tier] ?? tier.replace(/_/g, ' ');
}

export const GROUP_LABEL: Record<ModelFindingState, string> = {
  open: 'Open',
  deferred: 'Deferred',
  dismissed: 'Dismissed',
};

const GROUP_ORDER: ModelFindingState[] = ['open', 'deferred', 'dismissed'];

/** Split rows into Open / Deferred / Dismissed. The server already orders the
 *  list (state, then consumer, then key); within a group the server's order is
 *  kept exactly, never re-sorted. Empty groups are dropped. */
export function groupModelFindings(rows: ModelFindingRow[]): { state: ModelFindingState; label: string; rows: ModelFindingRow[] }[] {
  const buckets = new Map<ModelFindingState, ModelFindingRow[]>(GROUP_ORDER.map(s => [s, []]));
  for (const row of rows) {
    // An unrecognised state is still a finding the user must be able to see:
    // it reads as open, the contract's default for "no state row".
    const bucket = buckets.get(row.state) ?? buckets.get('open')!;
    bucket.push(row);
  }
  return GROUP_ORDER
    .map(state => ({ state, label: GROUP_LABEL[state], rows: buckets.get(state)! }))
    .filter(group => group.rows.length > 0);
}

/** Fixed snooze lengths, all inside the server's 180-day limit. */
export const SNOOZE_OPTIONS: { label: string; days: number }[] = [
  { label: '1 week', days: 7 },
  { label: '30 days', days: 30 },
  { label: '90 days', days: 90 },
];

/** RFC3339 UTC time `days` from `now`, whole seconds. */
export function snoozeUntil(days: number, now: number = Date.now()): string {
  return new Date(now + days * 86_400_000).toISOString().replace(/\.\d{3}Z$/, 'Z');
}

export function unknownNote(count: number): string {
  return count === 1
    ? '1 finding was no longer in the ledger.'
    : `${count} findings were no longer in the ledger.`;
}

export function candidateLabel(discovery: ModelFindingDiscovery): string {
  return discovery.candidate_identity ?? 'no candidate named';
}
