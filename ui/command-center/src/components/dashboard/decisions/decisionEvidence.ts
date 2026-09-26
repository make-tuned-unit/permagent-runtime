/**
 * What a decision says about itself, in plain words — pure functions only.
 *
 * The inbox body reads eyebrow (kind + age) → title → cause → consequences →
 * evidence → technical details. Everything here feeds one of those rows and is
 * a pure function of the decision row, so each fallback is testable on its own:
 *
 *  - `kindLabel`      the eyebrow's human kind name ("Goal ran out of time"),
 *                     never a wire id like `unblock` or `tier 1` (U15).
 *  - `decisionTitle`  the goal's full title, else the untruncated headline (U2).
 *  - `plainCause`     the daemon's `summary` when it sends one, else a local
 *                     classifier over kind + payload (+ detail for salvage
 *                     only). It NEVER returns `detail` or any slice of it (U1).
 *  - `technicalDetails` the level-2 disclosure: verbatim detail, ids, raw error.
 *  - `evidenceTargetFor` where "Open full evidence…" goes (U2, U16).
 */
import type { Decision } from './types';

export type EvidenceTarget =
  | { kind: 'council_report'; councilSessionId: string; label: string }
  | { kind: 'dispatch'; projectId: string; goalId: string; label: string }
  | { kind: 'pause_episode'; jobId: string; episodeId: string; label: string }
  | { kind: 'budget'; label: string; detail: string }
  | { kind: 'none'; reason: string };

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function asText(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function usd(value: number): string {
  return `$${value.toFixed(2)}`;
}

/** Fixed label: the goal title already heads the sheet, so the button never
 *  repeats it (U2). */
export const OPEN_EVIDENCE_LABEL = 'Open full evidence…';

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
    const jobId = scheduledJobId(d);
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
      label: OPEN_EVIDENCE_LABEL,
    };
  }

  if (!payload) {
    return { kind: 'none', reason: 'Decision has no payload or bound goal evidence.' };
  }
  return { kind: 'none', reason: `No evidence target is defined for kind "${d.kind}".` };
}

/** The scheduled job an unblock pauses, or null for a goal unblock. */
export function scheduledJobId(d: Decision): string | null {
  if (d.kind !== 'unblock') return null;
  const target = asObject(asObject(d.payload)?.target);
  return target && target.type === 'scheduled_job' && typeof target.job_id === 'string'
    ? target.job_id
    : null;
}

/** The typed `UnblockPayload.reason` (decisions.rs:248-268), or null. */
export function unblockReason(d: Decision): string | null {
  if (d.kind !== 'unblock') return null;
  return asText(asObject(d.payload)?.reason);
}

// ── Durations and ages (R3: "3 min", "5 hr ago", never "240s") ─────────────

/** Seconds → "45 sec", "4 min", "1 hr 5 min", "2 days". */
export function humanDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} sec`;
  const min = Math.round(s / 60);
  if (min < 60) return `${min} min`;
  const hr = Math.floor(min / 60);
  const rem = min % 60;
  if (hr < 24) return rem ? `${hr} hr ${rem} min` : `${hr} hr`;
  const days = Math.round(hr / 24);
  return `${days} day${days === 1 ? '' : 's'}`;
}

/** "just now", "13 min ago", "5 hr ago", "2 days ago". */
export function ageLabel(iso: string, now: number = Date.now()): string {
  const ms = now - new Date(iso).getTime();
  if (!Number.isFinite(ms)) return '';
  const min = Math.floor(ms / 60_000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min} min ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr} hr ago`;
  const days = Math.floor(hr / 24);
  return `${days} day${days === 1 ? '' : 's'} ago`;
}

/** Absolute timestamp for the age Tooltip. */
export function absoluteTime(iso: string): string {
  const t = new Date(iso);
  if (Number.isNaN(t.getTime())) return iso;
  return t.toLocaleString(undefined, {
    weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
  });
}

function compactCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 10_000) return `${Math.round(n / 1_000)}K`;
  return n.toLocaleString('en-US');
}

// ── Eyebrow ───────────────────────────────────────────────────────────────

const UNBLOCK_KIND_LABELS: Record<string, string> = {
  wallclock_cap: 'Goal ran out of time',
  token_budget: 'Goal hit its spending limit',
  attempt_cap: 'Goal ran out of attempts',
  stuck: 'Goal stopped making progress',
  refinement_budget: 'Goal ran out of revisions',
};

const KIND_LABELS: Record<string, string> = {
  approve_review: 'Work ready for review',
  choice: 'Pick an option',
  risk_gate: 'Permission needed',
  automation_proposal: 'Automation suggestion',
  enrichment_proposal: 'Contact details found',
  project_intel_proposal: 'Project notes found',
  file_to_project: 'File to a project',
  council_action: 'Council proposal',
  tool_approval: 'Tool approval',
  session_gate: 'Terminal permission',
  malformed: 'Unreadable request',
  model_upgrade: 'Model upgrade',
  config_change_proposal: 'Setting change',
  person_merge_proposal: 'Possible duplicate contact',
  person_delete_proposal: 'Contact deletion',
  capability_gap: 'Missing capability',
  regression_proposal: 'Possible regression',
};

