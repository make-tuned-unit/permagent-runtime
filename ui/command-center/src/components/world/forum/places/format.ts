/**
 * World W2 — pure wording helpers for the place panels. No clock reads: every
 * function takes `now` so fixture snapshots are deterministic.
 */
import type { AgentStateTruth } from '../data/identity';
import type { FailureCause, GoalPhase, GoalRecord, Provenance, RequestOutcome, Section } from '../data/types';
import { getIdentity } from '../../agents/roster';

/** "just now", "4 min ago", "3 h ago", "2 days ago". Unknown input says so. */
export function ageLabel(iso: string | null | undefined, now: number): string {
  if (!iso) return 'time not recorded';
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return 'time not recorded';
  const s = Math.round((now - t) / 1000);
  if (s < 0) return 'just now';
  if (s < 45) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 36) return `${h} h ago`;
  const d = Math.round(h / 24);
  return `${d} day${d === 1 ? '' : 's'} ago`;
}

export function failureLabel(cause: FailureCause): string {
  switch (cause) {
    case 'disconnected': return 'Can’t reach the daemon. This is a lost connection, not a failed task.';
    case 'unauthorized': return 'The daemon refused this client’s credential.';
    case 'not_found': return 'This daemon has no such route or record.';
    case 'server_error': return 'The daemon answered with an error.';
    case 'section_unavailable': return 'The daemon reported this section unavailable.';
    case 'malformed': return 'The daemon’s answer could not be read.';
  }
}

export function displayName(worldId: string, names?: Readonly<Record<string, string>>): string {
  return names?.[worldId] ?? (worldId === 'henry' ? 'Your orchestrator' : getIdentity(worldId)?.name ?? worldId);
}

const CONFIGURED: Record<AgentStateTruth['configured']['value'], string> = {
  on: 'Switched on', off: 'Switched off', no_switch: 'No on/off switch', unknown: 'Not reported',
};
const REACHABLE: Record<AgentStateTruth['reachable']['value'], string> = {
  reachable: 'Reachable', unreachable: 'Unreachable', probe_failed: 'Probe failed', not_probed: 'Not probed', unknown: 'Unknown',
};
const READY: Record<AgentStateTruth['ready']['value'], string> = {
  ready: 'Ready', not_ready: 'Not ready', unknown: 'Unconfirmed',
};
const USED: Record<AgentStateTruth['activelyUsed']['value'], string> = {
  active_now: 'Working now', attributed_in_window: 'Recent work attributed', none_attributed_in_window: 'Nothing attributed in this window', unknown: 'Unknown',
};

/** The four facts, in order, each with its value label and evidence sentence. */
export function truthFacts(truth: AgentStateTruth): Array<{ id: keyof AgentStateTruth; label: string; value: string; raw: string; evidence: string }> {
  return [
    { id: 'configured', label: 'Configured', value: CONFIGURED[truth.configured.value], raw: truth.configured.value, evidence: truth.configured.evidence },
    { id: 'reachable', label: 'Reachable', value: REACHABLE[truth.reachable.value], raw: truth.reachable.value, evidence: truth.reachable.evidence },
    { id: 'ready', label: 'Ready', value: READY[truth.ready.value], raw: truth.ready.value, evidence: truth.ready.evidence },
    { id: 'activelyUsed', label: 'Actively used', value: USED[truth.activelyUsed.value], raw: truth.activelyUsed.value, evidence: truth.activelyUsed.evidence },
  ];
}

export function requestOutcomeLabel(outcome: RequestOutcome, raw: string): string {
  switch (outcome) {
    case 'running': return 'Running';
    case 'completed': return 'Completed';
    case 'failed': return 'Failed';
    case 'cancelled': return 'Cancelled';
    case 'other': return `Reported “${raw}”`;
  }
}

