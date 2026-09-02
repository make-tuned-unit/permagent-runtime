/**
 * Pending decisions in the live conversation — approve here instead of
 * leaving for the Decision Inbox.
 *
 * A different surface CLASS from Home's card (this one is in-turn: a tool
 * parks, an enrichment is proposed, and you are already talking to your agent),
 * so it stays. What it no longer has is its own idea of what a decision row
 * looks like.
 *
 * It used to carry a private `ChatDecisionCard` — a third independent build of
 * the answer contract, with its own confirm rule, its own error copy and its
 * own buttons. Which meant a Tier-2 approval read one way here and another way
 * in the Inbox, and every improvement to the canonical row (typed actions, the
 * informed-reject warning, approve-with-edits, the conflict state) stopped at
 * the dock's edge. One row component, one place (J3): this now renders the very
 * same `DecisionItem` Home's inbox does, so the answer contract is identical
 * wherever a decision appears, including the inline confirm step.
 *
 * The Inbox remains the full board (evidence, history, the +N more).
 */

import { useState, type CSSProperties } from 'react';
import { font, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { useDecisions } from '../dashboard/decisions/useDecisions';
import { DecisionInbox } from '../dashboard/decisions/DecisionInbox';
import { DecisionItem } from '../dashboard/decisions/DecisionItem';

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
      <div style={{ fontSize: textSize.caption }}>
        {decisions.slice(0, 3).map(d => (
          <DecisionItem
            key={d.id}
            decision={d}
            onAnswer={inbox.answer}
            // Answering here refreshes the same list the Inbox reads, so a
            // conflict settles the same way it does there: refetch and let the
            // item leave.
            onConflictSettled={inbox.refresh}
            onDiscardStaged={inbox.discardStaged}
          />
        ))}
      </div>
    </div>
  );
}
