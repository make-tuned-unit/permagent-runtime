/**
 * Decision Inbox — single decision row (Lane L4).
 *
 * Plain words first (headline → detail), technical detail one tap deeper
 * (evidence summary), raw output one tap below that (Show details).
 * Tier-2 answers are confirmed individually, inline — there is deliberately
 * no checkbox, no select-all, no batch affordance.
 *
 * Kinds and effects mirror L1's handler (routes/decisions.rs:170-335):
 *  - approve_review: approve → Review→Complete; reject → rework (or park at
 *    attempt cap). Evidence digest available via the goal card (L2).
 *  - unblock: approve → unpark with a raised attempt cap; reject/input →
 *    recorded, goal stays parked. Freeform replies travel as answer='input'.
 *  - choice: answer='choice' + choiceId from payload.options
 *    (decisions.rs:625-643); payload.default marks Henry's recommendation.
 *  - risk_gate: approve authorizes the gated action class; reject records.
 *  - enrichment_proposal: approve writes the proposed person fields with
 *    Enriched provenance (manual entries protected); reject records, and a
 *    find-online hint in the note is written to the person as a manual field.
 *  - file_to_project: approve files the content as a project note (Brain-
 *    indexed) and adds named people address-less; reject persists nothing.
 *  - malformed: acknowledgement only — recorded, no state change.
 *
 * S2: every string from the daemon renders as a React text node. No markdown,
 * no auto-links, no dangerouslySetInnerHTML.
 */

import { useEffect, useId, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { font, radius, textSize } from '../../../styles/tokens';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';
import type { AnswerBody, Decision } from './types';
import {
  checkApprovalOf,
  choiceOptions,
  recommendedChoiceId,
  draftText,
  stagedSummary,
} from './types';
import type { AnswerResult } from './useDecisions';
import { EvidenceDigest } from './EvidenceDigest';
import { decisionsClient } from './client';
import { formatAge, withAlpha } from './format';
import { usePersona } from '../../settings/useSettings';
import { useCommandCenter } from '../../../lib/store';
import { toast } from '../../../lib/notifications';
import { isSafeHttpUrl } from '../../../lib/url';

import { Tooltip } from '../../common/Tooltip';
interface Props {
  decision: Decision;
  onAnswer: (id: string, body: AnswerBody) => Promise<AnswerResult>;
  /** Called after the "someone already answered this" state has been shown. */
  onConflictSettled: () => void;
  /** Cancel this decision's goal (#490) — kills the worker and marks it
   *  terminal; the list refreshes after. Absent for non-goal decisions. */
  onCancelGoal?: () => Promise<void>;
  /** Throw away a staged (spoken, uncommitted) verdict — D29's discard.
   *  Absent only where the surface cannot refresh itself afterwards. */
  onDiscardStaged?: (id: string) => Promise<void>;
}

interface PendingAnswer {
  body: AnswerBody;
  confirmLabel: string;
  /** Plain-language restatement shown in the confirm row. */
  effectText: string;
}

/**
 * Confirm-step copy per kind+answer. UI copy describing the server's
 * documented gated effect (routes/decisions.rs:176-334) — never derived from
 * decision content (A1).
 */
export function effectTextFor(
  kind: string,
  answer: 'approve' | 'reject',
  agentName: string,
  actionClass?: string,
): string {
  if (kind === 'approve_review') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will mark this goal complete and start anything waiting on it.`
      : `Confirm reject — ${agentName} will send the work back for another attempt.`;
  }
  if (kind === 'unblock') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will wake this goal up and let it try again.`
      : 'Confirm reject — the goal stays parked.';
  }
  if (kind === 'risk_gate') {
    // Steward git-health cleanup (decisions_effects.rs repo-hygiene arm):
    // informed-consent copy — the effect is a real removal, and every safety
    // check re-runs at the moment it happens.
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
    // Honest pre-S5 contract (routes/decisions.rs session_gate arm): the
    // answer records the ruling; nothing is relayed into the terminal session
    // yet, so it advances only when the gate is answered in its tab (the
    // server's effect message hands back the exact line to paste).
    return answer === 'approve'
      ? 'Confirm allow — records your ruling. The terminal session itself advances once the gate is answered in its tab (the confirmation gives you the exact line to paste).'
      : 'Confirm deny — records your ruling. The terminal session itself advances once the gate is answered in its tab (the confirmation gives you the exact line to paste).';
  }
  // malformed and anything unknown: recorded only, no state change.
  return answer === 'approve'
    ? 'Confirm — this is recorded for the audit trail; nothing else changes.'
    : 'Confirm reject — this is recorded for the audit trail; nothing else changes.';
}