export function goalPhaseLabel(phase: GoalPhase, raw: string): string {
  switch (phase) {
    case 'triage': return 'In triage';
    case 'ready': return 'Ready to start';
    case 'executing': return 'Executing';
    case 'review': return 'Needs your review';
    case 'complete': return 'Complete';
    case 'failed': return 'Failed';
    case 'cancelled': return 'Cancelled';
    case 'unknown': return `State “${raw}”`;
  }
}

/**
 * Where a piece of work stands on the draft → sent → tracked → verified line.
 * A goal is "verified" only when it is complete AND a review decision is on
 * record; complete without one says so.
 */
export type WorkStage = 'draft' | 'sent' | 'tracked' | 'needs_review' | 'verified' | 'complete_unreviewed' | 'failed' | 'cancelled' | 'unknown';

export function goalStage(goal: Pick<GoalRecord, 'phase' | 'reviewDecisions'>): WorkStage {
  switch (goal.phase) {
    case 'triage': case 'ready': case 'executing': return 'tracked';
    case 'review': return 'needs_review';
    case 'complete':
      return goal.reviewDecisions?.status === 'ok' && goal.reviewDecisions.count > 0 ? 'verified' : 'complete_unreviewed';
    case 'failed': return 'failed';
    case 'cancelled': return 'cancelled';
    case 'unknown': return 'unknown';
  }
}

export const STAGE_LABEL: Record<WorkStage, string> = {
  draft: 'Draft — not sent',
  sent: 'Sent — acknowledged',
  tracked: 'Tracked goal',
  needs_review: 'Tracked — needs your review',
  verified: 'Verified — complete with a review on record',
  complete_unreviewed: 'Complete — no review on record',
  failed: 'Failed',
  cancelled: 'Cancelled',
  unknown: 'State not recognised',
};

export function provenanceLabel(p: Provenance): { label: string; gap: boolean } {
  switch (p.kind) {
    case 'source_tag': return { label: `Source tag: ${p.tag}. A tag names where it came from; it is not necessarily an openable document.`, gap: false };
    case 'key_origin': return { label: `Written as a “${p.origin}” record. No source document is attached.`, gap: false };
    case 'none': return { label: 'No provenance recorded. Treat this as stored context, not sourced evidence.', gap: true };
  }
}

export function usd(n: number): string {
  return `$${n.toFixed(2)}`;
}

/** Freshness summary for a set of sections (the panel's state line). */
export function snapshotStateLine(sections: ReadonlyArray<Section<unknown> | undefined>, now: number): { text: string; tone: 'ok' | 'stale' | 'error' | 'setup' } {
  const live = sections.filter((s): s is Section<unknown> => !!s);
  const errors = live.filter((s) => s.state === 'error');
  if (errors.length) {
    const disconnected = errors.some((s) => s.state === 'error' && s.failure.cause === 'disconnected');
    const withData = errors.some((s) => s.state === 'error' && s.lastGood);
    const lead = disconnected ? 'Daemon unreachable — a lost connection, not a failed task.' : 'Some evidence could not be read.';
    return { text: `${lead} ${withData ? 'Showing the last good data, marked stale.' : 'No earlier data to show.'}`, tone: 'error' };
  }
  const timed = live.filter((s): s is Extract<Section<unknown>, { fetchedAt: string }> => 'fetchedAt' in s && s.endpoint.startsWith('/'));
  const stale = timed.filter((s) => now - Date.parse(s.fetchedAt) > s.staleAfterMs);
  if (stale.length) return { text: `Some evidence is stale (read ${ageLabel(stale[0].fetchedAt, now)}). It refreshes on your next visit.`, tone: 'stale' };
  if (live.length && live.every((s) => s.state === 'setup_required' || s.state === 'unsupported')) return { text: 'Needs setup before evidence can be shown.', tone: 'setup' };
  const newest = timed.map((s) => s.fetchedAt).sort().pop();
  return { text: newest ? `Evidence read ${ageLabel(newest, now)}.` : 'No daemon evidence for this place.', tone: 'ok' };
}
