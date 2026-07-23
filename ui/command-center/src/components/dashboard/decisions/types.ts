/**
 * Decision Inbox — shared types (Lane L4).
 *
 * These mirror Lane L1's ACTUAL wire format, field for field. Source of truth:
 *   crates/goose-server/src/routes/decisions.rs  (handlers + envelopes)
 *   crates/goose/src/decisions.rs                (Decision/InboxSummary/HistoryItem)
 *   crates/goose-server/src/verification/digest.rs (L2 evidence digest)
 *
 * Serde reality check (verified against the Rust derives):
 *  - `Decision`, `InboxSummary`, `OpenDecisionItem`, `HistoryItem` derive
 *    Serialize with NO rename_all → snake_case on the wire
 *    (decisions.rs:97-116, 378-392, 451-459).
 *  - The envelopes (`InboxResponse`, `AnswerResponse`, `HistoryResponse`) and
 *    the answer REQUEST are rename_all = "camelCase"
 *    (routes/decisions.rs:46-85) — so the POST body uses choiceId/inputText
 *    and the answer response uses effectError.
 *
 * Plain-language rules (Jesse's amendments):
 *  - `headline`/`detail` come verbatim from the daemon (A1) — the UI NEVER
 *    synthesizes headlines from technical fields.
 *  - ALL decision text is untrusted (S2): rendered as React text nodes only —
 *    no markdown pipeline, no auto-linking, no dangerouslySetInnerHTML.
 */

/** Decision kinds (decisions.rs kind vocabulary; escalate.rs:109-127).
 *  'session_gate' (S3, #429): a supervised terminal Claude Code session is
 *  blocked on a can_use_tool permission gate; payload carries
 *  {question, target_session_id, pty_session_id, request_id, tool_name,
 *   input, tool_use_id, options}. */
export type DecisionKind =
  | 'approve_review'
  | 'choice'
  | 'unblock'
  | 'risk_gate'
  | 'automation_proposal'
  | 'enrichment_proposal'
  | 'project_intel_proposal'
  | 'file_to_project'
  | 'tool_approval'
  | 'session_gate'
  | 'malformed';

/** One selectable option (decisions.rs:71-74 — {id, label} only). */
export interface ChoiceOption {
  id: string;
  label: string;
}

/**
 * A decision row exactly as serialized by the daemon (decisions.rs:97-116,
 * snake_case). Open-list items additionally carry `goal_title`
 * (decisions.rs:388-392, flattened join).
 */
export interface Decision {
  id: string;
  /** Real kinds in DecisionKind; tolerate future additions. */
  kind: DecisionKind | (string & {});
  goal_id: string | null;
  project_id: string | null;
  /** 1 = Henry may auto-answer; 2 = requires Jesse. */
  tier: number;
  /** Plain-language outcome statement (A1). Rendered as the item title. */
  headline: string;
  /** Technical detail: why blocked, attribution, evidence refs (verbatim). */
  detail: string;
  /** Kind-typed payload (decisions.rs:29-93). Untrusted; read via helpers. */
  payload: Record<string, unknown> | null;
  rank: number | null;
  /** 'open' | 'answered' | ... */
  status: string;
  answer: string | null;
  answer_note: string | null;
  answer_choice_id: string | null;
  answer_input: string | null;
  acted_by: string | null;
  created_at: string;
  resolved_at: string | null;
  /** Joined goal title — present on GET /api/decisions items only. */
  goal_title?: string | null;
}

/** Summary envelope beside the open list (decisions.rs:378-384). */
export interface InboxSummary {
  total_pending: number;
  handled_count: number;
  goals_in_flight: number;
  oldest_pending_at: string | null;
}

/** UI-side flattening of GET /api/decisions (routes/decisions.rs:46-51). */
export interface DecisionsResponse {
  /** Ranked, server-ordered (top 10 unless ?all=1). Render as given. */
  decisions: Decision[];
  total_pending: number;
  handled_count: number;
  goals_in_flight: number;
  oldest_pending_at: string | null;
}

