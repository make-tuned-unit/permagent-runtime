/**
 * Pending decisions in the live conversation — approve here instead of
 * leaving for the Decision Inbox.
 *
 * The Inbox remains the full board (evidence, edit-and-accept, history).
 * This strip is the in-turn path: tool parks, enrichment proposals, and
 * anything else waiting while the user is already talking to Henry.
 */

import { useState, type CSSProperties } from 'react';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { useDecisions } from '../dashboard/decisions/useDecisions';
import { DecisionInbox } from '../dashboard/decisions/DecisionInbox';
import type { AnswerBody, Decision } from '../dashboard/decisions/types';
import { choiceOptions } from '../dashboard/decisions/types';

function isBinary(d: Decision): boolean {
  return d.kind !== 'choice' && d.kind !== 'unblock';
}

function needsConfirm(d: Decision): boolean {
  return d.kind === 'risk_gate' || d.kind === 'approve_review' || d.kind === 'session_gate';
}

export function ChatPendingDecisions({ overlay = false }: { overlay?: boolean }) {
  const { colors } = useTheme();
  const inbox = useDecisions();
  const [inboxOpen, setInboxOpen] = useState(false);
  const decisions = inbox.data?.decisions ?? [];
  if (decisions.length === 0) return null;

  const hidden = decisions.length - Math.min(decisions.length, 3);

  return (
    <div
      data-testid="chat-pending-decisions"
      onClick={e => e.stopPropagation()}
      onPointerDown={e => e.stopPropagation()}
      style={{
        flexShrink: 0,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        padding: overlay ? 0 : '8px 12px 0',
        maxHeight: overlay ? 220 : 180,
        overflowY: 'auto',
      }}
    >
      {/* These cards are global — they float above the composer in every
          conversation, whether or not the chat you are having has anything to
          do with them. Unlabeled, an approval card that appeared mid-sentence
          read as a response to what you had just typed. One line says where
          they came from, and offers the board they came from. */}
      <div
        data-testid="chat-decisions-source"
        style={{
          display: 'flex', alignItems: 'baseline', gap: 8, flexWrap: 'wrap',
          fontFamily: font.body, fontSize: 10, color: colors.textDim,
        }}
      >
        <span>From your Decision Inbox</span>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          className="hover:underline"
          onClick={() => setInboxOpen(true)}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            '--pa-btn-weight': 600,
            fontFamily: font.body,
            fontSize: 10,
          } as CSSProperties}
        >
          {hidden > 0 ? `Open the Inbox · ${hidden} more waiting` : 'Open the Inbox'}
        </Button>
      </div>
      {inboxOpen && <DecisionInbox inbox={inbox} onClose={() => setInboxOpen(false)} />}
      {decisions.slice(0, 3).map(d => (
        <ChatDecisionCard
          key={d.id}
          decision={d}
          onAnswer={inbox.answer}
          colors={colors}
        />
      ))}
    </div>
  );
}

function ChatDecisionCard({
  decision: d,
  onAnswer,
  colors,
}: {
  decision: Decision;
  onAnswer: (id: string, body: AnswerBody) => Promise<{ ok: boolean }>;
  colors: ReturnType<typeof useTheme>['colors'];
}) {
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<AnswerBody | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const options = choiceOptions(d);

  const submit = async (body: AnswerBody) => {
    if (busy) return;
    setBusy(true);
    setErr(null);
    try {
      const result = await onAnswer(d.id, body);
      if (!result.ok) setErr('Already answered elsewhere — refresh.');
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Couldn't send");
      setBusy(false);
      return;
    }
    setBusy(false);
    setPending(null);
  };

  const askOrGo = (body: AnswerBody) => {
    if (needsConfirm(d) && !pending) {
      setPending(body);
      return;
    }
    void submit(body);
  };

  return (
    <div
      style={{
        border: `1px solid ${colors.borderHi}`,
        borderRadius: radius.md,
        background: colors.surfaceHi,
        padding: '10px 12px',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <div style={{ fontFamily: font.body, fontSize: 12, color: colors.text, lineHeight: 1.4 }}>
        {d.headline}
      </div>
      {err && (
        <div style={{ fontSize: 11, color: colors.danger }}>{err}</div>
      )}
      {pending ? (
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
          <span style={{ fontSize: 11, color: colors.textMuted }}>
            {pending.answer === 'reject' ? 'Confirm reject?' : 'Confirm approve?'}
          </span>
          <Button
            colors={colors}
            variant="ghostOn"
            type="button"
            disabled={busy}
            onClick={() => void submit(pending)}
            style={btn(colors, true)}
          >
            {busy ? 'Sending…' : 'Confirm'}
          </Button>
          <Button
            colors={colors}
            type="button"
            disabled={busy}
            onClick={() => setPending(null)}
            style={btn(colors, false)}
          >
            Cancel
          </Button>
        </div>
      ) : (
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          {isBinary(d) && (
            <>
              <Button
                colors={colors}
                variant="ghostOn"
                type="button"
                disabled={busy}
                onClick={() => askOrGo({ answer: 'approve' })}
                style={btn(colors, true)}
              >
                {busy ? '…' : 'Approve'}
              </Button>
              <Button
                colors={colors}
                type="button"
                disabled={busy}
                onClick={() => askOrGo({ answer: 'reject' })}
                style={btn(colors, false)}
              >
                Reject
              </Button>
            </>
          )}
          {options.map(opt => (
            <Button
              colors={colors}
              key={opt.id}
              type="button"
              disabled={busy}
              onClick={() => void submit({ answer: 'choice', choice_id: opt.id })}
              style={btn(colors, false)}
            >
              {opt.label}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Card actions. `primary` rides the `ghostOn` variant (cyan hairline) with the
 *  soft cyan wash it has always had; the rest are plain `ghost`. Everything the
 *  eye sees goes through `--pa-btn-*` rather than inline `background`/`border`,
 *  because an inline declaration outranks `.pa-btn:hover` and would leave these
 *  as unpressable-looking as they were before. */
function btn(colors: ReturnType<typeof useTheme>['colors'], primary: boolean): CSSProperties {
  return {
    ...(primary
      ? {
        '--pa-btn-bg': colors.cyanSoft,
        '--pa-btn-bg-hover': `${colors.cyan}33`,
      }
      : {}),
    '--pa-btn-pad': '5px 10px',
    '--pa-btn-radius': `${radius.sm}px`,
    '--pa-btn-weight': 600,
    fontFamily: font.body,
    fontSize: 11,
  } as CSSProperties;
}
