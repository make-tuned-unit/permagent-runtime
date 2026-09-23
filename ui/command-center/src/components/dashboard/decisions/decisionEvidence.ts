import type { Decision } from './types';

export type EvidenceTarget =
  | { kind: 'council_report'; councilSessionId: string; label: string }
  | { kind: 'dispatch'; projectId: string; goalId: string; label: string }
  | { kind: 'pause_episode'; jobId: string; episodeId: string; label: string }
  | { kind: 'budget'; label: string; detail: string }
  | { kind: 'none'; reason: string };

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function usd(value: number): string {
  return `$${value.toFixed(2)}`;
}

export function evidenceTargetFor(d: Decision): EvidenceTarget {
  const payload = asObject(d.payload);

  if (d.kind === 'council_action') {
    const sessionId = payload && typeof payload.session_id === 'string'
      ? payload.session_id.trim()
      : '';
    if (!sessionId) {
      return { kind: 'none', reason: 'council_action payload has no council session id.' };
    }
    return {
      kind: 'council_report',
      councilSessionId: sessionId,
      label: 'Open council report',
    };
  }

  if (d.kind === 'unblock' && payload) {
    const target = asObject(payload.target);
    const jobId = target && target.type === 'scheduled_job' && typeof target.job_id === 'string'
      ? target.job_id
      : null;
    const episodeId = typeof payload.pause_episode_id === 'string'
      ? payload.pause_episode_id
      : null;
    if (jobId && episodeId) {
      return {
        kind: 'pause_episode',
        jobId,
        episodeId,
        label: 'Open pause episode evidence',
      };
    }
  }

  if (d.kind === 'choice' && payload) {
    const gate = asObject(payload.reservation_gate);
    if (gate) {
      const scope = typeof gate.scope === 'string' ? gate.scope : 'unknown';
      const settled = asNumber(gate.settled_usd);
      const held = asNumber(gate.held_usd);
      const requested = asNumber(gate.requested_usd);
      const ceiling = asNumber(gate.ceiling_usd);
      const detail = (
        settled !== null &&
        held !== null &&
        requested !== null &&
        ceiling !== null
      )
        ? `${scope} budget gate — settled ${usd(settled)}, held ${usd(held)}, requested ${usd(requested)}, ceiling ${usd(ceiling)}`
        : `${scope} budget gate evidence`;
      return {
        kind: 'budget',
        label: 'Open budget evidence',
        detail,
      };
    }
  }

  if (d.project_id && d.goal_id) {
    return {
      kind: 'dispatch',
      projectId: d.project_id,
      goalId: d.goal_id,
      label: d.goal_title ? `Open evidence for ${d.goal_title}` : 'Open goal evidence',
    };
  }

  if (!payload) {
    return { kind: 'none', reason: 'Decision has no payload or bound goal evidence.' };
  }
  return { kind: 'none', reason: `No evidence target is defined for kind "${d.kind}".` };
}