// 'edit' = approve-with-edits: an acceptance that carries the user's revised
// draft in input_text (the original is in payload.draft). The daemon stores it
// as answer='edit' and learns the draft→revision delta (edit-as-training,
// crates/goose/src/decision_inbox/learn.rs).
export type AnswerKind = 'approve' | 'reject' | 'choice' | 'input' | 'edit';

/**
 * UI-side answer body. The client maps to the wire's camelCase
 * (choiceId/inputText — routes/decisions.rs:53-60).
 */
export interface AnswerBody {
  answer: AnswerKind;
  note?: string;
  choice_id?: string;
  input_text?: string;
}

/** POST answer result (routes/decisions.rs:62-71; effectError → effect_error). */
export interface AnswerOutcome {
  decision: Decision;
  /** What the gated effect did, e.g. "goal approved: Review → Complete". */
  effect: string | null;
  /** Answer committed but the effect failed (also audit-logged). */
  effect_error: string | null;
}

/** History row: audit join + flattened decision (decisions.rs:451-459). */
export interface HistoryItem extends Decision {
  seq: number;
  outcome: string;
  audit_acted_by: string;
  audit_created_at: string;
}

/** GET /api/decisions/history (routes/decisions.rs:79-85; nextBefore mapped). */
export interface DecisionHistoryResponse {
  items: HistoryItem[];
  next_before: number | null;
}

// ── L2 evidence digest (verification/digest.rs:24-107, snake_case) ──────────
// Reached via the goal card: GET /api/projects/{pid}/cards/{gid} →
// metadataJson.dispatch_evidence.verdict.evidence_digest — the single
// canonical evidence record (#466/#458); metadataJson.verification remains a
// read-only legacy fallback for goals verified before the consolidation.
// (CardResponse is camelCase, routes/cards.rs:57-73; the record itself is
// snake_case.)

export interface ChecksSummary {
  passed_count: number;
  total_count: number;
  tests_run?: number | null;
  /** Server-built one-liner, e.g. "All 4 automated checks passed (412 tests)". */
  one_line: string;
}

export interface Costs {
  /** Dollars first; null when no token rate is configured (note explains). */
  cost_usd: number | null;
  accumulated_total_tokens?: number | null;
  worker_session_id?: string | null;
  attempt_count: number;
  note?: string | null;
}

export interface DigestCheck {
  type: string;
  summary: string;
  status: 'pass' | 'fail' | 'error';
  output_excerpt: string;
}

export interface PerFileDiff {
  path: string;
  insertions: number;
  deletions: number;
}

export interface DiffSummary {
  files_changed: number;
  insertions: number;
  deletions: number;
  per_file: PerFileDiff[];
}

export interface VerifierDetail {
  status: 'pass' | 'fail' | 'uncertain';
  rationale: string;
  model: string;
  degraded_reason?: string | null;
}

export interface EvidenceDigestData {
  version: number;
  goal_id: string;
  goal_title: string;
  // Summary layer (rendered first — A3)
  checks_summary: ChecksSummary;
  verifier_summary: string;
  costs: Costs;
  // Detail layer
  checks: DigestCheck[];
  diff: DiffSummary;
  out_of_path_files: string[];
  verifier: VerifierDetail;
  started_at: string;
  finished_at: string;
}

// ── Dispatch evidence (deterministic proof-of-work, goal_engine::GoalEvidence) ─
// Reached via the goal card: GET /api/projects/{pid}/cards/{gid} →
// metadataJson.dispatch_evidence. Captured at completion for external-CLI goals
// (commit SHAs, diffstat, push target, worker summary) — zero LLM, always
// present for a worktree+push dispatch even when the L2 digest is absent.
// Source of truth: crates/goose/src/agents/platform_extensions/goal_engine.rs.