/** The human kind name shown in the eyebrow. Never a wire id (U15). */
export function kindLabel(d: Decision): string {
  if (d.kind === 'unblock') {
    if (scheduledJobId(d)) return 'Scheduled job paused';
    return UNBLOCK_KIND_LABELS[unblockReason(d) ?? ''] ?? 'Goal needs direction';
  }
  if (d.kind === 'choice' && asObject(d.payload)?.proposal === 'check_approval') return 'Command approval';
  if (d.kind === 'risk_gate' && asObject(d.payload)?.action_class === 'user_data_deletion') return 'Deletion request';
  return KIND_LABELS[d.kind] ?? 'Needs you';
}

// ── Title ─────────────────────────────────────────────────────────────────

/**
 * The goal's own full title when the decision is bound to one (the daemon's
 * headline quotes it and cuts it at 80 chars, D1), else the headline. Shown
 * once, in the title row, and nowhere else (U2).
 */
export function decisionTitle(d: Decision): string {
  const goal = asText(d.goal_title);
  if (goal) return goal;
  return d.headline.trim();
}

// ── Cause ─────────────────────────────────────────────────────────────────

/** The salvage note the orchestrator appends to a parked goal's last error
 *  (orchestrator.rs:23782). Read, never shown. */
const NOTHING_SALVAGED = /no commits past the (dispatch )?baseline|nothing to salvage/i;

function sentence(text: string): string {
  const t = text.trim();
  return /[.!?…]$/.test(t) ? t : `${t}.`;
}

/** Short enough, single-line agent prose from a payload field. Anything long
 *  or multi-line is treated as machine output and kept out of the body. */
function shortProse(value: unknown, max = 180): string | null {
  const text = asText(value);
  if (!text || text.length > max || text.includes('\n') || text.includes('{')) return null;
  return sentence(text);
}

function unblockCause(d: Decision, agentName: string): string[] {
  const payload = asObject(d.payload);
  if (scheduledJobId(d)) {
    return unblockReason(d) === 'consecutive_failure_limit'
      ? ['This scheduled job paused itself after failing several times in a row.', `${agentName} needs your go-ahead to resume it.`]
      : ['This scheduled job is paused.', `${agentName} needs your go-ahead to resume it.`];
  }
  const spent = asNumber(payload?.spent);
  const cap = asNumber(payload?.cap);
  let first: string;
  switch (unblockReason(d)) {
    case 'wallclock_cap':
      first = cap !== null
        ? `Ran out of time: each attempt gets ${humanDuration(cap)}, and this one used all of it.`
        : 'Ran out of time before it finished.';
      break;
    case 'token_budget':
      first = spent !== null && cap !== null
        ? `Hit its spending limit: used ${compactCount(spent)} of ${compactCount(cap)} tokens.`
        : 'Hit its spending limit.';
      break;
    case 'attempt_cap':
      first = spent !== null && cap !== null
        ? `Tried ${spent} times without finishing (limit ${cap}).`
        : 'Used all its attempts without finishing.';
      break;
    case 'stuck':
      first = 'Stopped making progress and paused itself.';
      break;
    case 'refinement_budget':
      first = cap !== null
        ? `Used all ${cap} revision rounds without passing review.`
        : 'Used all its revision rounds without passing review.';
      break;
    default:
      first = 'Paused before it finished.';
  }
  // The salvage note is read from the technical text to choose a sentence;
  // the text itself never reaches the body.
  const technical = [d.detail, d.technical?.detail, d.technical?.last_error].filter(Boolean).join('\n');
  const second = NOTHING_SALVAGED.test(technical)
    ? 'It made no changes worth keeping.'
    : `${agentName} needs your direction.`;
  return [first, second];
}

/**
 * The body's cause: at most two sentences, plain words, no ids, never the raw
 * `detail` (U1, R3). The daemon's `summary` wins when present.
 */