/**
 * Full tool-call arguments carried by a tool_approval payload, pretty-printed
 * for display. The `detail` line above holds only a clipped preview (marked
 * "[truncated — N more chars]" when clipped, tool_execution.rs); informed
 * consent requires the WHOLE thing be inspectable before approving. Untrusted
 * (S2): rendered as plain text only. Null when absent or unserializable.
 * Exported for tests.
 */
export function toolArgumentsText(d: Decision): string | null {
  if (!d.payload) return null;
  // tool_approval carries the tool call in `arguments`; session_gate (S3,
  // #429) carries the supervised session's gated tool input in `input` — the
  // detail line for both holds only a clipped preview.
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

/**
 * Informed-reject warning (#458, GOAL_COMPLETION_AND_VERIFICATION.md §3e).
 * When an approve_review goal's work was already pushed
 * (`dispatch_evidence.push_target` non-null, captured at completion by
 * goal_engine.rs), rejecting does NOT un-ship it — the reviewer must know
 * that before confirming. Pure logic over the canonical evidence record;
 * exported for tests. Null = no warning applies.
 */
export function pushedRejectWarning(
  kind: string,
  pushTarget: string | null | undefined,
): string | null {
  if (kind !== 'approve_review' || !pushTarget) return null;
  return (
    `This work is already on ${pushTarget} — rejecting won't un-ship it. ` +
    'Rework lands as new commits; undoing the pushed commit needs a revert.'
  );
}

export function DecisionItem({
  decision: d,
  onAnswer,
  onConflictSettled,
  onCancelGoal,
  onDiscardStaged,
}: Props) {
  const { colors } = useTheme();
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const discussDecision = useCommandCenter(s => s.discussDecision);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const [pending, setPending] = useState<PendingAnswer | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [noteOpen, setNoteOpen] = useState(false);
  const [note, setNote] = useState('');
  const [inputOpen, setInputOpen] = useState(false);
  const [inputText, setInputText] = useState('');
  const [editOpen, setEditOpen] = useState(false);
  const [editText, setEditText] = useState('');
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const [argsOpen, setArgsOpen] = useState(false);
  const argsId = useId();
  const [cancelErr, setCancelErr] = useState<string | null>(null);
  const [answerErr, setAnswerErr] = useState<string | null>(null);
  const [discardErr, setDiscardErr] = useState<string | null>(null);
  // Where the goal's work was pushed (dispatch_evidence.push_target), fetched
  // from the canonical evidence record for the informed-reject warning (#458).
  const [pushTarget, setPushTarget] = useState<string | null>(null);
  const conflictTimer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => () => clearTimeout(conflictTimer.current), []);

  const isReviewWithGoal = d.kind === 'approve_review' && !!d.goal_id && !!d.project_id;

  // Informed reject (#458 §3e): fetch push state up front so the warning is
  // already in hand when the reviewer reaches the reject confirm step.
  useEffect(() => {
    if (!isReviewWithGoal) return;
    let cancelled = false;
    decisionsClient
      .dispatchEvidence(d.project_id!, d.goal_id!)
      .then(ev => { if (!cancelled) setPushTarget(ev?.push_target ?? null); })
      .catch(() => { /* evidence absent — no warning, nothing to surface */ });
    return () => { cancelled = true; };
  }, [isReviewWithGoal, d.project_id, d.goal_id]);

  // Resolves `false` on anything that is not a clean commit — a conflict or a
  // network failure — so the Button contract never ticks on one.
  const submit = async (p: PendingAnswer) => {
    setSubmitting(true);
    setAnswerErr(null);
    try {
      // A typed note wins; otherwise whatever the answer already carried (a
      // staged verdict brings the words that were said with it) survives.
      const body: AnswerBody = { ...p.body, note: note.trim() ? note.trim() : p.body.note };
      const result = await onAnswer(d.id, body);
      if (!result.ok) {
        setConflict(true);
        setPending(null);
        conflictTimer.current = setTimeout(onConflictSettled, 1600);
        setSubmitting(false);
        return false;
      } else if (result.effect_error) {
        // Partial failure: the answer committed but the gated effect didn't
        // apply. The item leaves the list on refresh, so surface via toast
        // (2026-07 wiring audit — this used to vanish silently).
        toast('Answer recorded — but the follow-through failed', result.effect_error);
      }
      // On success the refreshed list drops this item; nothing else to do.
    } catch (e) {
      // Network / server error: stay on the confirm row and SAY so — the old
      // silent revert was indistinguishable from a dead button.
      setAnswerErr(e instanceof Error ? e.message : 'The answer didn\'t send — try again.');
      setSubmitting(false);
      return false;
    }
    setSubmitting(false);
    return true;
  };

  const badge = badgeFor(d, colors);
  const isUnblock = d.kind === 'unblock';
  const isChoice = d.kind === 'choice';
  const isApprovalLike =
    d.kind === 'approve_review' || d.kind === 'risk_gate' || d.kind === 'malformed' ||
    d.kind === 'enrichment_proposal' || d.kind === 'project_intel_proposal' ||
    d.kind === 'automation_proposal' ||
    d.kind === 'file_to_project' || d.kind === 'tool_approval' ||
    d.kind === 'session_gate' || d.kind === 'council_action';
  // The agent's original draft, when this decision carries one (payload.draft):
  // enables "approve with edits" — revise the text, then accept (answer='edit').
  const draft = draftText(d);
  // risk_gate confirm copy varies by action class (Steward repo hygiene has
  // its own informed-consent wording). Read defensively — payload is untrusted.
  const riskActionClass =
    d.kind === 'risk_gate' && d.payload && typeof d.payload === 'object'
      ? String((d.payload as { action_class?: unknown }).action_class ?? '')
      : undefined;
  const options = choiceOptions(d);
  const recommendedId = recommendedChoiceId(d);
  const recommended = options.find(o => o.id === recommendedId) ?? null;
  // L2 digest lives on the goal card; only goal-bound reviews can have one.
  const hasEvidence = isReviewWithGoal;
  // Full tool-call arguments (tool_approval) / gated tool input
  // (session_gate) — the detail above holds only a clipped preview.
  const toolArgs = toolArgumentsText(d);
  // Tier-2 command approval (verification approval ladder): a choice decision
  // asking the user to rule on a blocked shell command. When present, the
  // command/cwd/reason render as their own block — the whole point of the
  // card is that the user can see EXACTLY what they'd be authorising.
  const checkApproval = checkApprovalOf(d);
  // D29: a verdict said out loud but not committed. Voice cannot authenticate
  // (NIST SP 800-63B-4 §3.2.3.2), so the daemon staged it against a decision
  // that is still open and still unanswered — this row is where it becomes an
  // answer, and the tap is what authenticates it. Expired stagings (30-minute
  // TTL) never arrive here, so anything present is live.
  const staged = d.staged_answer ?? null;
  const stagedVerdict =
    staged && (staged.answer === 'approve' || staged.answer === 'reject')
      ? (staged.answer as 'approve' | 'reject')
      : null;
  const intelItems = d.kind === 'project_intel_proposal' && Array.isArray(d.payload?.items)
    ? d.payload.items.filter((item): item is Record<string, unknown> =>
        typeof item === 'object' && item !== null)
    : [];

  return (
    <div data-testid={`decision-${d.id}`} style={{
      padding: '14px 18px',
      borderBottom: `1px solid ${colors.border}`,
      fontFamily: font.body,
    }}>
      {/* Row 1: badge + plain-language headline + age */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{
          fontFamily: font.mono, fontSize: 10, letterSpacing: '0.06em',
          textTransform: 'uppercase', borderRadius: radius.xs, padding: '2px 6px',
          flexShrink: 0, color: badge.color, background: badge.bg,
        }}>
          {badge.label}
        </span>
        <span style={{
          fontSize: textSize.small, fontWeight: 600, color: colors.text, flex: 1,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {d.headline}
        </span>
        <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
          {formatAge(d.created_at)}
        </span>
      </div>

      {/* Goal binding, where known (joined title — plain data). Click opens the
          shared goal-detail modal (#503) when the decision is goal-bound. */}
      {d.goal_title && (
        d.goal_id && d.project_id ? (
          <Tooltip content="View goal detail">
            <Button
              colors={colors}
              variant="bare"
              type="button"
              className="hover:underline"
              onClick={() => openGoalDetail(d.project_id!, d.goal_id!)}
              style={{
                '--pa-btn-fg': colors.cyan,
                '--pa-btn-fg-hover': colors.cyan,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': '0',
                '--pa-btn-weight': 400,
                display: 'flex', justifyContent: 'flex-start', marginTop: 4,
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Goal: {d.goal_title}
            </Button>
          </Tooltip>
        ) : (
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 4 }}>
            Goal: {d.goal_title}
          </div>
        )
      )}

      {/* Detail: technical why/attribution, verbatim (S2) */}
      {d.detail && (
        <div style={{
          fontSize: textSize.caption, color: colors.textMuted, marginTop: 4,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
          maxHeight: 96, overflow: 'auto',
        }}>
          {d.detail}
        </div>
      )}

      {/* Command approval (verification approval ladder, Tier 2): the exact
          blocked command, its cwd, and why it was stopped. Rendered as plain
          text only (S2, never dangerouslySetInnerHTML) — no truncation, CSS
          wraps/scrolls instead so the full command stays inspectable. */}
      {checkApproval && (
        <div style={{ marginTop: 8 }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: 4 }}>
            Command
          </div>
          <pre style={{
            margin: 0, borderRadius: radius.sm, background: colors.codeBg,
            padding: '10px 12px', fontFamily: font.mono, fontSize: textSize.caption,
            lineHeight: 1.6, color: colors.text, maxHeight: 240,
            overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            userSelect: 'text',
          }}>
            {checkApproval.command}
          </pre>
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 6 }}>
            <span style={{ color: colors.textDim }}>in </span>
            <span style={{ fontFamily: font.mono, overflowWrap: 'anywhere' }}>{checkApproval.cwd}</span>
          </div>
          {checkApproval.reason && (
            <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: 4 }}>
              {checkApproval.reason}
            </div>
          )}
        </div>
      )}

      {intelItems.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, marginTop: 8 }}>
          {intelItems.map((item, index) => {
            const kind = typeof item.kind === 'string' ? item.kind : 'item';
            const name = typeof item.name === 'string' ? item.name : 'Unnamed';
            const note = typeof item.note === 'string' ? item.note : null;
            const source = typeof item.source_url === 'string' ? item.source_url : null;
            return (
              <div key={`${kind}-${name}-${index}`} style={{ borderLeft: `2px solid ${colors.purpleBright}`, paddingLeft: 9 }}>
                <div style={{ color: colors.text, fontSize: textSize.caption, fontWeight: 600 }}>{kind}: {name}</div>
                {note && <div style={{ color: colors.textMuted, fontSize: textSize.micro }}>{note}</div>}
                {source && isSafeHttpUrl(source) ? (
                  <a href={source} target="_blank" rel="noreferrer" style={{ color: colors.cyan, fontSize: textSize.micro }}>
                    Source
                  </a>
                ) : source ? (
                  <span style={{ color: colors.textMuted, fontSize: textSize.micro }}>Source</span>
                ) : null}
              </div>
            );
          })}
        </div>
      )}

      {/* Full tool-call arguments (tool_approval) — the detail above holds a
          clipped preview; what gets approved must be inspectable in full.
          S2: plain text in a <pre>; nothing is interpreted or linked. */}
      {toolArgs && (
        <div>
          {/* Disclosure toggle for the <pre> right below: nothing to await, so
              it keeps the element and takes the shared `.pa-btn` interaction
              rules rather than the Button primitive's pending/success
              machinery. */}
          <button
            type="button"
            className="pa-btn"
            aria-expanded={argsOpen}
            aria-controls={argsId}
            onClick={() => setArgsOpen(o => !o)}
            style={{
              '--pa-btn-bg': 'transparent',
              '--pa-btn-fg': argsOpen ? colors.cyan : colors.textDim,
              '--pa-btn-border': 'transparent',
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-fg-hover': argsOpen ? colors.cyan : colors.textMuted,
              '--pa-btn-bg-active': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-radius': '0',
              '--pa-btn-weight': 400,
              marginTop: 6,
              fontSize: textSize.micro, fontFamily: font.body,
            } as CSSProperties}
          >
            {argsOpen ? 'Hide full arguments ▾' : 'Show full arguments ▸'}
          </button>
          {argsOpen && (
            <pre id={argsId} style={{
              margin: '6px 0 0', borderRadius: radius.sm,
              background: colors.codeBg, padding: '10px 12px',
              fontFamily: font.mono, fontSize: textSize.micro, lineHeight: 1.6,
              color: colors.textMuted, maxHeight: 240, overflow: 'auto',
              whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
            }}>
              {toolArgs}
            </pre>
          )}
        </div>
      )}

      {/* Recommendation chip — only when the daemon marked a default option */}
      {recommended && (
        <div style={{ marginTop: 8 }}>
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 6,
            borderRadius: radius.pill, background: colors.cyanSoft,
            color: colors.cyan, fontSize: textSize.micro, fontWeight: 500, padding: '3px 10px',
          }}>
            {agentName} recommends · {recommended.label}
          </span>
        </div>
      )}

      {/* Staged verdict (D29): what was said out loud, and the tap that makes
          it an answer. Deliberately loud — a proposal sitting unnoticed is the
          failure mode — and the discard beside it is the same size, because
          "I didn't mean that" must be as easy as "yes I did". */}
      {staged && stagedVerdict && !conflict && !pending && (
        <div
          data-testid={`staged-${d.id}`}
          style={{
            marginTop: 10, borderRadius: radius.md,
            border: `1px solid ${colors.purpleBright}`,
            background: colors.purpleSoft, padding: '10px 12px',
            display: 'flex', flexDirection: 'column', gap: 8,
          }}
        >
          <div style={{
            display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap',
            fontSize: textSize.caption, fontWeight: 600, color: colors.text,
          }}>
            <span aria-hidden="true">🎙</span>
            <span>{stagedSummary(staged)}</span>
            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, fontWeight: 400 }}>
              {/* formatAge says "now" under a minute — "now ago" would be nonsense. */}
              · {formatAge(staged.staged_at) === 'now' ? 'just now' : `${formatAge(staged.staged_at)} ago`}
            </span>
          </div>
          {/* Honest about what has and has not happened yet. */}
          <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>
            Heard, not committed — nothing has happened yet. Committing does
            this: {effectTextFor(d.kind, stagedVerdict, agentName, riskActionClass)}
          </div>
          {staged.note && (
            <div style={{
              fontSize: textSize.micro, color: colors.textMuted,
              whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
            }}>
              Said with it: {staged.note}
            </div>
          )}
          {stagedVerdict === 'reject' && pushedRejectWarning(d.kind, pushTarget) && (
            <span role="alert" style={{
              fontSize: textSize.caption, fontWeight: 600, color: colors.warning,
              display: 'flex', gap: 6, alignItems: 'baseline',
            }}>
              <span aria-hidden="true">⚠</span>
              {pushedRejectWarning(d.kind, pushTarget)}
            </span>
          )}
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            {/* One tap, and it travels the ordinary answer route: same actor,
                same tier gate, attributed to this device — never to "voice". */}
            <Btn
              variant="primary"
              disabled={submitting}
              onClick={() =>
                submit({
                  body: {
                    answer: stagedVerdict,
                    note: staged.note ?? undefined,
                  },
                  confirmLabel: '',
                  effectText: '',
                })
              }
            >
              {submitting
                ? 'Sending…'
                : stagedVerdict === 'approve' ? 'Commit approve' : 'Commit reject'}
            </Btn>
            {onDiscardStaged && (
              <Btn
                danger
                disabled={submitting}
                onClick={async () => {
                  setDiscardErr(null);
                  try {
                    await onDiscardStaged(d.id);
                    return true;
                  } catch (e) {
                    setDiscardErr(
                      e instanceof Error ? e.message : "The discard didn't send — try again.",
                    );
                    return false;
                  }
                }}
              >
                Discard
              </Btn>
            )}
          </div>
          {answerErr && (
            <span role="alert" style={{ fontSize: textSize.micro, color: colors.danger }}>
              Couldn't send: {answerErr}
            </span>
          )}
          {discardErr && (
            <span role="alert" style={{ fontSize: textSize.micro, color: colors.danger }}>
              Couldn't discard: {discardErr}
            </span>
          )}
        </div>
      )}

      {/* Conflict: already answered elsewhere */}
      {conflict ? (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8, marginTop: 10,
          borderRadius: radius.md, border: `1px solid ${colors.warning}`,
          background: withAlpha(colors.warning, 0.08), padding: '8px 12px',
          fontSize: textSize.caption, color: colors.text,
        }}>
          Someone already answered this — refreshing…
        </div>
      ) : pending ? (
        /* Inline confirm step — one item at a time, never batch */
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8, marginTop: 10, flexWrap: 'wrap',
          borderRadius: radius.md, border: `1px solid ${colors.borderHi}`,
          background: colors.cyanSoft, padding: '8px 12px',
        }}>
          <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1, minWidth: 180 }}>
            {pending.effectText}
          </span>
          {/* Informed reject (#458): the work is already pushed — say so loudly
              BEFORE the reject is confirmed. Reject stays enabled (advisory). */}
          {pending.body.answer === 'reject' && pushedRejectWarning(d.kind, pushTarget) && (
            <span role="alert" style={{
              flexBasis: '100%', order: -1, fontSize: textSize.caption, fontWeight: 600,
              color: colors.warning, display: 'flex', gap: 6, alignItems: 'baseline',
            }}>
              <span aria-hidden="true">⚠</span>
              {pushedRejectWarning(d.kind, pushTarget)}
            </span>
          )}
          {pending.body.answer === 'reject' && d.kind === 'enrichment_proposal' && (
            <textarea
              value={note}
              onChange={e => setNote(e.target.value)}
              placeholder="How can the agent find this person online? Company, LinkedIn, city…"
              rows={3}
              style={{
                flexBasis: '100%', width: '100%', boxSizing: 'border-box', resize: 'vertical',
                borderRadius: radius.md, border: `1px solid ${colors.border}`,
                background: colors.inputBg, color: colors.text,
                fontFamily: font.body, fontSize: textSize.caption, padding: '8px 10px', outline: 'none',
              }}
            />
          )}
          <Btn variant="primary" disabled={submitting} onClick={() => submit(pending)}>
            {submitting ? 'Sending…' : pending.confirmLabel}
          </Btn>
          <Btn disabled={submitting} onClick={() => { setPending(null); setAnswerErr(null); }}>Cancel</Btn>
          {answerErr && (
            <span role="alert" style={{ fontSize: textSize.micro, color: colors.danger, flexBasis: '100%' }}>
              Couldn't send: {answerErr}
            </span>
          )}
        </div>
      ) : inputOpen ? (
        /* Freeform answer (unblock) — travels as answer='input' */
        <div style={{ marginTop: 10 }}>
          <textarea
            value={inputText}
            onChange={e => setInputText(e.target.value)}
            placeholder="Type your answer — it goes to the worker exactly as written"
            rows={3}
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'vertical',
              borderRadius: radius.md, border: `1px solid ${colors.border}`,
              background: colors.inputBg, color: colors.text,
              fontFamily: font.body, fontSize: textSize.caption, padding: '8px 10px', outline: 'none',
            }}
          />
          <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
            <Btn
              variant="primary"
              disabled={!inputText.trim()}
              onClick={() => {
                setInputOpen(false);
                setPending({
                  body: { answer: 'input', input_text: inputText.trim() },
                  confirmLabel: 'Confirm answer',
                  effectText: 'Confirm answer — your reply goes to the worker exactly as written.',
                });
              }}
            >
              Send answer
            </Btn>
            <Btn onClick={() => setInputOpen(false)}>Cancel</Btn>
          </div>
        </div>
      ) : editOpen ? (
        /* Approve-with-edits — revise the agent's draft, then accept. Travels as
           answer='edit': the daemon keeps the revision AND learns the
           draft→revision delta (edit-as-training, decision_inbox/learn.rs). */
        <div style={{ marginTop: 10 }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: 6 }}>
            Revise the draft, then accept — your version becomes the answer and {agentName} learns the change.
          </div>
          <textarea
            value={editText}
            onChange={e => setEditText(e.target.value)}
            rows={4}
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'vertical',
              borderRadius: radius.md, border: `1px solid ${colors.border}`,
              background: colors.inputBg, color: colors.text,
              fontFamily: font.body, fontSize: textSize.caption, padding: '8px 10px', outline: 'none',
            }}
          />
          <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
            <Btn
              variant="primary"
              disabled={!editText.trim()}
              onClick={() => {
                setEditOpen(false);
                setPending({
                  body: { answer: 'edit', input_text: editText.trim() },
                  confirmLabel: 'Confirm accept',
                  effectText: `Confirm accept — ${agentName} will use your edited version and learn from the change.`,
                });
              }}
            >
              Accept edited
            </Btn>
            <Btn onClick={() => setEditOpen(false)}>Cancel</Btn>
          </div>
        </div>
      ) : (
        /* Action row per kind (A4): binary approvals get Approve/Reject/Add note
           only; option chips appear only on choice-kind decisions. */
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
          {(isApprovalLike || isUnblock) && (
            <>
              <Btn
                variant="primary"
                onClick={() => setPending({
                  body: { answer: 'approve' },
                  confirmLabel: 'Confirm approve',
                  effectText: effectTextFor(d.kind, 'approve', agentName, riskActionClass),
                })}
              >
                Approve
              </Btn>
              <Btn
                danger
                onClick={() => setPending({
                  body: { answer: 'reject' },
                  confirmLabel: 'Confirm reject',
                  effectText: effectTextFor(d.kind, 'reject', agentName, riskActionClass),
                })}
              >
                Reject
              </Btn>
            </>
          )}

          {/* Approve-with-edits — only when the decision carries an editable
              draft (payload.draft). Revise, then accept as answer='edit'. */}
          {draft && (
            <Btn onClick={() => { setEditText(draft); setEditOpen(true); }}>
              Edit &amp; accept
            </Btn>
          )}

          {isChoice && options.map(opt => (
            <Btn
              key={opt.id}
              variant={opt.id === recommendedId ? 'primary' : 'ghost'}
              onClick={() => setPending({
                body: { answer: 'choice', choice_id: opt.id },
                confirmLabel: 'Confirm choice',
                effectText: `Confirm “${opt.label}” — ${agentName} will go with this option.`,
              })}
            >
              {opt.label}
            </Btn>
          ))}

          {isUnblock && (
            <Btn onClick={() => setInputOpen(true)}>Send answer</Btn>
          )}

          <Btn onClick={() => setNoteOpen(o => !o)}>Add note</Btn>

          {/* Deep-link into a context-preloaded chat (#303) — available on every
              kind; discuss-only in v1, the decision stays answerable from here. */}
          <Btn onClick={() => discussDecision(d.id, d.headline)}>
            Discuss with {agentName}
          </Btn>

          {/* Cancel the underlying goal (#490). User-initiated and immediate:
              kills the worker and supersedes this decision. */}
          {onCancelGoal && (
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={async () => {
                if (submitting) return false;
                setSubmitting(true);
                setCancelErr(null);
                try {
                  await onCancelGoal();
                  return true;
                } catch (e) {
                  // Resolves `false` so a cancel that failed cannot tick; the
                  // message below is what actually says what happened.
                  setCancelErr(e instanceof Error ? e.message : 'Cancel failed');
                  return false;
                } finally {
                  setSubmitting(false);
                }
              }}
              disabled={submitting}
              style={{
                '--pa-btn-fg': colors.warning,
                '--pa-btn-fg-hover': colors.warning,
                '--pa-btn-bg-hover': withAlpha(colors.warning, 0.12),
                '--pa-btn-bg-active': withAlpha(colors.warning, 0.18),
                '--pa-btn-pad': '4px',
                '--pa-btn-radius': `${radius.xs}px`,
                '--pa-btn-weight': 400,
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Cancel goal
            </Button>
          )}

          {hasEvidence && (
            /* Disclosure toggle for the digest below — same treatment as the
               full-arguments toggle: the element stays, the interaction rules
               are the shared ones. */
            <button
              type="button"
              className="pa-btn"
              aria-expanded={evidenceOpen}
              onClick={() => setEvidenceOpen(o => !o)}
              style={{
                '--pa-btn-bg': 'transparent',
                '--pa-btn-fg': evidenceOpen ? colors.cyan : colors.textDim,
                '--pa-btn-border': 'transparent',
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-fg-hover': evidenceOpen ? colors.cyan : colors.textMuted,
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '4px',
                '--pa-btn-radius': `${radius.xs}px`,
                '--pa-btn-weight': 400,
                marginLeft: 'auto',
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Evidence {evidenceOpen ? '▾' : '▸'}
            </button>
          )}
        </div>
      )}

      {/* Cancel failure surfaced inline — never silently swallowed (#503) */}
      {cancelErr && (
        <div style={{
          marginTop: 8, fontSize: textSize.caption, color: colors.danger,
          borderRadius: radius.md, border: `1px solid ${colors.danger}`,
          background: withAlpha(colors.danger, 0.08), padding: '6px 10px',
        }}>
          {cancelErr}
        </div>
      )}

      {/* Note rides along with whatever answer is confirmed */}
      {noteOpen && !conflict && !(pending?.body.answer === 'reject' && d.kind === 'enrichment_proposal') && (
        <textarea
          value={note}
          onChange={e => setNote(e.target.value)}
          placeholder="Add a note — it travels with your answer"
          rows={2}
          style={{
            width: '100%', boxSizing: 'border-box', resize: 'vertical', marginTop: 8,
            borderRadius: radius.md, border: `1px solid ${colors.border}`,
            background: colors.inputBg, color: colors.text,
            fontFamily: font.body, fontSize: textSize.caption, padding: '8px 10px', outline: 'none',
          }}
        />
      )}

      {/* Layered evidence (A3) — lazy-fetched L2 digest from the goal card */}
      {evidenceOpen && hasEvidence && (
        <EvidenceDigest projectId={d.project_id!} goalId={d.goal_id!} />
      )}
    </div>
  );
}

function badgeFor(d: Decision, colors: ReturnType<typeof useTheme>['colors']) {
  switch (d.kind) {
    case 'unblock': return { label: 'unblock', color: colors.warning, bg: withAlpha(colors.warning, 0.14) };
    case 'choice':
      return checkApprovalOf(d)
        ? { label: 'command approval', color: colors.danger, bg: withAlpha(colors.danger, 0.14) }
        : { label: 'choice', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'risk_gate': return { label: 'permission', color: colors.danger, bg: withAlpha(colors.danger, 0.14) };
    case 'session_gate': return { label: 'terminal gate', color: colors.danger, bg: withAlpha(colors.danger, 0.14) };
    case 'enrichment_proposal': return { label: 'enrichment', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'project_intel_proposal': return { label: 'project intel', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'file_to_project': return { label: 'file note', color: colors.cyan, bg: colors.cyanSoft };
    case 'council_action': return { label: 'council', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'malformed': return { label: 'review', color: colors.warning, bg: withAlpha(colors.warning, 0.14) };
    default: return { label: 'approval', color: colors.cyan, bg: colors.cyanSoft };
  }
}

/**
 * The decision row's button shape, now a thin adapter over the app's one Button
 * primitive rather than a third button contract of its own.
 *
 * What it used to be: a local `hover` state driven by a pair of mouse handlers,
 * which is the only way an inline `style` object can express a hover at all —
 * and which still left pressing one indistinguishable from not pressing it.
 * That machinery is gone. The look is unchanged, but it now arrives as
 * `--pa-btn-*` custom properties so the shared `:hover`/`:active`/disabled
 * rules can reach it, and an `onClick` that returns a promise gets the pending
 * and success contract for free.
 *
 * R7 (D10): the ghost/danger hover and press states used to repeat the resting
 * `colors.surface` fill verbatim — a color swap on the border and label, but
 * no fill at all, on a button that sits on that exact same surface (this row
 * lives inside `DetailModal`, whose panel background IS `colors.surface`), so
 * hovering or pressing one produced no visible lift whatsoever. `fillHover` /
 * `fillActive` are the tokens built for precisely this (dashboard cards'
 * `InFlightCard`/`GrowthResultsCard` use the identical
 * `pressed ? fillActive : hover ? fillHover : colors.surface` ladder over the
 * same surface). Primary is unaffected — its lift is `.pa-btn--primary`'s
 * global `filter: brightness()`, already applied through CSS regardless of
 * this token.
 */
function Btn({ variant = 'ghost', danger = false, disabled = false, onClick, children }: {
  variant?: 'primary' | 'ghost';
  danger?: boolean;
  disabled?: boolean;
  /** Returning a promise opts this button into pending + success; resolving
   *  `false` means it failed, so a failure never ticks. */
  onClick: () => unknown;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  const primary = variant === 'primary';
  return (
    <Button
      colors={colors}
      variant={primary ? 'primary' : 'ghost'}
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={{
        '--pa-btn-bg': primary ? colors.ribbonGradient : colors.surface,
        '--pa-btn-fg': primary
          ? colors.textOnAccent
          : danger ? colors.danger : colors.textMuted,
        '--pa-btn-border': primary ? 'transparent' : colors.border,
        '--pa-btn-bg-hover': primary ? colors.ribbonGradient : colors.fillHover,
        // Danger keeps its colour through hover, exactly as it did before.
        '--pa-btn-fg-hover': primary
          ? colors.textOnAccent
          : danger ? colors.danger : colors.text,
        '--pa-btn-border-hover': primary ? 'transparent' : colors.borderHi,
        '--pa-btn-bg-active': primary ? colors.ribbonGradient : colors.fillActive,
        '--pa-btn-pad': '5px 14px',
        '--pa-btn-radius': `${radius.md}px`,
        '--pa-btn-weight': primary ? 600 : 500,
        fontFamily: font.body, fontSize: textSize.caption,
      } as CSSProperties}
    >
      {children}
    </Button>
  );
}
