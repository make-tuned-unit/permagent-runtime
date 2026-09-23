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
import { font, radius, space, textSize } from '../../../styles/tokens';
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
import { effectTextFor, toolArgumentsText } from './decisionEffects';
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
export { effectTextFor, toolArgumentsText };

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
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const badge = badgeFor(d, colors);

  return (
    <div data-testid={`decision-${d.id}`} style={{
      padding: '14px 18px',
      borderBottom: `1px solid ${colors.border}`,
      fontFamily: font.body,
    }}>
      {/* Row 1: badge + plain-language headline + age */}
      <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
        <span style={{
          fontFamily: font.mono, fontSize: 10, letterSpacing: '0.06em',
          textTransform: 'uppercase', borderRadius: radius.xs, padding: `${space.xxs}px ${space.sm}px`,
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
                display: 'flex', justifyContent: 'flex-start', marginTop: space.xs,
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Goal: {d.goal_title}
            </Button>
          </Tooltip>
        ) : (
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: space.xs }}>
            Goal: {d.goal_title}
          </div>
        )
      )}

      {/* Detail: technical why/attribution, verbatim (S2) */}
      {d.detail && (
        <div style={{
          fontSize: textSize.caption, color: colors.textMuted, marginTop: space.xs,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
          maxHeight: 96, overflow: 'auto',
        }}>
          {d.detail}
        </div>
      )}

      <DecisionActions
        decision={d}
        agentName={agentName}
        note=""
        onAnswered={body => onAnswer(d.id, body)}
        onConflict={onConflictSettled}
        onCancelGoal={onCancelGoal}
        onDiscardStaged={onDiscardStaged}
      />
    </div>
  );
}

interface DecisionActionsProps {
  decision: Decision;
  agentName: string;
  note: string;
  onNoteChange?: (note: string) => void;
  embeddedReview?: boolean;
  onAnswered: (body: AnswerBody) => Promise<AnswerResult>;
  onConflict: () => void;
  onCancelGoal?: () => Promise<void>;
  onDiscardStaged?: (id: string) => Promise<void>;
}

