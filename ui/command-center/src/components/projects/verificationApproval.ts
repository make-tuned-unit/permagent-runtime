/**
 * Verification approval ladder — the per-project shell-command allowlist and
 * earned privilege that governs Tier-1 auto-approval of model-authored
 * verification checks (crates/goose/src/verification_approval/).
 *
 * READ:  GET /api/projects/{id} → metadataJson.verification_approval
 *        (camelCase on the wire, every field optional/defaulted).
 * WRITE: PUT /api/projects/{id}/verification-approval — a server-side MERGE,
 *        same contract as PUT /api/projects/{id}/brand: send ONLY the fields
 *        being changed. `{ reset: true }` restores defaults (empty allowlist,
 *        cleanRuns 0, default thresholds, cleared grants) while preserving
 *        audit history. NEVER PATCH metadataJson directly for this bag — the
 *        merge-write endpoint is the one writer (one concept, one place).
 */

import { apiFetch } from '../../lib/api';
import type { Project } from './types';

export const DEFAULT_READ_ONLY_THRESHOLD = 5;
export const DEFAULT_FULL_THRESHOLD = 20;

export type PrivilegeLevel = 'none' | 'read_only' | 'full';

export interface AuditRow {
  at: string;
  command: string;
  cwd?: string;
  tier: 'auto' | 'agent_trust' | 'user';
  decision:
    | 'auto'
    | 'user_authored'
    | 'agent_approved'
    | 'approved_once'
    | 'approved_and_allowlisted'
    | 'parked'
    | 'denied';
  privilege: number;
  level: PrivilegeLevel;
  reason: string;
  deny?: string;
  goal_id?: string;
}

export interface VerificationApproval {
  allowlist: string[];
  cleanRuns: number;
  readOnlyThreshold: number;
  fullThreshold: number;
  onceGrants: string[];
  /** Server order is newest-last; callers wanting newest-first should reverse. */
  audit: AuditRow[];
}

const EMPTY: VerificationApproval = {
  allowlist: [],
  cleanRuns: 0,
  readOnlyThreshold: DEFAULT_READ_ONLY_THRESHOLD,
  fullThreshold: DEFAULT_FULL_THRESHOLD,
  onceGrants: [],
  audit: [],
};

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function stringArray(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
}

/** Tolerant read of metadataJson.verification_approval — defaults fill in
 *  anything absent or malformed (the bag is agent-writable and shared). */
export function readVerificationApproval(meta: unknown): VerificationApproval {
  if (!isRecord(meta)) return EMPTY;
  const raw = meta['verification_approval'];
  if (!isRecord(raw)) return EMPTY;
  const audit = Array.isArray(raw.audit)
    ? raw.audit.filter((r): r is Record<string, unknown> => isRecord(r)).map(
        (r): AuditRow => ({
          at: typeof r.at === 'string' ? r.at : '',
          command: typeof r.command === 'string' ? r.command : '',
          cwd: typeof r.cwd === 'string' ? r.cwd : undefined,
          tier: (r.tier as AuditRow['tier']) ?? 'user',
          decision: (r.decision as AuditRow['decision']) ?? 'parked',
          privilege: typeof r.privilege === 'number' ? r.privilege : 0,
          level: (r.level as PrivilegeLevel) ?? 'none',
          reason: typeof r.reason === 'string' ? r.reason : '',
          deny: typeof r.deny === 'string' ? r.deny : undefined,
          goal_id: typeof r.goal_id === 'string' ? r.goal_id : undefined,
        }),
      )
    : [];
  return {
    allowlist: stringArray(raw.allowlist),
    cleanRuns: typeof raw.cleanRuns === 'number' ? raw.cleanRuns : 0,
    readOnlyThreshold:
      typeof raw.readOnlyThreshold === 'number' ? raw.readOnlyThreshold : DEFAULT_READ_ONLY_THRESHOLD,
    fullThreshold: typeof raw.fullThreshold === 'number' ? raw.fullThreshold : DEFAULT_FULL_THRESHOLD,
    onceGrants: stringArray(raw.onceGrants),
    audit,
  };
}

/**
 * Earned privilege derived from clean runs vs the two thresholds — pure, so
 * the UI and any future caller agree on the exact boundary (>=, not >).
 */
export function derivePrivilegeLevel(
  cleanRuns: number,
  readOnlyThreshold: number,
  fullThreshold: number,
): PrivilegeLevel {
  // A threshold of 0 reads as "nobody has cleared this bar", never as "everybody
  // has" — the same fail-closed reading as ApprovalSettings::level() in Rust.
  if (fullThreshold > 0 && cleanRuns >= fullThreshold) return 'full';
  if (readOnlyThreshold > 0 && cleanRuns >= readOnlyThreshold) return 'read_only';
  return 'none';
}

/** What each privilege level actually permits — plain language for the panel. */
export function privilegeLevelBlurb(level: PrivilegeLevel): string {
  switch (level) {
    case 'full':
      return 'Full — an unrecognised command runs without asking, whether or not it looks read-only. Denied commands (network tools, deletions outside the project, privilege escalation) still ask, at every level.';
    case 'read_only':
      return 'Read-only — an unrecognised command runs without asking only if it carries no write flags and no redirects. Anything else asks.';
    default:
      return 'None — every command outside the allowlist asks before it runs.';
  }
}

export interface VerificationApprovalSave {
  allowlist?: string[];
  readOnlyThreshold?: number;
  fullThreshold?: number;
  cleanRuns?: number;
  reset?: boolean;
}

/** GET /api/projects/{id} → the verification_approval bag (defaulted). */
export async function fetchVerificationApproval(projectId: string): Promise<VerificationApproval> {
  const id = encodeURIComponent(projectId);
  const project = await apiFetch<Project>(`/api/projects/${id}`);
  return readVerificationApproval(project.metadataJson);
}

/**
 * PUT /api/projects/{id}/verification-approval — send only the changed
 * fields; the server merges. Returns the fresh bag from the updated project.
 */
export async function saveVerificationApproval(
  projectId: string,
  changes: VerificationApprovalSave,
): Promise<VerificationApproval> {
  const id = encodeURIComponent(projectId);
  const project = await apiFetch<Project>(`/api/projects/${id}/verification-approval`, {
    method: 'PUT',
    body: JSON.stringify(changes),
  });
  return readVerificationApproval(project.metadataJson);
}
