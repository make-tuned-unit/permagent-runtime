/**
 * Decision Inbox — overlay (Lane L4).
 *
 * Full-screen scrim overlay rendered from inside dashboard/ (AddCardPicker
 * pattern) so the workspaces/ mount point stays untouched. Ranked Tier-2 +
 * unblock items, "+M more" overflow, collapsed Tier-1 group, history view.
 * Zero batch/multi-select affordances anywhere — answers are one at a time.
 */

import { useState, useCallback } from 'react';
import { useLiveGoals } from '../../../lib/useLiveGoals';
import { FiX } from 'react-icons/fi';
import { font, radius, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import type { useDecisions } from './useDecisions';
import type { HistoryItem } from './types';
import { resolutionText } from './types';
import { DecisionItem } from './DecisionItem';
import { decisionsClient } from './client';
import { formatAge } from './format';
import { usePersona } from '../../settings/useSettings';

interface Props {
  inbox: ReturnType<typeof useDecisions>;
  onClose: () => void;
}

export function DecisionInbox({ inbox, onClose }: Props) {
  const { colors, reduceMotion } = useTheme();
  const { data, loading, error } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'Aria';
  const [view, setView] = useState<'list' | 'history'>('list');
  const [history, setHistory] = useState<HistoryItem[] | null>(null);
  const [tier1Open, setTier1Open] = useState(false);

  const openHistory = useCallback(async () => {
    setView('history');
    if (history === null) {
      try { setHistory(await inbox.loadHistory()); } catch { setHistory([]); }
    }
  }, [history, inbox]);

  const toggleTier1 = useCallback(async () => {
    const next = !tier1Open;
    setTier1Open(next);
    if (next && history === null) {
      try { setHistory(await inbox.loadHistory()); } catch { setHistory([]); }
    }
  }, [tier1Open, history, inbox]);

  const decisions = data?.decisions ?? [];
  const total = data?.total_pending ?? 0;
  const handled = data?.handled_count ?? 0;
  // #464/#515: the live shared source (event-driven, parked/archived excluded)
  // — the summary number is only a fallback and now shares its definition.
  const { activeCount: liveGoalCount, loaded: liveGoalsLoaded } = useLiveGoals();
  const goals = liveGoalsLoaded ? liveGoalCount : (data?.goals_in_flight ?? 0);
  const moreCount = total - decisions.length;
  const tier1Rows = (history ?? []).filter(d => d.tier === 1);

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 100,
        background: 'rgba(0,0,0,0.5)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 'min(720px, 92vw)', maxHeight: '86vh',
          borderRadius: radius.lg,
          background: colors.surface,
          border: `1px solid ${colors.border}`,
          boxShadow: [colors.cardShadow, colors.cardHighlight].filter(Boolean).join(', '),
          overflow: 'hidden',
          display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10,
          padding: '14px 18px',
          borderBottom: `1px solid ${colors.border}`,
        }}>
          {view === 'history' && (
            <button
              onClick={() => setView('list')}
              style={{
                background: 'none', border: 'none', cursor: 'pointer',
                color: colors.textMuted, fontFamily: font.body, fontSize: 12,
                padding: '2px 4px',
              }}
            >
              ← Back
            </button>
          )}
          <span style={{ fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text, flex: 1 }}>
            {view === 'history' ? 'History' : 'Decision inbox'}
          </span>
          {view === 'list' && total > 0 && (
            <span style={{
              fontFamily: font.mono, fontSize: 11, color: colors.cyan,
              background: colors.cyanSoft, borderRadius: radius.pill, padding: '2px 8px',
            }}>
              {total} pending
            </span>
          )}
          <button
            onClick={onClose}
            title="Close"
            style={{
              background: 'none', border: 'none', color: colors.textMuted,
              cursor: 'pointer', padding: 4, display: 'flex',
            }}
          >
            <FiX size={16} />
          </button>
        </div>

        {/* Body */}
        <div data-testid="inbox-body" style={{ overflow: 'auto', padding: '6px 0', flex: 1 }}>
          {view === 'history' ? (
            <HistoryList items={history} />
          ) : loading && !data ? (
            <div style={{ padding: '48px 18px', textAlign: 'center', fontSize: 12, color: colors.textDim }}>
              Checking with {agentName}…
            </div>
          ) : error && !data ? (
            /* Cold load failure — must NOT read as the "No decisions needed"
               all-clear (2026-07 wiring audit D4): a dead daemon looked
               identical to a clear inbox. */
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>
                Couldn't reach the decision inbox.
              </div>
              <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 14 }}>
                This is a connection problem, not an empty inbox.
              </div>
              <button
                onClick={() => inbox.refresh()}
                style={{
                  fontFamily: font.body, fontSize: 12, fontWeight: 600, color: colors.cyan,
                  background: 'none', border: `1px solid ${colors.borderHi}`, borderRadius: 8,
                  padding: '5px 14px', cursor: 'pointer',
                }}
              >
                Retry
              </button>
            </div>
          ) : total === 0 ? (
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{
                width: 34, height: 34, borderRadius: '50%',
                background: colors.success + '26', color: colors.success,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: 16, margin: '0 auto 10px',
              }}>✓</div>
              <div style={{ fontSize: 13, color: colors.textMuted, marginBottom: 4 }}>No decisions needed.</div>
              <div style={{ fontSize: 11, color: colors.textDim }}>
                {goals} goal{goals === 1 ? '' : 's'} in flight.
              </div>
            </div>
          ) : (
            <>
              {decisions.map(d => (
                <DecisionItem
                  key={d.id}
                  decision={d}
                  onAnswer={inbox.answer}
                  onConflictSettled={inbox.refresh}
                  onCancelGoal={
                    d.goal_id && d.project_id
                      ? async () => {
                          await decisionsClient.cancelGoal(d.project_id!, d.goal_id!);
                          inbox.refresh();
                        }
                      : undefined
                  }
                />
              ))}

              {moreCount > 0 && (
                <button
                  onClick={() => { inbox.showAll(); }}
                  style={{
                    display: 'block', width: '100%', textAlign: 'center',
                    padding: 10, background: 'none', border: 'none',
                    color: colors.cyan, fontFamily: font.body, fontSize: 12,
                    fontWeight: 500, cursor: 'pointer',
                  }}
                >
                  +{moreCount} more decision{moreCount === 1 ? '' : 's'}
                </button>
              )}

              {handled > 0 && (
                <>
                  <button
                    onClick={toggleTier1}
                    style={{
                      display: 'flex', alignItems: 'center', gap: 10, width: '100%',
                      padding: '12px 18px', background: 'none', border: 'none',
                      borderTop: `1px solid ${colors.border}`,
                      cursor: 'pointer', textAlign: 'left', fontFamily: font.body,
                    }}
                  >
                    <span style={{
                      width: 8, height: 8, borderRadius: '50%',
                      background: colors.success, flexShrink: 0,
                    }} />
                    <span style={{ fontSize: 13, fontWeight: 500, color: colors.text, flex: 1 }}>
                      {agentName} handled {handled} routine item{handled === 1 ? '' : 's'} overnight
                    </span>
                    <span style={{
                      color: colors.textDim, fontSize: 11,
                      transform: tier1Open ? 'rotate(90deg)' : 'none',
                      transition: reduceMotion ? 'none' : `transform 150ms ${ease.out}`,
                    }}>▸</span>
                  </button>
                  {tier1Open && (
                    <div>
                      {history === null ? (
                        <div style={{ padding: '10px 18px 10px 36px', fontSize: 12, color: colors.textDim }}>
                          Loading…
                        </div>
                      ) : (
                        tier1Rows.map(row => (
                          <div
                            key={row.id}
                            style={{
                              display: 'flex', alignItems: 'center', gap: 10,
                              padding: '8px 18px 8px 36px', fontSize: 12,
                              color: colors.textMuted,
                              borderTop: `1px solid ${colors.border}`,
                              fontFamily: font.body,
                            }}
                          >
                            <span style={{
                              flex: 1, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
                            }}>
                              {row.headline}
                            </span>
                            <span style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, flexShrink: 0 }}>
                              {formatAge(row.created_at)}
                            </span>
                            <button
                              onClick={openHistory}
                              style={{
                                background: 'none', border: 'none', cursor: 'pointer',
                                color: colors.cyan, fontSize: 11, fontFamily: font.body,
                                padding: 0, flexShrink: 0,
                              }}
                            >
                              audit →
                            </button>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        {view === 'list' && (
          <div style={{
            padding: '10px 18px', borderTop: `1px solid ${colors.border}`,
            display: 'flex', justifyContent: 'flex-end',
          }}>
            <button
              onClick={openHistory}
              style={{
                background: 'none', border: 'none', cursor: 'pointer',
                color: colors.textDim, fontSize: 12, fontFamily: font.body, padding: 0,
              }}
            >
              History →
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/** Read-only audit list: every resolved decision, incl. Tier-1 auto-handled. */
function HistoryList({ items }: { items: HistoryItem[] | null }) {
  const { colors } = useTheme();
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'Aria';
  if (items === null) {
    return (
      <div style={{ padding: '32px 18px', textAlign: 'center', fontSize: 12, color: colors.textDim }}>
        Loading…
      </div>
    );
  }
  if (items.length === 0) {
    return (
      <div style={{ padding: '32px 18px', textAlign: 'center', fontSize: 12, color: colors.textDim }}>
        Nothing here yet.
      </div>
    );
  }
  return (
    <div>
      {items.map(item => (
        <div
          key={item.id}
          style={{
            padding: '12px 18px',
            borderBottom: `1px solid ${colors.border}`,
            fontFamily: font.body,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{
              fontFamily: font.mono, fontSize: 9, letterSpacing: '0.06em',
              textTransform: 'uppercase', borderRadius: 4, padding: '2px 6px',
              flexShrink: 0,
              color: item.tier === 1 ? colors.success : colors.cyan,
              background: item.tier === 1 ? colors.success + '26' : colors.cyanSoft,
            }}>
              {item.tier === 1 ? agentName : 'you'}
            </span>
            <span style={{
              fontSize: 13, fontWeight: 500, color: colors.text, flex: 1,
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            }}>
              {item.headline}
            </span>
            <span style={{ fontFamily: font.mono, fontSize: 11, color: colors.textDim, flexShrink: 0 }}>
              {formatAge(item.created_at)}
            </span>
          </div>
          <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 4 }}>
            {resolutionText(item, agentName)}
          </div>
        </div>
      ))}
    </div>
  );
}
