/**
 * Decision Inbox — the sheet (Lane L4, redesigned 2026-09-25).
 *
 * One top-anchored 720px column on `DetailModal` (U9): ① header (position,
 * ‹ ›, and a quiet "N handled since …" link that replaced both the
 * "handled overnight" row and the History footer, U18), the review body
 * (②–⑧, DecisionReview), and ⑨ the action bar in the shell's footer.
 *
 * The separate "Parked goals" list is gone (U2): every parked goal with an
 * open unblock decision is already in the queue, so it showed the same goal
 * twice. Zero batch/multi-select affordances — answers are one at a time
 * (Jesse's delegated ruling, 2026-09-25).
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { useCommandCenter } from '../../../lib/store';
import { useLiveGoals } from '../../../lib/useLiveGoals';
import { font, radius, space, type } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
import { Chip } from '../../common/Chip';
import { DetailModal } from '../../common/DetailModal';
import { SkeletonText } from '../../common/Skeleton';
import { StateBlock } from '../../common/StateBlock';
import type { useDecisions } from './useDecisions';
import type { HistoryItem } from './types';
import { resolutionText, deadLetterText } from './types';
import {
  DecisionReview, LoadingShape, REVIEW_PAD_X, ReviewActionBar, ReviewHeader,
  useDecisionReviewState,
} from './DecisionReview';
import { handledLinkLabel, handledSentence, handledSummary } from './DecisionInbox.handled';
import { ageLabel } from './decisionEvidence';
import { usePersona } from '../../settings/useSettings';

interface Props {
  inbox: ReturnType<typeof useDecisions>;
  onClose: () => void;
  placement?: 'center' | 'contained';
}

/** Focusable controls, for trapping Tab inside a dialog stacked on this one. */
const FOCUSABLE = 'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

const quietLink = (colors: ReturnType<typeof useTheme>['colors']): CSSProperties => ({
  '--pa-btn-fg': colors.textMuted,
  '--pa-btn-fg-hover': colors.text,
  '--pa-btn-bg-hover': colors.fillHover,
  '--pa-btn-bg-active': colors.fillActive,
  '--pa-btn-pad': `${space.xs}px ${space.md}px`,
  '--pa-btn-weight': 400,
  fontFamily: font.body, fontSize: type.caption.fontSize,
} as CSSProperties);

export function DecisionInbox({ inbox, onClose, placement = 'center' }: Props) {
  const { colors } = useTheme();
  const goalDetail = useCommandCenter(state => state.goalDetail);
  const { data, loading, error } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'Henry';
  const review = useDecisionReviewState(inbox, agentName);
  const [view, setView] = useState<'list' | 'history'>('list');
  const [historyFilter, setHistoryFilter] = useState<'agent' | 'all'>('all');
  const [history, setHistory] = useState<HistoryItem[] | null>(null);
  /** A failed history read must not render as an empty audit trail. */
  const [historyError, setHistoryError] = useState(false);

  const loadHistory = useCallback(async () => {
    setHistoryError(false);
    try {
      setHistory(await inbox.loadHistory());
    } catch { setHistoryError(true); }
  }, [inbox]);

  const openHistory = useCallback(async (filter: 'agent' | 'all') => {
    setHistoryFilter(filter);
    setView('history');
    review.setShortcutsOpen(false);
    if (history === null) await loadHistory();
  }, [history, loadHistory, review]);

  const showAll = inbox.showAll;
  useEffect(() => { void showAll(); }, [showAll]);
  const handled = handledSummary(data);
  // #464/#515: the live shared source (event-driven, parked/archived excluded).
  const { activeCount: liveGoalCount, loaded: liveGoalsLoaded } = useLiveGoals();
  const goals = liveGoalsLoaded ? liveGoalCount : (data?.goals_in_flight ?? 0);

  // A dialog stacked on this one (ConfirmDialog) or the shortcut popover owns
  // Escape and Tab while it is up. Both shells listen on `document`, so this
  // runs in the capture phase and stops the event before either sees it.
  const overlay = review.confirm ? 'confirm' : review.shortcutsOpen ? 'sheet' : null;
  const { setConfirm, setShortcutsOpen } = review;
  useEffect(() => {
    if (!overlay) return;
    const guard = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        e.preventDefault();
        if (overlay === 'confirm') setConfirm(null); else setShortcutsOpen(false);
        return;
      }
      if (e.key !== 'Tab' || overlay !== 'confirm') return;
      const dialogs = document.querySelectorAll<HTMLElement>('[role="dialog"][aria-modal="true"]');
      const top = dialogs[dialogs.length - 1];
      if (!top) return;
      const items = Array.from(top.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) return;
      e.stopPropagation();
      e.preventDefault();
      const index = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey
        ? items[(index <= 0 ? items.length : index) - 1]
        : items[(index + 1) % items.length];
      next.focus();
    };
    document.addEventListener('keydown', guard, true);
    return () => document.removeEventListener('keydown', guard, true);
  }, [overlay, setConfirm, setShortcutsOpen]);

  const inList = view === 'list';
  const coldLoading = loading && !data;
  const coldError = error && !data;
  const empty = !coldLoading && !coldError && review.complete && review.queue.ordered.length === 0;
  const reviewing = inList && !coldLoading && !coldError && !empty;
  const showBar = reviewing && review.complete && !!review.d && !review.report;

  return (
    // Suspend this shell's trap and Escape while the goal evidence overlay is
    // open; review state above the shell survives the body remount.
    <div style={{ display: goalDetail ? 'none' : 'contents' }}>
      <DetailModal
        title={inList ? '' : historyFilter === 'agent' ? `What ${agentName} did` : 'History'}
        ariaLabel="Decisions that need you"
        onClose={() => { if (!goalDetail && !review.confirm) onClose(); }}
        width="min(720px, 92vw)"
        align="top"
        placement={goalDetail ? 'contained' : placement}
        headerLeft={inList ? (
          <ReviewHeader state={review} />
        ) : (
          <Button colors={colors} variant="bare" type="button" onClick={() => setView('list')} style={quietLink(colors)}>
            ← Back
          </Button>
        )}
        headerRight={inList && !coldError ? (
          <Button
            colors={colors}
            variant="bare"
            type="button"
            data-testid="handled-link"
            onClick={() => { void openHistory(handled.count > 0 ? 'agent' : 'all'); }}
            style={quietLink(colors)}
          >
            {handledLinkLabel(handled)} ›
          </Button>
        ) : undefined}
        bodyStyle={{ padding: `${space.huge}px ${REVIEW_PAD_X}px` }}
        footer={showBar ? <ReviewActionBar state={review} /> : undefined}
        footerStyle={{ padding: `${space.xl}px ${REVIEW_PAD_X}px` }}
      >
        <div data-testid="inbox-body">
          {view === 'history' && (
            <HistoryList
              items={history}
              failed={historyError}
              onRetry={loadHistory}
              filter={historyFilter}
              onShowAll={() => setHistoryFilter('all')}
              agentName={agentName}
            />
          )}
          {inList && coldLoading && <LoadingShape />}
          {inList && coldError && (
            // A dead daemon must not read as the all-clear (2026-07 audit D4).
            <StateBlock
              tone="error"
              title="Couldn't reach the decision inbox."
              detail="This is a connection problem, not an empty inbox."
              onRetry={() => { void inbox.refresh(); }}
            />
          )}
          {inList && empty && (
            <div className="pa-enter" style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
              <StateBlock
                tone="empty"
                title="Nothing needs you"
                detail={handledSentence(handled, agentName) ?? `${goals} goal${goals === 1 ? '' : 's'} in flight.`}
              />
              {handled.count > 0 && (
                <Button colors={colors} variant="ghost" type="button" onClick={() => { void openHistory('agent'); }}>
                  Review what {agentName} did
                </Button>
              )}
            </div>
          )}
          {/* Mounted while a verdict is held on an empty queue too, so Z and
              the toast's Undo keep working after the last answer. */}
          <DecisionReview inbox={inbox} agentName={agentName} state={review} active={reviewing || (inList && empty)} />
        </div>
      </DetailModal>
    </div>
  );
}

