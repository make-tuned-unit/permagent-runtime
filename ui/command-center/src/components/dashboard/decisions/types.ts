/**
 * Decision Inbox — shared types (Lane L4).
 *
 * Shape mirrors Lane L1's API contract:
 *   GET  /api/decisions
 *   POST /api/decisions/{id}/answer
 *   GET  /api/decisions/history
 *
 * Plain-language rules (Jesse's amendments):
 *  - `headline` is a plain-language outcome statement; `detail` is the
 *    secondary line. The UI NEVER synthesizes headlines from technical fields.
 *  - Internal worker ids never appear in user copy; they live only in the raw
 *    evidence layer.
 *  - ALL decision text is untrusted (S2): rendered as React text nodes only —
 *    no markdown pipeline, no auto-linking, no dangerouslySetInnerHTML.
 */

export type DecisionTier = 1 | 2;
export type DecisionKind = 'approval' | 'choice' | 'unblock' | 'fyi';

export interface DecisionOption {
  id: string;
  label: string;
  effect_summary: string;
}

export interface DecisionRecommendation {
  /** Plain-language recommended action, e.g. "Approve" or an option label. */
  label: string;
  confidence?: 'low' | 'medium' | 'high';
}

/** Machine-assembled plain-language summary (Lane L2). Leads with dollars. */
export interface DecisionEvidenceSummary {
  checks_passed: number;
  checks_total: number;
  tests_passed: number;
  verifier_confirmed: boolean;
  /** Plain words, e.g. "Henry's reviewer confirmed the work matches what was approved" */
  verifier_summary: string;
  cost_usd: number;
}

/** Raw output layers — plain text, untrusted, collapsed behind "Show details". */
export interface DecisionEvidenceRaw {
  checks: string;
  diff: string;
  verifier: string;
  /** Token counts / session detail — only shown in this layer. */
  cost_detail: string;
}

export interface DecisionEvidence {
  summary: DecisionEvidenceSummary;
  raw: DecisionEvidenceRaw;
}

export interface Decision {
  id: string;
  tier: DecisionTier;
  kind: DecisionKind;
  /** Plain-language outcome statement (A1). Rendered as the item title. */
  headline: string;
  /** Secondary plain-language line (A1). */
  detail: string;
  /** Unblock items: the worker's question, rendered verbatim and quoted. */
  specific_ask?: string;
  /** "Approving will: ..." / "Answering will: ..." line, verbatim. */
  effect_summary: string;
  recommendation?: DecisionRecommendation;
  /** Present only on choice-kind decisions (and option-style unblocks). */
  options?: DecisionOption[];
  evidence?: DecisionEvidence;
  /** Agent display name where known; copy falls back to "one of Henry's workers". */
  agent_name?: string;
  created_at: string;
  resolved_at?: string;
  /** History/Tier-1 rows: plain-language record of what happened. */
  resolution?: string;
}

export interface DecisionsResponse {
  /** Ranked, server-ordered. Render as given. */
  decisions: Decision[];
  total_pending: number;
  /** Tier-1 items Henry auto-handled since last view. */
  handled_count: number;
  goals_in_flight: number;
  oldest_pending_at: string | null;
}

export type AnswerKind = 'approve' | 'reject' | 'choice' | 'input';

export interface AnswerBody {
  answer: AnswerKind;
  note?: string;
  choice_id?: string;
  input_text?: string;
}

export interface DecisionHistoryResponse {
  items: Decision[];
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
  answer(id: string, body: AnswerBody): Promise<Decision>;
  history(): Promise<DecisionHistoryResponse>;
}
