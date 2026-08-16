/**
 * Honesty helpers for Settings → Agents.
 * Every tri-state API tag maps to distinct copy here so the panel cannot
 * accidentally render unavailable as idle or not_declared as "needs no secrets".
 */

import type {
  ActivitySection,
  Availability,
  DispatchPersona,
  LiveState,
  RequiredSecrets,
  SecretPresence,
} from '../../lib/agentsApi';

export type LabelTone = 'ok' | 'unknown' | 'error';

export interface StatusLabel {
  text: string;
  tone: LabelTone;
}

/** Empty activity must never read as "this agent did nothing". */
export const EMPTY_ACTIVITY_NOTE =
  'No activity is attributed to this agent. Rows written before attribution landed carry a different actor, so this is not proof the agent did nothing.';

export const EMPTY_GOALS_NOTE =
  'No goals are attributed to this agent. Absence here is not proof the agent did nothing.';

export const EMPTY_SPEND_NOTE =
  'No spend is attributed to this agent via its goals. Absence here is not proof the agent spent nothing.';

export const EMPTY_JOBS_NOTE =
  'No scheduled jobs are attributed to this agent. Absence here is not proof the agent has never run on a schedule.';

export const NOT_DECLARED_SECRETS =
  'this capability declares no secret metadata';

export const NO_DEFAULT_DECLARED = 'no default declared';

export const GRANTS_NOT_ENFORCED_NOTE =
  'This engine runs an external CLI process that the runtime cannot restrict, so grants would be recorded and not enforced.';

export function liveStateLabel(s: LiveState): StatusLabel {
  switch (s.status) {
    case 'ok':
      return { text: s.value, tone: 'ok' };
    case 'not_queryable':
      // Property of the worker, not a failure.
      return { text: 'no live state to query', tone: 'unknown' };
    case 'unavailable':
      return {
        text: `state could not be read — ${s.reason}`,
        tone: 'error',
      };
  }
}

export function availabilityLabel(a: Availability): StatusLabel {
  switch (a.status) {
    case 'available':
      return { text: 'available', tone: 'ok' };
    case 'unavailable':
      return { text: `unavailable — ${a.reason}`, tone: 'error' };
    case 'probe_failed':
      // Distinct from unavailable: we could not check, not that it is down.
      return { text: `could not check — ${a.reason}`, tone: 'error' };
  }
}

export function presenceLabel(p: SecretPresence): StatusLabel {
  if (p === 'present') return { text: 'present', tone: 'ok' };
  if (p === 'absent') return { text: 'absent', tone: 'unknown' };
  return { text: `unreadable — ${p.unreadable}`, tone: 'error' };
}

export function grantsSummary(
  persona: Pick<DispatchPersona, 'grants' | 'grants_enforced'>,
): string {
  const { grants, grants_enforced } = persona;
  let text: string;
  if (grants.mode === 'inherit_global') {
    text = 'Inherits globally enabled capabilities';
  } else if (grants.extensions.length === 0) {
    text = 'Grants nothing';
  } else {
    text = `Narrowed to ${grants.extensions.join(', ')}`;
    if (grants.truncated) text += ' (list truncated)';
  }
  if (!grants_enforced) {
    text +=
      ' — grants are recorded but not enforced for this engine';
  }
  return text;
}

export function emptyWorkNote(
  section: ActivitySection | { attribution?: string },
): string {
  // Attribution mode is on the wire so callers can assert it; the user-facing
  // sentence stays fixed so empty never implies "did nothing".
  void section;
  return EMPTY_ACTIVITY_NOTE;
}

export function truncatedNote(count: number): string {
  return `showing the first ${count} — there are more`;
}

export function requiredSecretsLabel(rs: RequiredSecrets): string {
  if (rs.status === 'not_declared') return NOT_DECLARED_SECRETS;
  if (rs.items.length === 0) {
    return rs.truncated
      ? truncatedNote(0)
      : 'declared secret list is empty';
  }
  const parts = rs.items.map(s => `${s.name}: ${s.present ? 'present' : 'absent'}`);
  let text = parts.join(', ');
  if (rs.truncated) text += ` (${truncatedNote(rs.items.length)})`;
  return text;
}

export function defaultEnabledLabel(value: boolean | null): string {
  if (value === null) return NO_DEFAULT_DECLARED;
  return value ? 'default on' : 'default off';
}

export function engineLabel(engine: string): string {
  switch (engine) {
    case 'internal_subagent':
      return 'internal subagent';
    case 'external_cli':
      return 'external CLI';
    case 'supervised_cli':
      return 'supervised CLI';
    case 'pending':
      return 'pending';
    default:
      return engine;
  }
}
