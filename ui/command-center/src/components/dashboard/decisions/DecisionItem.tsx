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
 *    Enriched provenance (manual entries protected); reject records only.
 *  - malformed: acknowledgement only — recorded, no state change.
 *
 * S2: every string from the daemon renders as a React text node. No markdown,
 * no auto-links, no dangerouslySetInnerHTML.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { font, radius, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import type { AnswerBody, Decision } from './types';
import { choiceOptions, recommendedChoiceId, draftText } from './types';
import type { AnswerResult } from './useDecisions';
import { EvidenceDigest } from './EvidenceDigest';
import { formatAge } from './format';
import { usePersona } from '../../settings/useSettings';
import { useCommandCenter } from '../../../lib/store';
import { toast } from '../../../lib/notifications';

interface Props {
  decision: Decision;
  onAnswer: (id: string, body: AnswerBody) => Promise<AnswerResult>;
  /** Called after the "someone already answered this" state has been shown. */
  onConflictSettled: () => void;
  /** Cancel this decision's goal (#490) — kills the worker and marks it
   *  terminal; the list refreshes after. Absent for non-goal decisions. */
  onCancelGoal?: () => Promise<void>;
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
function effectTextFor(kind: string, answer: 'approve' | 'reject', agentName: string): string {
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
    return answer === 'approve'
      ? `Confirm approve — ${agentName} may go ahead with this action.`
      : `Confirm reject — ${agentName} will not go ahead with this.`;
  }
  if (kind === 'enrichment_proposal') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will save these details to the person's profile (your manual entries stay protected).`
      : 'Confirm reject — nothing is written to the profile.';
  }
  if (kind === 'tool_approval') {
    return answer === 'approve'
      ? `Confirm approve — ${agentName} will run this tool and continue the turn.`
      : `Confirm reject — ${agentName} will skip this tool and continue the turn.`;
  }
  // malformed and anything unknown: recorded only, no state change.
  return answer === 'approve'
    ? 'Confirm — this is recorded for the audit trail; nothing else changes.'
    : 'Confirm reject — this is recorded for the audit trail; nothing else changes.';
}