function isAgentActed(item: HistoryItem): boolean {
  return item.acted_by === 'henry-policy' || item.acted_by === 'system' || item.tier === 1;
}

/** Read-only audit list: every resolved decision, or only the agent's own. */
function HistoryList({ items, failed, onRetry, filter, onShowAll, agentName }: {
  items: HistoryItem[] | null;
  failed: boolean;
  onRetry: () => void;
  filter: 'agent' | 'all';
  onShowAll: () => void;
  agentName: string;
}) {
  const { colors } = useTheme();
  if (failed) {
    return (
      <StateBlock
        tone="error"
        compact
        title="Couldn't load the decision history."
        detail="This is a connection problem, not an empty record — decisions you have already answered are still there."
        onRetry={onRetry}
      />
    );
  }
  if (items === null) {
    return <SkeletonText lines={6} />;
  }
  const rows = filter === 'agent' ? items.filter(isAgentActed) : items;
  if (rows.length === 0) {
    return <StateBlock tone="empty" compact title="Nothing here yet." />;
  }
  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {filter === 'agent' && (
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: space.md }}>
          <Button colors={colors} variant="bare" type="button" onClick={onShowAll} style={quietLink(colors)}>Show everything</Button>
        </div>
      )}
      {rows.map((item, index) => (
        <div
          key={`${item.id}-${item.seq}`}
          className="pa-enter"
          style={{
            padding: `${space.xl}px 0`,
            borderTop: index === 0 ? undefined : `1px solid ${colors.border}`,
            fontFamily: font.body,
            ['--pa-enter-delay' as string]: `${Math.min(index, 8) * 32}ms`,
          } as CSSProperties}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: space.md }}>
            <Chip kind="static" tone={isAgentActed(item) ? 'success' : 'accent'}>
              {isAgentActed(item) ? 'auto' : 'you'}
            </Chip>
            <span style={{
              ...type.small, fontWeight: 500, color: colors.text, flex: 1, minWidth: 0,
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            }}>
              {item.headline}
            </span>
            <span style={{ ...type.caption, color: colors.textDim, flexShrink: 0 }}>
              {ageLabel(item.created_at)}
            </span>
          </div>
          <div style={{ ...type.caption, color: colors.textMuted, marginTop: space.xs }}>
            {resolutionText(item, agentName)}
          </div>
          {/* CASE A fix #4: the answer succeeded but its gated effect never
              took effect after every retry. */}
          {deadLetterText(item) && (
            <div
              role="alert"
              style={{
                marginTop: space.sm, ...type.caption, color: colors.danger,
                borderRadius: radius.md, border: `1px solid ${colors.danger}`,
                padding: `${space.sm}px ${space.lg}px`,
              }}
            >
              {deadLetterText(item)}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
