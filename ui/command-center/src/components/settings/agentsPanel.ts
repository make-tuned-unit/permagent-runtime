/**
 * Honesty helpers for Settings → Agents.
 * Every tri-state API tag maps to distinct copy here so the panel cannot
 * accidentally render unavailable as idle or not_declared as "needs no secrets".
 */

import type {
  Availability,
  DispatchPersona,
  LiveState,
  RequiredSecret,
  RequiredSecrets,
  SecretPresence,
} from '../../lib/agentsApi';

export type LabelTone = 'ok' | 'unknown' | 'error';

export interface StatusLabel {
  text: string;
  tone: LabelTone;
}

/**
 * The one boolean config key that switches an agent on, as the daemon serialises
 * it on a worker row and on a dispatch persona.
 */
export type AgentGate = { config_key: string; enabled: boolean };

/**
 * The daemon may be OLDER than this app, so the switch is validated rather than
 * typed. A missing or malformed gate must read as "no switch known" and render
 * nothing — never as a switch that is off, which would invite the user to flip a
 * control writing a key that nothing on that daemon reads.
 *
 * It takes the whole row rather than a typed `gate`, so the one cast lives at a
 * single validated boundary. `lib/agentsApi.ts` declares the field too — that is
 * what catches a daemon-side RENAME at build time — but a type is a claim about
 * a daemon we have not spoken to yet, so it cannot be the check.
 */
export function readAgentGate(row: unknown): AgentGate | null {
  if (typeof row !== 'object' || row === null) return null;
  const gate = (row as { gate?: unknown }).gate;
  if (typeof gate !== 'object' || gate === null) return null;
  const { config_key, enabled } = gate as { config_key?: unknown; enabled?: unknown };
  if (typeof config_key !== 'string' || config_key.length === 0) return null;
  if (typeof enabled !== 'boolean') return null;
  return { config_key, enabled };
}

/**
 * Names the key, because the whole point of the switch living here is that it
 * is the ONLY one. There used to be two more surfaces writing these keys — the
 * Settings → Features board and the Models pane's Guard block — and both are
 * gone; this hint used to point at Features, which no longer exists.
 */
export function gateRowHint(gate: AgentGate): string {
  return `Writes ${gate.config_key} — the one key that switches this on, and this is the only place it is written. Takes effect at the daemon's next tick, no restart.`;
}

/**
 * "No secrets set" would be an answer to a question nobody asked. The honest
 * answer to "what am I supposed to put here?" is that nothing in the runtime
 * reads a per-agent secret at all, so the field had no consumer to serve.
 */
export const NO_AGENT_SECRETS_NOTE =
  'This agent declares no secrets, and nothing in the runtime reads a per-agent secret. The Guard, for instance, scans on the model and provider key already stored under API keys. There is nothing to enter here.';

/** Stored values stay listed and removable — presence only, never the value. */
export const STORED_SECRETS_NOTE =
  'These values are stored, but nothing in the runtime reads a per-agent secret today. Presence is shown, values never are; Remove deletes.';

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

/**
 * Why grants are not enforced depends on the engine, and the difference is not
 * cosmetic: a CLI engine hands work to a separate process the runtime cannot
 * restrict, while a `pending` persona is registered with no runnable engine at
 * all. Telling a pending persona it "runs an external CLI process" would be a
 * false statement on a page whose whole point is not making them.
 */
export function grantsNotEnforcedNote(engine: string): string {
  switch (engine) {
    case 'external_cli':
    case 'supervised_cli':
      return 'This engine hands work to a separate CLI process that the runtime cannot restrict, so grants here are recorded and not enforced.';
    case 'pending':
      return 'This persona is registered but has no runnable engine yet, so grants here are recorded and nothing enforces them.';
    default:
      return `Grants are not enforced for the ${engineLabel(engine)} engine — they are recorded only.`;
  }
}

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