export function DecisionItem({ decision: d, onAnswer, onConflictSettled, onCancelGoal }: Props) {
  const { colors, reduceMotion } = useTheme();
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'Aria';
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
  const [cancelErr, setCancelErr] = useState<string | null>(null);
  const [answerErr, setAnswerErr] = useState<string | null>(null);
  const conflictTimer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => () => clearTimeout(conflictTimer.current), []);

  const submit = async (p: PendingAnswer) => {
    setSubmitting(true);
    setAnswerErr(null);
    try {
      const body: AnswerBody = { ...p.body, note: note.trim() ? note.trim() : undefined };
      const result = await onAnswer(d.id, body);
      if (!result.ok) {
        setConflict(true);
        setPending(null);
        conflictTimer.current = setTimeout(onConflictSettled, 1600);
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
      return;
    }
    setSubmitting(false);
  };

  const badge = badgeFor(d, colors);
  const isUnblock = d.kind === 'unblock';
  const isChoice = d.kind === 'choice';
  const isApprovalLike =
    d.kind === 'approve_review' || d.kind === 'risk_gate' || d.kind === 'malformed' ||
    d.kind === 'enrichment_proposal' || d.kind === 'automation_proposal' ||
    d.kind === 'tool_approval';
  // The agent's original draft, when this decision carries one (payload.draft):
  // enables "approve with edits" — revise the text, then accept (answer='edit').
  const draft = draftText(d);
  const options = choiceOptions(d);
  const recommendedId = recommendedChoiceId(d);
  const recommended = options.find(o => o.id === recommendedId) ?? null;
  // L2 digest lives on the goal card; only goal-bound reviews can have one.
  const hasEvidence = d.kind === 'approve_review' && !!d.goal_id && !!d.project_id;

  return (
    <div data-testid={`decision-${d.id}`} style={{
      padding: '14px 18px',
      borderBottom: `1px solid ${colors.border}`,
      fontFamily: font.body,
    }}>
      {/* Row 1: badge + plain-language headline + age */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{
          fontFamily: font.mono, fontSize: 9, letterSpacing: '0.06em',
          textTransform: 'uppercase', borderRadius: 4, padding: '2px 6px',
          flexShrink: 0, color: badge.color, background: badge.bg,
        }}>
          {badge.label}
        </span>
        <span style={{
          fontSize: 13, fontWeight: 600, color: colors.text, flex: 1,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {d.headline}
        </span>
        <span style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, flexShrink: 0 }}>
          {formatAge(d.created_at)}
        </span>
      </div>

      {/* Goal binding, where known (joined title — plain data). Click opens the
          shared goal-detail modal (#503) when the decision is goal-bound. */}
      {d.goal_title && (
        d.goal_id && d.project_id ? (
          <button
            onClick={() => openGoalDetail(d.project_id!, d.goal_id!)}
            title="View goal detail"
            style={{
              display: 'block', background: 'none', border: 'none', padding: 0,
              marginTop: 4, textAlign: 'left', cursor: 'pointer',
              fontSize: 11, color: colors.cyan, fontFamily: font.body,
            }}
          >
            Goal: {d.goal_title}
          </button>
        ) : (
          <div style={{ fontSize: 11, color: colors.textDim, marginTop: 4 }}>
            Goal: {d.goal_title}
          </div>
        )
      )}

      {/* Detail: technical why/attribution, verbatim (S2) */}
      {d.detail && (
        <div style={{
          fontSize: 12, color: colors.textMuted, marginTop: 4,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
          maxHeight: 96, overflow: 'auto',
        }}>
          {d.detail}
        </div>
      )}

      {/* Recommendation chip — only when the daemon marked a default option */}
      {recommended && (
        <div style={{ marginTop: 8 }}>
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 6,
            borderRadius: radius.pill, background: colors.cyanSoft,
            color: colors.cyan, fontSize: 11, fontWeight: 500, padding: '3px 10px',
          }}>
            {agentName} recommends · {recommended.label}
          </span>
        </div>
      )}

      {/* Conflict: already answered elsewhere */}
      {conflict ? (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8, marginTop: 10,
          borderRadius: radius.md, border: `1px solid ${colors.warning}`,
          background: colors.warning + '14', padding: '8px 12px',
          fontSize: 12, color: colors.text,
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
          <span style={{ fontSize: 12, color: colors.text, flex: 1, minWidth: 180 }}>
            {pending.effectText}
          </span>
          <Btn variant="primary" disabled={submitting} onClick={() => submit(pending)}>
            {submitting ? 'Sending…' : pending.confirmLabel}
          </Btn>
          <Btn disabled={submitting} onClick={() => { setPending(null); setAnswerErr(null); }}>Cancel</Btn>
          {answerErr && (
            <span role="alert" style={{ fontSize: 11, color: colors.danger, flexBasis: '100%' }}>
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
              fontFamily: font.body, fontSize: 12, padding: '8px 10px', outline: 'none',
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
          <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 6 }}>
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
              fontFamily: font.body, fontSize: 12, padding: '8px 10px', outline: 'none',
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
                  effectText: effectTextFor(d.kind, 'approve', agentName),
                })}
              >
                Approve
              </Btn>
              <Btn
                danger
                onClick={() => setPending({
                  body: { answer: 'reject' },
                  confirmLabel: 'Confirm reject',
                  effectText: effectTextFor(d.kind, 'reject', agentName),
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
            <button
              onClick={async () => {
                if (submitting) return;
                setSubmitting(true);
                setCancelErr(null);
                try {
                  await onCancelGoal();
                } catch (e) {
                  setCancelErr(e instanceof Error ? e.message : 'Cancel failed');
                } finally {
                  setSubmitting(false);
                }
              }}
              disabled={submitting}
              style={{
                background: 'none', border: 'none', color: colors.warning,
                fontSize: 11, fontFamily: font.body,
                cursor: submitting ? 'default' : 'pointer', padding: 4,
                opacity: submitting ? 0.5 : 1,
              }}
            >
              Cancel goal
            </button>
          )}

          {hasEvidence && (
            <button
              onClick={() => setEvidenceOpen(o => !o)}
              style={{
                marginLeft: 'auto', background: 'none', border: 'none',
                color: evidenceOpen ? colors.cyan : colors.textDim,
                fontSize: 11, fontFamily: font.body, cursor: 'pointer', padding: 4,
                transition: reduceMotion ? 'none' : `color 150ms ${ease.out}`,
              }}
            >
              Evidence {evidenceOpen ? '▾' : '▸'}
            </button>
          )}
        </div>
      )}

      {/* Cancel failure surfaced inline — never silently swallowed (#503) */}
      {cancelErr && (
        <div style={{
          marginTop: 8, fontSize: 12, color: colors.danger,
          borderRadius: radius.md, border: `1px solid ${colors.danger}`,
          background: colors.danger + '14', padding: '6px 10px',
        }}>
          {cancelErr}
        </div>
      )}

      {/* Note rides along with whatever answer is confirmed */}
      {noteOpen && !conflict && (
        <textarea
          value={note}
          onChange={e => setNote(e.target.value)}
          placeholder="Add a note — it travels with your answer"
          rows={2}
          style={{
            width: '100%', boxSizing: 'border-box', resize: 'vertical', marginTop: 8,
            borderRadius: radius.md, border: `1px solid ${colors.border}`,
            background: colors.inputBg, color: colors.text,
            fontFamily: font.body, fontSize: 12, padding: '8px 10px', outline: 'none',
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
    case 'unblock': return { label: 'unblock', color: colors.warning, bg: colors.warning + '24' };
    case 'choice': return { label: 'choice', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'risk_gate': return { label: 'permission', color: colors.danger, bg: colors.danger + '24' };
    case 'enrichment_proposal': return { label: 'enrichment', color: colors.purpleBright, bg: colors.purpleSoft };
    case 'malformed': return { label: 'review', color: colors.warning, bg: colors.warning + '24' };
    default: return { label: 'approval', color: colors.cyan, bg: colors.cyanSoft };
  }
}

/** Inline-styled button per the post-#273 convention (no shared atom yet). */
function Btn({ variant = 'ghost', danger = false, disabled = false, onClick, children }: {
  variant?: 'primary' | 'ghost';
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  const { colors, reduceMotion } = useTheme();
  const [hover, setHover] = useState(false);
  const primary = variant === 'primary';
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        borderRadius: radius.md, fontFamily: font.body, fontSize: 12,
        fontWeight: primary ? 600 : 500, padding: '5px 14px',
        cursor: disabled ? 'default' : 'pointer',
        opacity: disabled ? 0.6 : 1,
        border: primary ? 'none' : `1px solid ${hover && !disabled ? colors.borderHi : colors.border}`,
        background: primary ? colors.ribbonGradient : colors.surface,
        color: primary
          ? colors.textOnAccent
          : danger ? colors.danger
          : hover && !disabled ? colors.text : colors.textMuted,
        transition: reduceMotion ? 'none' : `all 150ms ${ease.out}`,
      }}
    >
      {children}
    </button>
  );
}
