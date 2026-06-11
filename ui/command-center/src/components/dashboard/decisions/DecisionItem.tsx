/**
 * Decision Inbox — single decision row (Lane L4).
 *
 * Plain words first (headline → detail), technical detail one tap deeper
 * (evidence summary), raw output one tap below that (Show details).
 * Tier-2 answers are confirmed individually, inline — there is deliberately
 * no checkbox, no select-all, no batch affordance.
 *
 * S2: every string from the daemon renders as a React text node. No markdown,
 * no auto-links, no dangerouslySetInnerHTML.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { font, radius, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import type { AnswerBody, Decision } from './types';
import type { AnswerResult } from './useDecisions';
import { EvidenceDigest } from './EvidenceDigest';
import { formatAge } from './format';

interface Props {
  decision: Decision;
  onAnswer: (id: string, body: AnswerBody) => Promise<AnswerResult>;
  /** Called after the "someone already answered this" state has been shown. */
  onConflictSettled: () => void;
}

interface PendingAnswer {
  body: AnswerBody;
  confirmLabel: string;
  /** Plain-language restatement shown in the confirm row. */
  effectText: string;
}

export function DecisionItem({ decision: d, onAnswer, onConflictSettled }: Props) {
  const { colors, reduceMotion } = useTheme();
  const [pending, setPending] = useState<PendingAnswer | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [noteOpen, setNoteOpen] = useState(false);
  const [note, setNote] = useState('');
  const [inputOpen, setInputOpen] = useState(false);
  const [inputText, setInputText] = useState('');
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const conflictTimer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => () => clearTimeout(conflictTimer.current), []);

  const submit = async (p: PendingAnswer) => {
    setSubmitting(true);
    try {
      const body: AnswerBody = { ...p.body, note: note.trim() ? note.trim() : undefined };
      const result = await onAnswer(d.id, body);
      if (!result.ok) {
        setConflict(true);
        setPending(null);
        conflictTimer.current = setTimeout(onConflictSettled, 1600);
      }
      // On success the refreshed list drops this item; nothing else to do.
    } catch {
      setSubmitting(false); // network error: return to confirm state, stale data stays
      return;
    }
    setSubmitting(false);
  };

  const badge = badgeFor(d, colors);
  const isUnblock = d.kind === 'unblock';
  const isChoice = d.kind === 'choice';
  const recommended = d.recommendation?.label;

  return (
    <div style={{
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

      {/* Detail line */}
      {d.detail && (
        <div style={{
          fontSize: 12, color: colors.textMuted, marginTop: 4,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {d.detail}
        </div>
      )}

      {/* Unblock: the worker's question, verbatim and quoted */}
      {isUnblock && d.specific_ask && (
        <div style={{
          marginTop: 8, padding: '8px 12px',
          borderLeft: `2px solid ${colors.warning}`,
          background: colors.warning + '14',
          borderRadius: radius.sm,
          fontSize: 12, color: colors.text, fontStyle: 'italic',
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
        }}>
          “{d.specific_ask}”
        </div>
      )}

      {/* Effect line, verbatim from the daemon */}
      {d.effect_summary && (
        <div style={{
          fontSize: 12, color: colors.textMuted, marginTop: 6,
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {d.effect_summary}
        </div>
      )}

      {/* Recommendation chip — informational, not a button */}
      {d.recommendation && (
        <div style={{ marginTop: 8 }}>
          <span style={{
            display: 'inline-flex', alignItems: 'center', gap: 6,
            borderRadius: radius.pill, background: colors.cyanSoft,
            color: colors.cyan, fontSize: 11, fontWeight: 500, padding: '3px 10px',
          }}>
            Henry recommends · {d.recommendation.label}
            {d.recommendation.confidence ? ` (${d.recommendation.confidence} confidence)` : ''}
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
          <Btn disabled={submitting} onClick={() => setPending(null)}>Cancel</Btn>
        </div>
      ) : inputOpen ? (
        /* Freeform answer (unblock "Other…") */
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
      ) : (
        /* Action row per kind (A4): binary approvals get Approve/Reject/Add note
           only; option chips appear only on choice-kind (and option unblocks). */
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
          {d.kind === 'approval' && (
            <>
              <Btn
                variant="primary"
                onClick={() => setPending({
                  body: { answer: 'approve' },
                  confirmLabel: 'Confirm approve',
                  effectText: `Confirm approve — ${d.effect_summary}`,
                })}
              >
                Approve
              </Btn>
              <Btn
                danger
                onClick={() => setPending({
                  body: { answer: 'reject' },
                  confirmLabel: 'Confirm reject',
                  effectText: 'Confirm reject — Henry will not go ahead with this.',
                })}
              >
                Reject
              </Btn>
            </>
          )}

          {(isChoice || isUnblock) && d.options?.map(opt => (
            <Btn
              key={opt.id}
              variant={opt.label === recommended ? 'primary' : 'ghost'}
              onClick={() => setPending({
                body: { answer: 'choice', choice_id: opt.id },
                confirmLabel: 'Confirm choice',
                effectText: `Confirm “${opt.label}” — ${opt.effect_summary}`,
              })}
            >
              {opt.label}
            </Btn>
          ))}

          {isUnblock && (
            <Btn onClick={() => setInputOpen(true)}>
              {d.options?.length ? 'Other…' : 'Send answer'}
            </Btn>
          )}

          <Btn onClick={() => setNoteOpen(o => !o)}>Add note</Btn>

          {d.evidence && (
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

      {/* Layered evidence (A3) */}
      {evidenceOpen && d.evidence && <EvidenceDigest evidence={d.evidence} />}
    </div>
  );
}

function badgeFor(d: Decision, colors: ReturnType<typeof useTheme>['colors']) {
  if (d.kind === 'unblock') return { label: 'unblock', color: colors.warning, bg: colors.warning + '24' };
  if (d.kind === 'choice') return { label: 'choice', color: colors.purpleBright, bg: colors.purpleSoft };
  return { label: 'approval', color: colors.cyan, bg: colors.cyanSoft };
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