/** The five words a worker's live status is allowed to say to a human. Every
 *  raw daemon value narrows to exactly one of these — never rendered bare —
 *  with the raw string (a config key, a reason, whatever the daemon actually
 *  said) available only as the caller's tooltip content, never in the chip
 *  itself (ruling 2026-09-25's 1440px review: "off (strix_enabled=false)" and "no live
 *  state to query" were leaking straight onto the row). */
export interface LiveStatusChip {
  label: 'Running' | 'Idle' | 'Off' | 'Unknown' | string;
  tone: LabelTone;
  /** The raw daemon text — a tooltip's content, never rendered directly. */
  detail: string;
}

export function liveStatusChip(s: LiveState): LiveStatusChip {
  const detail = liveStateLabel(s).text;
  if (s.status === 'unavailable') return { label: 'Unknown', tone: 'error', detail };
  if (s.status === 'not_queryable') return { label: 'Idle', tone: 'unknown', detail };
  const raw = s.value.trim();
  const jobs = raw.match(/^(\d+)\s*jobs?$/i);
  if (jobs) {
    const n = Number(jobs[1]);
    return { label: `Scheduled (${n} job${n === 1 ? '' : 's'})`, tone: 'ok', detail };
  }
  if (/^off\b/i.test(raw)) return { label: 'Off', tone: 'unknown', detail };
  if (/^(on|running)\b/i.test(raw)) return { label: 'Running', tone: 'ok', detail };
  if (/^idle$/i.test(raw)) return { label: 'Idle', tone: 'unknown', detail };
  // An 'ok' read with a value shaped like none of the above — still an
  // affirmative signal from the daemon, so "Running" is the honest default
  // rather than inventing a sixth word or leaking the raw value.
  return { label: 'Running', tone: 'ok', detail };
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

/** "degraded" / "unavailable" as the consequence of the secret being absent. */
export function secretImpactLabel(impact: string): string {
  switch (impact) {
    case 'degraded':
      return 'degraded without it';
    case 'unavailable':
      return 'unavailable without it';
    default:
      return `${impact} without it`;
  }
}

/**
 * One hint line per declared secret that ships an impact or unlocks sentence.
 * Configured-transport secrets carry neither and produce no line — the
 * absence of a hint is not a claim that the secret does nothing.
 */
export function requiredSecretHint(secret: RequiredSecret): string | null {
  const parts: string[] = [];
  if (secret.unlocks) parts.push(secret.unlocks);
  if (secret.impact) parts.push(secretImpactLabel(secret.impact));
  if (parts.length === 0) return null;
  return `${secret.name}: ${parts.join(' — ')}`;
}

export function requiredSecretHints(rs: RequiredSecrets): string[] {
  if (rs.status === 'not_declared') return [];
  return rs.items
    .map(requiredSecretHint)
    .filter((h): h is string => h !== null);
}

export function defaultEnabledLabel(value: boolean | null): string {
  if (value === null) return NO_DEFAULT_DECLARED;
  return value ? 'default on' : 'default off';
}

/**
 * First sentence of a longer description, trimmed and capped — the client-side
 * twin of the daemon's own fallback (`agents_surface.rs::first_sentence`). Used
 * only when a worker row arrives with no `summary` at all (an older daemon):
 * the daemon curates a real summary for every worker it knows about, so this
 * exists purely so a client talking to an old daemon degrades to "the first
 * sentence" rather than to the whole paragraph.
 */
export function firstSentence(text: string): string {
  const idx = text.indexOf('. ');
  const cut = idx === -1 ? text : text.slice(0, idx + 1);
  if (cut.length > 160) return `${cut.slice(0, 157)}...`;
  return cut.endsWith('.') ? cut : `${cut}.`;
}

/** The one-sentence line a worker row shows by default (R3 — plain language,
 *  not the agent-facing engineering paragraph). Prefers the daemon's curated
 *  `summary`; falls back to `firstSentence(what_it_does)` only for a daemon
 *  that predates the field. */
export function workerSummary(w: { summary?: string; what_it_does: string }): string {
  return w.summary && w.summary.trim().length > 0 ? w.summary : firstSentence(w.what_it_does);
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
