/**
 * Decision Inbox — overlay (Lane L4).
 *
 * Full-screen scrim overlay rendered from inside dashboard/ (AddCardPicker
 * pattern) so the workspaces/ mount point stays untouched. Ranked Tier-2 +
 * unblock items, "+M more" overflow, collapsed Tier-1 group, history view.
 * Zero batch/multi-select affordances anywhere — answers are one at a time.
 */

import { useState, useCallback, useId, type CSSProperties } from 'react';
import { useLiveGoals } from '../../../lib/useLiveGoals';
import { FiX } from 'react-icons/fi';
import { font, radius, ease, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { Button } from '../../common/Button';
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
  const agentName = persona?.display_name ?? 'your agent';
  const [view, setView] = useState<'list' | 'history'>('list');
  const [history, setHistory] = useState<HistoryItem[] | null>(null);
  const [tier1Open, setTier1Open] = useState(false);
  const tier1Id = useId();

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
  const attentionGoals = data?.attention_goals ?? [];
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
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => setView('list')}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': colors.border,
                '--pa-btn-pad': '2px 4px',
                '--pa-btn-radius': `${radius.xs}px`,
                '--pa-btn-weight': 400,
                fontFamily: font.body, fontSize: textSize.caption,
              } as CSSProperties}
            >
              ← Back
            </Button>
          )}
          <span style={{ fontFamily: font.display, fontSize: textSize.body, fontWeight: 600, color: colors.text, flex: 1 }}>
            {view === 'history' ? 'History' : 'Decision inbox'}
          </span>
          {view === 'list' && total > 0 && (
            <span style={{
              fontFamily: font.mono, fontSize: textSize.micro, color: colors.cyan,
              background: colors.cyanSoft, borderRadius: radius.pill, padding: '2px 8px',
            }}>
              {total} pending
            </span>
          )}
          <Button
            colors={colors}
            variant="bare"
            type="button"
            onClick={onClose}
            title="Close"
            aria-label="Close"
            style={{
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-bg-hover': colors.border,
              '--pa-btn-pad': '4px',
              '--pa-btn-radius': `${radius.xs}px`,
            } as CSSProperties}
          >
            <FiX size={16} />
          </Button>
        </div>

        {/* Body */}
        <div data-testid="inbox-body" style={{ overflow: 'auto', padding: '6px 0', flex: 1 }}>
          {view === 'history' ? (
            <HistoryList items={history} />
          ) : loading && !data ? (
            <div style={{ padding: '48px 18px', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
              Checking with {agentName}…
            </div>
          ) : error && !data ? (
            /* Cold load failure — must NOT read as the "No decisions needed"
               all-clear (2026-07 wiring audit D4): a dead daemon looked
               identical to a clear inbox. */
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: 4 }}>
                Couldn't reach the decision inbox.
              </div>
              <div style={{ fontSize: textSize.micro, color: colors.textDim, marginBottom: 14 }}>
                This is a connection problem, not an empty inbox.
              </div>
              {/* No success tick: `refresh` swallows its own failure, so a
                  tick would claim a load that never landed. The list replacing
                  this dead end is the only honest confirmation. */}
              <Button
                colors={colors}
                type="button"
                flashSuccess={false}
                onClick={() => inbox.refresh()}
                style={{
                  '--pa-btn-bg': 'transparent',
                  '--pa-btn-fg': colors.cyan,
                  '--pa-btn-border': colors.borderHi,
                  '--pa-btn-bg-hover': colors.cyanSoft,
                  '--pa-btn-fg-hover': colors.cyan,
                  '--pa-btn-border-hover': colors.cyan,
                  '--pa-btn-bg-active': colors.cyanGlow,
                  '--pa-btn-pad': '5px 14px',
                  '--pa-btn-radius': `${radius.md}px`,
                  '--pa-btn-weight': 600,
                  fontFamily: font.body, fontSize: textSize.caption,
                } as CSSProperties}
              >
                Retry
              </Button>
            </div>
          ) : total === 0 && attentionGoals.length === 0 ? (
            <div style={{ padding: '48px 18px', textAlign: 'center' }}>
              <div style={{
                width: 34, height: 34, borderRadius: '50%',
                background: colors.success + '26', color: colors.success,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                fontSize: textSize.heading, margin: '0 auto 10px',
              }}>✓</div>
              <div style={{ fontSize: textSize.small, color: colors.textMuted, marginBottom: 4 }}>No decisions needed.</div>
              <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
                {goals} goal{goals === 1 ? '' : 's'} in flight.
              </div>
            </div>
          ) : (
            <>
              {/* Parked goals — needs_human_attention finally MEANS attention
                  (wave-1 item 1): before this bucket the flag only hid the
                  goal from every active list. */}
              {attentionGoals.length > 0 && (
                <div style={{ padding: '10px 18px 4px' }}>
                  <div style={{
                    fontFamily: font.body, fontSize: 10, fontWeight: 700,
                    letterSpacing: '0.10em', textTransform: 'uppercase',
                    color: '#e8a33d', marginBottom: 6,
                  }}>
                    Parked goals — waiting on you
                  </div>
                  {attentionGoals.map(g => (
                    <div
                      key={g.id}
                      style={{
                        display: 'flex', alignItems: 'baseline', gap: 8,
                        padding: '6px 0',
                        borderBottom: `1px solid ${colors.border}`,
                      }}
                    >
                      <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1 }}>{g.title}</span>
                      <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim }}>
                        {g.state_binding}
                      </span>
                      {g.reason && (
                        <span style={{ fontSize: textSize.micro, color: colors.textMuted, maxWidth: 260, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={g.reason}>
                          {g.reason}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
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
                /* No success tick: `showAll` swallows its own failure, and the
                   rest of the list appearing is the confirmation. */
                <Button
                  colors={colors}
                  variant="bare"
                  type="button"
                  flashSuccess={false}
                  onClick={() => { inbox.showAll(); }}
                  style={{
                    '--pa-btn-fg': colors.cyan,
                    '--pa-btn-fg-hover': colors.cyan,
                    '--pa-btn-bg-hover': colors.cyanSoft,
                    '--pa-btn-bg-active': colors.cyanSoft,
                    '--pa-btn-pad': '10px',
                    '--pa-btn-radius': '0',
                    display: 'flex', width: '100%',
                    fontFamily: font.body, fontSize: textSize.caption,
                  } as CSSProperties}
                >
                  +{moreCount} more decision{moreCount === 1 ? '' : 's'}
                </Button>
              )}

              {handled > 0 && (
                <>
                  {/* Disclosure toggle for the routine-items group below, with
                      its own three-part row (dot, label, chevron) laid out by
                      the button itself. It keeps the element and takes the
                      shared `.pa-btn` interaction rules instead of the Button
                      primitive, whose single label span would collapse that
                      row and whose pending/success machinery has nothing to
                      say about opening a list. */}
                  <button
                    type="button"
                    className="pa-btn"
                    aria-expanded={tier1Open}
                    aria-controls={tier1Id}
                    onClick={toggleTier1}
                    style={{
                      '--pa-btn-bg': 'transparent',
                      '--pa-btn-fg': colors.text,
                      '--pa-btn-border': 'transparent',
                      '--pa-btn-bg-hover': colors.cyanSoft,
                      '--pa-btn-fg-hover': colors.text,
                      '--pa-btn-bg-active': colors.cyanSoft,
                      '--pa-btn-pad': '12px 18px',
                      '--pa-btn-radius': '0',
                      gap: 10, width: '100%',
                      justifyContent: 'flex-start',
                      borderTop: `1px solid ${colors.border}`,
                      textAlign: 'left', fontFamily: font.body,
                      fontSize: 'inherit', lineHeight: 'inherit',
                    } as CSSProperties}
                  >
                    <span style={{
                      width: 8, height: 8, borderRadius: '50%',
                      background: colors.success, flexShrink: 0,
                    }} />
                    <span style={{ fontSize: textSize.small, fontWeight: 500, color: colors.text, flex: 1 }}>
                      {agentName} handled {handled} routine item{handled === 1 ? '' : 's'} overnight
                    </span>
                    <span style={{
                      color: colors.textDim, fontSize: textSize.micro,
                      transform: tier1Open ? 'rotate(90deg)' : 'none',
                      transition: reduceMotion ? 'none' : `transform 150ms ${ease.out}`,
                    }}>▸</span>
                  </button>
                  {tier1Open && (
                    <div id={tier1Id}>
                      {history === null ? (
                        <div style={{ padding: '10px 18px 10px 36px', fontSize: textSize.caption, color: colors.textDim }}>
                          Loading…
                        </div>
                      ) : (
                        tier1Rows.map(row => (
                          <div
                            key={row.id}
                            style={{
                              display: 'flex', alignItems: 'center', gap: 10,
                              padding: '8px 18px 8px 36px', fontSize: textSize.caption,
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
                            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
                              {formatAge(row.created_at)}
                            </span>
                            <Button
                              colors={colors}
                              variant="bare"
                              type="button"
                              className="hover:underline"
                              onClick={openHistory}
                              style={{
                                '--pa-btn-fg': colors.cyan,
                                '--pa-btn-fg-hover': colors.cyan,
                                '--pa-btn-bg-hover': 'transparent',
                                '--pa-btn-bg-active': 'transparent',
                                '--pa-btn-pad': '0',
                                '--pa-btn-radius': '0',
                                '--pa-btn-weight': 400,
                                fontSize: textSize.micro, fontFamily: font.body,
                                flexShrink: 0,
                              } as CSSProperties}
                            >
                              audit →
                            </Button>
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
            <Button
              colors={colors}
              variant="bare"
              type="button"
              className="hover:underline"
              onClick={openHistory}
              style={{
                '--pa-btn-fg': colors.textDim,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': 'transparent',
                '--pa-btn-bg-active': 'transparent',
                '--pa-btn-pad': '0',
                '--pa-btn-radius': '0',
                '--pa-btn-weight': 400,
                fontSize: textSize.caption, fontFamily: font.body,
              } as CSSProperties}
            >
              History →
            </Button>
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
  const agentName = persona?.display_name ?? 'your agent';
  if (items === null) {
    return (
      <div style={{ padding: '32px 18px', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
        Loading…
      </div>
    );
  }
  if (items.length === 0) {
    return (
      <div style={{ padding: '32px 18px', textAlign: 'center', fontSize: textSize.caption, color: colors.textDim }}>
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
              fontFamily: font.mono, fontSize: 10, letterSpacing: '0.06em',
              textTransform: 'uppercase', borderRadius: radius.xs, padding: '2px 6px',
              flexShrink: 0,
              color: item.tier === 1 ? colors.success : colors.cyan,
              background: item.tier === 1 ? colors.success + '26' : colors.cyanSoft,
            }}>
              {item.tier === 1 ? agentName : 'you'}
            </span>
            <span style={{
              fontSize: textSize.small, fontWeight: 500, color: colors.text, flex: 1,
              whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
            }}>
              {item.headline}
            </span>
            <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>
              {formatAge(item.created_at)}
            </span>
          </div>
          <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: 4 }}>
            {resolutionText(item, agentName)}
          </div>
        </div>
      ))}
    </div>
  );
}