export function DecisionActions({
  decision: d,
  agentName,
  note,
  onNoteChange,
  embeddedReview = false,
  onAnswered,
  onConflict,
  onCancelGoal,
  onDiscardStaged,
}: DecisionActionsProps) {
  const { colors } = useTheme();
  const discussDecision = useCommandCenter(s => s.discussDecision);
  const [pending, setPending] = useState<PendingAnswer | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [noteOpen, setNoteOpen] = useState(false);
  const [localNote, setLocalNote] = useState(note);
  const noteText = onNoteChange ? note : localNote;
  const setNoteText = onNoteChange ?? setLocalNote;
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
  const [pushTarget, setPushTarget] = useState<string | null>(null);
  const conflictTimer = useRef<ReturnType<typeof setTimeout>>();
  const submitInFlight = useRef(false);

  useEffect(() => {
    setLocalNote(note);
    setNoteOpen(false);
    setPending(null);
    setInputOpen(false);
    setEditOpen(false);
    setAnswerErr(null);
    setConflict(false);
  }, [d.id]);

  useEffect(() => () => clearTimeout(conflictTimer.current), []);

  const isReviewWithGoal = d.kind === 'approve_review' && !!d.goal_id && !!d.project_id;

  useEffect(() => {
    if (!isReviewWithGoal) return;
    let cancelled = false;
    decisionsClient
      .dispatchEvidence(d.project_id!, d.goal_id!)
      .then(ev => { if (!cancelled) setPushTarget(ev?.push_target ?? null); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [isReviewWithGoal, d.project_id, d.goal_id]);

  const submit = async (p: PendingAnswer) => {
    if (submitInFlight.current) return false;
    submitInFlight.current = true;
    setSubmitting(true);
    setAnswerErr(null);
    try {
      const body: AnswerBody = { ...p.body, note: noteText.trim() ? noteText.trim() : p.body.note };
      const result = await onAnswered(body);
      if (!result.ok) {
        setConflict(!embeddedReview);
        setPending(null);
        conflictTimer.current = setTimeout(onConflict, 1600);
        return false;
      }
      if (result.effect_error) {
        toast('Answer recorded — but the follow-through failed', result.effect_error);
      }
    } catch (e) {
      setAnswerErr(e instanceof Error ? e.message : 'The answer didn\'t send — try again.');
      return false;
    } finally {
      submitInFlight.current = false;
      setSubmitting(false);
    }
    return true;
  };

  const isUnblock = d.kind === 'unblock';
  const isChoice = d.kind === 'choice';
  const isApprovalLike =
    d.kind === 'approve_review' || d.kind === 'risk_gate' || d.kind === 'malformed' ||
    d.kind === 'enrichment_proposal' || d.kind === 'project_intel_proposal' ||
    d.kind === 'automation_proposal' || d.kind === 'file_to_project' ||
    d.kind === 'tool_approval' || d.kind === 'session_gate' || d.kind === 'council_action';
  const draft = draftText(d);
  const riskActionClass =
    d.kind === 'risk_gate' && d.payload && typeof d.payload === 'object'
      ? String((d.payload as { action_class?: unknown }).action_class ?? '')
      : undefined;
  const options = choiceOptions(d);
  const recommendedId = recommendedChoiceId(d);
  const recommended = options.find(o => o.id === recommendedId) ?? null;
  const hasEvidence = isReviewWithGoal;
  const toolArgs = toolArgumentsText(d);
  const checkApproval = checkApprovalOf(d);
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
    <>
      {checkApproval && (
        <div style={{ marginTop: space.md }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.xs }}>
            Command
          </div>
          <pre style={{
            margin: 0, borderRadius: radius.sm, background: colors.codeBg,
            padding: `${space.lg}px ${space.xl}px`, fontFamily: font.mono, fontSize: textSize.caption,
            lineHeight: 1.6, color: colors.text, maxHeight: 240,
            overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
          }}>
            {checkApproval.command}
          </pre>
          <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: space.sm }}>
            <span style={{ color: colors.textDim }}>in </span>
            <span style={{ fontFamily: font.mono, overflowWrap: 'anywhere' }}>{checkApproval.cwd}</span>
          </div>
          {checkApproval.reason && (
            <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: space.xs }}>
              {checkApproval.reason}
            </div>
          )}
        </div>
      )}

      {intelItems.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm, marginTop: space.md }}>
          {intelItems.map((item, index) => {
            const kind = typeof item.kind === 'string' ? item.kind : 'item';
            const name = typeof item.name === 'string' ? item.name : 'Unnamed';
            const itemNote = typeof item.note === 'string' ? item.note : null;
            const source = typeof item.source_url === 'string' ? item.source_url : null;
            return (
              <div key={`${kind}-${name}-${index}`} style={{ borderLeft: `2px solid ${colors.purpleBright}`, paddingLeft: space.md }}>
                <div style={{ color: colors.text, fontSize: textSize.caption, fontWeight: 600 }}>{kind}: {name}</div>
                {itemNote && <div style={{ color: colors.textMuted, fontSize: textSize.micro }}>{itemNote}</div>}
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

      {toolArgs && (
        <div>
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
              marginTop: space.sm,
              fontSize: textSize.micro, fontFamily: font.body,
            } as CSSProperties}
          >
            {argsOpen ? 'Hide full arguments ▾' : 'Show full arguments ▸'}
          </button>
          {argsOpen && (
            <pre id={argsId} style={{
              margin: '6px 0 0', borderRadius: radius.sm,
              background: colors.codeBg, padding: `${space.lg}px ${space.xl}px`,
              fontFamily: font.mono, fontSize: textSize.micro, lineHeight: 1.6,
              color: colors.textMuted, maxHeight: 240, overflow: 'auto',
              whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
            }}>
              {toolArgs}
            </pre>
          )}
        </div>
      )}

      {recommended && (
        <div style={{ marginTop: space.md }}>
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: space.sm,
            borderRadius: radius.pill, background: colors.cyanSoft,
            color: colors.cyan, fontSize: textSize.micro, fontWeight: 500, padding: `${space.xxs}px ${space.lg}px`,
          }}>
            {agentName} recommends · {recommended.label}
          </span>
        </div>
      )}

      {staged && stagedVerdict && !conflict && !pending && (
        <div
          data-testid={`staged-${d.id}`}
          style={{
            marginTop: space.lg, borderRadius: radius.md,
            border: `1px solid ${colors.purpleBright}`,
            background: colors.purpleSoft, padding: `${space.lg}px ${space.xl}px`,
            display: 'flex', flexDirection: 'column', gap: space.md,
          }}
        >
          <div style={{
            display: 'flex', alignItems: 'baseline', gap: space.md, flexWrap: 'wrap',
            fontSize: textSize.caption, fontWeight: 600, color: colors.text,
          }}>
            <span aria-hidden="true">🎙</span>
            <span>{stagedSummary(staged)}</span>
            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, fontWeight: 400 }}>
              · {formatAge(staged.staged_at) === 'now' ? 'just now' : `${formatAge(staged.staged_at)} ago`}
            </span>
          </div>
          <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>
            Heard, not committed — nothing has happened yet. Committing does
            this: {effectTextFor(d.kind, stagedVerdict, agentName, riskActionClass, d)}
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
              display: 'flex', gap: space.sm, alignItems: 'baseline',
            }}>
              <span aria-hidden="true">⚠</span>
              {pushedRejectWarning(d.kind, pushTarget)}
            </span>
          )}
          <div style={{ display: 'flex', gap: space.md, flexWrap: 'wrap' }}>
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

      {conflict ? (
        <div style={{
          display: 'flex', alignItems: 'center', gap: space.md, marginTop: space.lg,
          borderRadius: radius.md, border: `1px solid ${colors.warning}`,
          background: withAlpha(colors.warning, 0.08), padding: `${space.md}px ${space.xl}px`,
          fontSize: textSize.caption, color: colors.text,
        }}>
          {embeddedReview ? 'Answer not accepted — refreshing authoritative state…' : 'Someone already answered this — refreshing…'}
        </div>
      ) : pending ? (
        <div style={{
          display: 'flex', alignItems: 'center', gap: space.md, marginTop: space.lg, flexWrap: 'wrap',
          borderRadius: radius.md, border: `1px solid ${colors.borderHi}`,
          background: colors.cyanSoft, padding: `${space.md}px ${space.xl}px`,
        }}>
          <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1, minWidth: 180 }}>
            {pending.effectText}
          </span>
          {pending.body.answer === 'reject' && pushedRejectWarning(d.kind, pushTarget) && (
            <span role="alert" style={{
              flexBasis: '100%', order: -1, fontSize: textSize.caption, fontWeight: 600,
              color: colors.warning, display: 'flex', gap: space.sm, alignItems: 'baseline',
            }}>
              <span aria-hidden="true">⚠</span>
              {pushedRejectWarning(d.kind, pushTarget)}
            </span>
          )}
          {pending.body.answer === 'reject' && d.kind === 'enrichment_proposal' && (
            <textarea
              value={noteText}
              onChange={e => setNoteText(e.target.value)}
              placeholder="How can the agent find this person online? Company, LinkedIn, city…"
              rows={3}
              style={{
                flexBasis: '100%', width: '100%', boxSizing: 'border-box', resize: 'vertical',
                borderRadius: radius.md, border: `1px solid ${colors.border}`,
                background: colors.inputBg, color: colors.text,
                fontFamily: font.body, fontSize: textSize.caption, padding: `${space.md}px ${space.lg}px`, outline: 'none',
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
        <div style={{ marginTop: space.lg }}>
          <textarea
            value={inputText}
            onChange={e => setInputText(e.target.value)}
            placeholder="Type your answer — it goes to the worker exactly as written"
            rows={3}
            style={{
              width: '100%', boxSizing: 'border-box', resize: 'vertical',
              borderRadius: radius.md, border: `1px solid ${colors.border}`,
              background: colors.inputBg, color: colors.text,
              fontFamily: font.body, fontSize: textSize.caption, padding: `${space.md}px ${space.lg}px`, outline: 'none',
            }}
          />
          <div style={{ display: 'flex', gap: space.md, marginTop: space.md }}>
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
        <div style={{ marginTop: space.lg }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: space.sm }}>
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
              fontFamily: font.body, fontSize: textSize.caption, padding: `${space.md}px ${space.lg}px`, outline: 'none',
            }}
          />
          <div style={{ display: 'flex', gap: space.md, marginTop: space.md }}>
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
        <div style={{ display: 'flex', alignItems: 'center', gap: space.md, marginTop: space.lg, flexWrap: 'wrap' }}>
          {(isApprovalLike || isUnblock) && (
            <>
              <Btn
                variant="primary"
                onClick={() => setPending({
                  body: { answer: 'approve' },
                  confirmLabel: 'Confirm approve',
                  effectText: effectTextFor(d.kind, 'approve', agentName, riskActionClass, d),
                })}
              >
                Approve
              </Btn>
              <Btn
                danger
                onClick={() => setPending({
                  body: { answer: 'reject' },
                  confirmLabel: 'Confirm reject',
                  effectText: effectTextFor(d.kind, 'reject', agentName, riskActionClass, d),
                })}
              >
                Reject
              </Btn>
            </>
          )}

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

          {!embeddedReview && <Btn onClick={() => setNoteOpen(o => !o)}>Add note</Btn>}

          {!embeddedReview && <Btn onClick={() => discussDecision(d.id, d.headline)}>
            Discuss with {agentName}
          </Btn>}

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

      {cancelErr && (
        <div style={{
          marginTop: space.md, fontSize: textSize.caption, color: colors.danger,
          borderRadius: radius.md, border: `1px solid ${colors.danger}`,
          background: withAlpha(colors.danger, 0.08), padding: `${space.sm}px ${space.lg}px`,
        }}>
          {cancelErr}
        </div>
      )}

      {noteOpen && !conflict && !(pending?.body.answer === 'reject' && d.kind === 'enrichment_proposal') && (
        <textarea
          value={noteText}
          onChange={e => setNoteText(e.target.value)}
          placeholder="Add a note — it travels with your answer"
          rows={2}
          style={{
            width: '100%', boxSizing: 'border-box', resize: 'vertical', marginTop: space.md,
            borderRadius: radius.md, border: `1px solid ${colors.border}`,
            background: colors.inputBg, color: colors.text,
            fontFamily: font.body, fontSize: textSize.caption, padding: `${space.md}px ${space.lg}px`, outline: 'none',
          }}
        />
      )}

      {evidenceOpen && hasEvidence && (
        <EvidenceDigest projectId={d.project_id!} goalId={d.goal_id!} />
      )}
    </>
  );
}

export function badgeFor(d: Decision, colors: ReturnType<typeof useTheme>['colors']) {
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
