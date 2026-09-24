import type { Decision } from './types';

const RECORD_ONLY_TEXT = 'Records your answer; no automatic effect' as const;

export type EffectSummary =
  | { known: true; lines: string[]; missing?: string[] }
  | { known: false; text: typeof RECORD_ONLY_TEXT };

interface ScheduledJobUnblockShape {
  target?: { type?: string; job_id?: string };
  reason?: string;
  pause_episode_id?: string;
  answers?: Array<{ id?: string; label?: string }>;
}

interface ChoiceShape {
  options?: Array<{ id?: string; label?: string }>;
  default?: string | null;
  reservation_gate?: {
    scope?: string;
    settled_usd?: number;
    held_usd?: number;
    requested_usd?: number;
    ceiling_usd?: number;
    requesting_session_id?: string | null;
  };
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function normalizeEffect(effect: string): string {
  const marker = ' — ';
  const idx = effect.indexOf(marker);
  return idx >= 0 ? effect.slice(idx + marker.length).trim() : effect.trim();
}

function usd(value: number): string {
  return `$${value.toFixed(2)}`;
}

// Shared confirm-step copy per kind+answer. This remains the source that
// DecisionItem uses, now lifted to a reusable module.
export function effectTextFor(
  kind: string,
  answer: 'approve' | 'reject',
  agentName: string,
  actionClass?: string,
  decision?: Decision,
): string {
  if (kind === 'approve_review') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will mark this goal complete and start anything waiting on it.`
      : `Confirm reject — ${agentName} will send the work back for another attempt.`;
  }
  if (kind === 'unblock') {
    if (decision) {
      const scheduled = scheduledUnblockConfirmText(decision, answer);
      if (scheduled) return scheduled;
    }
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will wake this goal up and let it try again.`
      : 'Confirm reject — the goal stays parked.';
  }
  if (kind === 'risk_gate') {
    if (actionClass === 'repo_worktree_reap') {
      return answer === 'approve'
        ? `Confirm approve — ${agentName} removes this worktree folder. Anything with uncommitted or unpushed work is skipped, and every check is re-run at the moment of removal.`
        : 'Confirm reject — the worktree stays, and this cleanup will not be suggested again.';
    }
    if (actionClass === 'repo_branch_delete') {
      return answer === 'approve'
        ? `Confirm approve — ${agentName} deletes this local branch (safe delete only: git refuses if it stopped being merged). Every check is re-run at the moment of deletion.`
        : 'Confirm reject — the branch stays, and this cleanup will not be suggested again.';
    }
    return answer === 'approve'
      ? `Confirm approve — ${agentName} may go ahead with this action.`
      : `Confirm reject — ${agentName} will not go ahead with this.`;
  }
  if (kind === 'enrichment_proposal') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will save these details to the person's profile (your manual entries stay protected).`
      : 'Confirm reject — nothing is written to the profile. If this was the wrong person, add a hint below so the next pass can find them.';
  }
  if (kind === 'project_intel_proposal') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will save these cited findings to the project's intelligence panel.`
      : 'Confirm reject — no intelligence is stored.';
  }
  if (kind === 'file_to_project') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will save this as a project note and add the named people (name only, no contact details).`
      : 'Confirm reject — nothing is saved anywhere.';
  }
  if (kind === 'tool_approval') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will run this tool and continue the turn.`
      : `Confirm reject — ${agentName} will skip this tool and continue the turn.`;
  }
  if (kind === 'council_action') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will file this as a board card on the named project.`
      : 'Confirm reject — this council action is dismissed; nothing is filed on the board.';
  }
  if (kind === 'session_gate') {
    return answer === 'approve'
      ? 'Confirm allow — records your ruling. The terminal session itself advances once the gate is answered in its tab (the confirmation gives you the exact line to paste).'
      : 'Confirm deny — records your ruling. The terminal session itself advances once the gate is answered in its tab (the confirmation gives you the exact line to paste).';
  }
  return answer === 'approve'
    ? 'Confirm — this is recorded for the audit trail; nothing else changes.'
    : 'Confirm reject — this is recorded for the audit trail; nothing else changes.';
}

function scheduledUnblockConfirmText(
  d: Decision,
  answer: 'approve' | 'reject',
): string | null {
  const summary = scheduledUnblockSummary(d);
  if (!summary || !summary.known) return null;
  const line = summary.lines[answer === 'approve' ? 0 : 1];
  if (!line || line === RECORD_ONLY_TEXT) return null;
  const withoutIndex = normalizeEffect(line).replace(/^\d+\.\s*/, '');
  const separator = withoutIndex.indexOf(':');
  const detail = (separator >= 0 ? withoutIndex.slice(separator + 1) : withoutIndex).trim();
  if (!detail) return null;
  return answer === 'approve'
    ? `Confirm approve — ${detail}`
    : `Confirm reject — ${detail}`;
}

// Full tool-call arguments carried by a tool_approval payload / session_gate
// input, pretty-printed for display.
export function toolArgumentsText(d: Decision): string | null {
  if (!d.payload) return null;
  const raw =
    d.kind === 'tool_approval'
      ? (d.payload as { arguments?: unknown }).arguments
      : d.kind === 'session_gate'
        ? (d.payload as { input?: unknown }).input
        : undefined;
  if (raw === undefined || raw === null) return null;
  try {
    return JSON.stringify(raw, null, 2) ?? null;
  } catch {
    return null;
  }
}

function scheduledUnblockSummary(d: Decision): EffectSummary | null {
  if (d.kind !== 'unblock') return null;
  const payload = asObject(d.payload) as ScheduledJobUnblockShape | null;
  if (!payload) return null;

  const target = payload.target;
  const jobId = target && target.type === 'scheduled_job' && typeof target.job_id === 'string'
    ? target.job_id
    : null;
  const episodeId = typeof payload.pause_episode_id === 'string' ? payload.pause_episode_id : null;
  const reason = typeof payload.reason === 'string' ? payload.reason : null;
  const answers = Array.isArray(payload.answers)
    ? payload.answers.filter(a => a && typeof a.id === 'string' && typeof a.label === 'string')
    : [];

  if (!jobId && !episodeId && !reason && answers.length === 0) return null;

  const missing: string[] = [];
  if (!jobId) missing.push('scheduled job target');
  if (!episodeId) missing.push('pause episode id');
  if (!reason) missing.push('pause reason');
  const approveAnswer = answers.find(a => a.id === 'approve');
  const rejectAnswer = answers.find(a => a.id === 'reject');
  if (!approveAnswer || !rejectAnswer) missing.push('resume/keep-paused choices');

  const lines: string[] = [];
  if (approveAnswer && rejectAnswer && jobId && episodeId) {
    lines.push(
      `1. ${approveAnswer.label}: resume the scheduled job "${jobId}" (episode ${episodeId}); this resumes the job and resets consecutive failures and retry count to 0.`,
    );
    lines.push(`2. ${rejectAnswer.label}: keep the scheduled job paused and change nothing else.`);
  } else if (!approveAnswer || !rejectAnswer) {
    lines.push(RECORD_ONLY_TEXT);
  } else {
    lines.push('Approve records a resume decision for this scheduled job pause episode.');
    lines.push('Reject keeps the scheduled job paused.');
  }

  return missing.length > 0 ? { known: true, lines, missing } : { known: true, lines };
}

function choiceSummary(d: Decision): EffectSummary {
  const payload = asObject(d.payload) as ChoiceShape | null;
  const optionsRaw = payload && Array.isArray(payload.options) ? payload.options : [];
  const options = optionsRaw.filter(
    (o): o is { id: string; label: string } => !!o && typeof o.id === 'string' && typeof o.label === 'string',
  );
  const lines: string[] = [];
  const missing: string[] = [];

  if (options.length === 0) {
    missing.push('choice options');
    lines.push('Records your selected choice.');
  } else {
    options.forEach((opt, idx) => {
      const recommended = payload?.default === opt.id ? ' (recommended)' : '';
      lines.push(`${idx + 1}. ${opt.label}${recommended}`);
    });
    lines.push('Records your selected option for this decision.');
  }

  if (payload?.reservation_gate) {
    const gate = payload.reservation_gate;
    const settled = asNumber(gate.settled_usd);
    const held = asNumber(gate.held_usd);
    const requested = asNumber(gate.requested_usd);
    const ceiling = asNumber(gate.ceiling_usd);
    const scope = typeof gate.scope === 'string' && gate.scope.trim() ? gate.scope.trim() : null;
    if (
      settled !== null &&
      held !== null &&
      requested !== null &&
      ceiling !== null &&
      scope
    ) {
      lines.push(
        `Budget context (${scope}): settled ${usd(settled)}, held ${usd(held)}, requested ${usd(requested)}, ceiling ${usd(ceiling)}.`,
      );
    } else {
      if (settled === null) missing.push('settled budget');
      if (held === null) missing.push('held budget');
      if (requested === null) missing.push('requested budget');
      if (ceiling === null) missing.push('budget ceiling');
      if (!scope) missing.push('budget scope');
    }
    lines.push('This records a budget-gate choice; it does not itself promise an automatic convene.');
  }

  return missing.length > 0 ? { known: true, lines, missing } : { known: true, lines };
}

export function effectSummaryFor(d: Decision): EffectSummary {
  const knownKinds = new Set([
    'approve_review',
    'unblock',
    'choice',
    'risk_gate',
    'automation_proposal',
    'enrichment_proposal',
    'project_intel_proposal',
    'file_to_project',
    'tool_approval',
    'session_gate',
    'council_action',
    'malformed',
  ]);

  if (!knownKinds.has(d.kind)) {
    return { known: false, text: RECORD_ONLY_TEXT };
  }

  if (d.kind === 'choice') return choiceSummary(d);

  const scheduled = scheduledUnblockSummary(d);
  if (scheduled) return scheduled;

  const actionClass =
    d.kind === 'risk_gate'
      ? (() => {
          const payload = asObject(d.payload);
          return payload && typeof payload.action_class === 'string' ? payload.action_class : undefined;
        })()
      : undefined;

  // Labelled: the review shows these under one heading, and an unlabelled
  // reject line read as a second consequence of approving.
  const lines = [
    `If you approve: ${normalizeEffect(effectTextFor(d.kind, 'approve', 'your agent', actionClass))}`,
    `If you reject: ${normalizeEffect(effectTextFor(d.kind, 'reject', 'your agent', actionClass))}`,
  ];
  return { known: true, lines };
}