export function plainCause(d: Decision, agentName = 'Your agent'): string {
  const fromDaemon = asText(d.summary);
  if (fromDaemon) return fromDaemon;
  const payload = asObject(d.payload);
  const parts: string[] = (() => {
    switch (d.kind) {
      case 'unblock':
        return unblockCause(d, agentName);
      case 'approve_review':
        return [`${agentName} finished this and it's waiting for your review.`];
      case 'choice':
        if (payload?.proposal === 'check_approval') {
          return ['A verification check wants to run a command that needs your say-so first.'];
        }
        return [`${agentName} needs you to pick one option before continuing.`];
      case 'risk_gate': {
        const what = shortProse(payload?.description);
        if (payload?.action_class === 'user_data_deletion') {
          return [`${agentName} wants to permanently delete something and needs your permission.`];
        }
        return [`${agentName} wants to do something that needs your permission first.`, ...(what ? [what] : [])];
      }
      case 'automation_proposal': {
        const n = asNumber(payload?.occurrence_count);
        return n !== null
          ? [`You've run the same command ${n} times recently.`, `${agentName} can automate it for you.`]
          : [`${agentName} spotted a routine it could automate for you.`];
      }
      case 'enrichment_proposal': {
        const name = asText(payload?.person_name);
        return [`${agentName} found new details for ${name ?? 'a contact'} and wants to save them.`];
      }
      case 'project_intel_proposal':
        return [`${agentName} found notes worth adding to this project.`];
      case 'file_to_project': {
        const project = asText(payload?.project_name);
        return [`${agentName} wants to file this item under ${project ?? 'a project'}.`];
      }
      case 'council_action':
        return ['The Council proposed an action and wants a board card for it.'];
      case 'tool_approval': {
        const tool = asText(payload?.tool_name);
        return [`${agentName} wants to run ${tool ? humanToolName(tool) : 'a tool'} and needs your okay first.`];
      }
      case 'session_gate': {
        const tool = asText(payload?.tool_name);
        return [`A terminal session is waiting for your permission to use ${tool ? humanToolName(tool) : 'a tool'}.`];
      }
      case 'malformed':
        return [`${agentName} received a request it couldn't read.`, 'Dismiss it to clear your queue.'];
      case 'model_upgrade': {
        const why = shortProse(payload?.why_better);
        return [`${agentName} found a model that may suit you better.`, ...(why ? [why] : [])];
      }
      case 'config_change_proposal': {
        const why = shortProse(payload?.rationale);
        return [`${agentName} suggests changing a setting it isn't allowed to change itself.`, ...(why ? [why] : [])];
      }
      case 'person_merge_proposal': {
        const a = asText(payload?.duplicate_name);
        const b = asText(payload?.survivor_name);
        return a && b
          ? [`${agentName} thinks ${a} and ${b} are the same person.`]
          : [`${agentName} thinks two of your contacts are the same person.`];
      }
      case 'person_delete_proposal': {
        const name = asText(payload?.display_name);
        const why = shortProse(payload?.reason);
        return [`${agentName} suggests deleting ${name ?? "a contact's"} record.`, ...(why ? [why] : [])];
      }
      case 'capability_gap':
        return [`${agentName} hit something it can't do yet.`];
      case 'regression_proposal':
        return [`${agentName} spotted a possible regression.`];
      default:
        return [`${agentName} needs you to look at this.`];
    }
  })();
  return parts.slice(0, 2).join(' ');
}

/** `developer__shell` → "shell"; `github__create_issue` → "create issue". */
export function humanToolName(tool: string): string {
  const last = tool.split('__').pop() ?? tool;
  return last.replace(/[_-]+/g, ' ').trim() || tool;
}

// ── Technical details (level 2) ─────────────────────────────────────────

export interface TechnicalDetails {
  detail: string | null;
  ids: Array<{ label: string; value: string }>;
  lastError: string | null;
}

function normalizeIds(raw: unknown): Array<{ label: string; value: string }> {
  if (Array.isArray(raw)) {
    return raw.flatMap(item => {
      const o = asObject(item);
      const label = asText(o?.label);
      const value = asText(o?.value);
      return label && value ? [{ label, value }] : [];
    });
  }
  const o = asObject(raw);
  if (!o) return [];
  return Object.entries(o).flatMap(([label, value]) =>
    typeof value === 'string' && value.trim() ? [{ label, value: value.trim() }] : []);
}

/**
 * Everything machine-shaped about a decision, for the collapsed "Technical
 * details" disclosure: the daemon's `technical` block when present, else the
 * verbatim `detail` and the ids already on the row.
 */
export function technicalDetails(d: Decision): TechnicalDetails {
  const payload = asObject(d.payload);
  const fallbackIds: Array<{ label: string; value: string }> = [{ label: 'Decision', value: d.id }];
  if (d.goal_id) fallbackIds.push({ label: 'Goal', value: d.goal_id });
  if (d.project_id) fallbackIds.push({ label: 'Project', value: d.project_id });
  const jobId = scheduledJobId(d);
  if (jobId) fallbackIds.push({ label: 'Scheduled job', value: jobId });
  for (const [key, label] of [
    ['pause_episode_id', 'Pause episode'], ['session_id', 'Session'], ['request_id', 'Request'],
    ['tool_use_id', 'Tool call'], ['target_session_id', 'Terminal session'],
  ] as const) {
    const value = asText(payload?.[key]);
    if (value) fallbackIds.push({ label, value });
  }
  const technical = d.technical ?? null;
  const daemonIds = normalizeIds(technical?.ids);
  const seen = new Set(daemonIds.map(i => i.value));
  const ids = [...daemonIds, ...fallbackIds.filter(i => !seen.has(i.value))];
  return {
    detail: asText(technical?.detail) ?? asText(d.detail),
    ids,
    lastError: asText(technical?.last_error),
  };
}