export interface DispatchEvidenceData {
  worktree_path: string;
  baseline_commit: string;
  head_commit?: string | null;
  /** "<short-sha> <subject>", newest first. */
  commits: string[];
  /** git diff --stat baseline..HEAD (truncated). */
  diffstat: string;
  files_changed: number;
  insertions: number;
  deletions: number;
  /** Remote ref the work was pushed to, or null (worktree only). */
  push_target?: string | null;
  /** Tail of the worker's own stdout — its self-reported summary. */
  worker_summary: string;
}

// ── Payload helpers (typed reads of the untrusted payload) ──────────────────

/** Options for a choice decision (ChoicePayload.options, decisions.rs:77-84). */
export function choiceOptions(d: Decision): ChoiceOption[] {
  if (d.kind !== 'choice' || !d.payload) return [];
  const raw = (d.payload as { options?: unknown }).options;
  if (!Array.isArray(raw)) return [];
  return raw.filter(
    (o): o is ChoiceOption =>
      !!o && typeof (o as ChoiceOption).id === 'string' && typeof (o as ChoiceOption).label === 'string',
  );
}

/** Default option id for a choice decision (ChoicePayload.default). */
export function recommendedChoiceId(d: Decision): string | null {
  if (d.kind !== 'choice' || !d.payload) return null;
  const def = (d.payload as { default?: unknown }).default;
  return typeof def === 'string' ? def : null;
}

/**
 * The agent's original draft for an editable decision (payload.draft). Present
 * on draft-carrying kinds (e.g. automation_proposal); null otherwise. When
 * present, the item offers "approve with edits": the user revises this text and
 * accepts (answer='edit'), and the daemon learns the draft→revision delta
 * (edit-as-training, decision_inbox/learn.rs).
 */
export function draftText(d: Decision): string | null {
  if (!d.payload) return null;
  const raw = (d.payload as { draft?: unknown }).draft;
  return typeof raw === 'string' && raw.trim() ? raw : null;
}

/**
 * Plain-language record of a resolved decision for history rows. Verb mapping
 * from the answer enum only (approve|reject|choice|input — decisions.rs:27);
 * free text (note, option labels, input) is appended verbatim.
 */
export function resolutionText(d: Decision, agentName = 'Aria'): string {
  const verb =
    d.answer === 'approve' ? 'Approved'
    : d.answer === 'reject' ? 'Rejected'
    : d.answer === 'edit' ? 'Accepted with edits'
    : d.answer === 'choice' ? `Chose "${d.answer_choice_id ?? 'an option'}"`
    : d.answer === 'input' ? 'Answered with a written reply'
    : 'Resolved';
  const by =
    d.acted_by === 'henry-policy' ? ` by ${agentName}`
    : d.acted_by === 'system' ? ' automatically'
    : '';
  return d.answer_note ? `${verb}${by} — note: ${d.answer_note}` : `${verb}${by}`;
}

/** Thrown when the decision was already resolved elsewhere (HTTP 409). */
export class DecisionConflictError extends Error {
  constructor() {
    super('decision already resolved');
    this.name = 'DecisionConflictError';
  }
}

/**
 * The hook seam: useDecisions() talks to this interface only, so the mock
 * server (VITE_DECISIONS_MOCK=1) and the real daemon client are drop-in swaps.
 */
export interface DecisionsClient {
  list(opts?: { all?: boolean }): Promise<DecisionsResponse>;
  answer(id: string, body: AnswerBody): Promise<AnswerOutcome>;
  history(): Promise<DecisionHistoryResponse>;
  /** L2 digest for a goal-bound decision; null when none recorded yet. */
  evidence(projectId: string, goalId: string): Promise<EvidenceDigestData | null>;
  /** Deterministic dispatch proof-of-work; null when none recorded yet. */
  dispatchEvidence(projectId: string, goalId: string): Promise<DispatchEvidenceData | null>;
  /** Cancel the decision's goal (#490): kills the worker and marks it terminal. */
  cancelGoal(projectId: string, goalId: string): Promise<void>;
}
